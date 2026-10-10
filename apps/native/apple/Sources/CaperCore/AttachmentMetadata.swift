import Foundation

/// Lossless metadata removal for files uploaded without re-encoding, so
/// photo and video locations never leave the device. Image and sample data
/// are copied byte for byte; only metadata containers change.
public enum AttachmentMetadata {
    // MARK: MP4 / QuickTime

    /// One ISO BMFF / QuickTime box: `[start, end)` in the file.
    struct Box: Equatable {
        let type: String
        let start: Int
        let headerSize: Int
        let end: Int
        var contentStart: Int { start + headerSize }
    }

    private static func bigEndian(_ bytes: [UInt8], _ offset: Int, _ count: Int) -> UInt64 {
        var value: UInt64 = 0
        for index in offset..<(offset + count) { value = value << 8 | UInt64(bytes[index]) }
        return value
    }

    /// The boxes laid end to end in `[start, end)`, or nil when a header is
    /// malformed. A 32-bit size of 1 means a 64-bit `largesize` follows; 0
    /// means the box runs to `end`.
    static func boxes(from start: Int, to end: Int, read: (Int, Int) -> [UInt8]?) -> [Box]? {
        var result: [Box] = []
        var offset = start
        while offset + 8 <= end {
            guard let header = read(offset, 8), header.count == 8 else { return nil }
            var size = Int(bigEndian(header, 0, 4))
            var headerSize = 8
            let type = String(decoding: header[4..<8], as: UTF8.self)
            if size == 1 {
                guard offset + 16 <= end, let large = read(offset + 8, 8), large.count == 8 else { return nil }
                let value = bigEndian(large, 0, 8)
                guard value <= UInt64(Int.max / 2) else { return nil }
                size = Int(value)
                headerSize = 16
            } else if size == 0 {
                size = end - offset
            }
            guard size >= headerSize, offset + size <= end else { return nil }
            result.append(Box(type: type, start: offset, headerSize: headerSize, end: offset + size))
            offset += size
        }
        return result
    }

    static let quickTimeMetadataBoxes: Set<String> = ["udta", "meta"]

    /// File offsets of the 4-byte types of every `udta` and `meta` box
    /// directly under `moov` and under each `moov/trak`. These hold
    /// `©xyz` and `com.apple.quicktime.location.ISO6709` locations. Nil when
    /// the file is not a readable MP4/QuickTime file.
    static func quickTimeMetadataTypeOffsets(fileSize: Int, read: (Int, Int) -> [UInt8]?) -> [Int]? {
        quickTimeMetadataBoxList(fileSize: fileSize, read: read)?.map { $0.start + 4 }
    }

    /// The metadata boxes themselves (see `quickTimeMetadataTypeOffsets`).
    static func quickTimeMetadataBoxList(fileSize: Int, read: (Int, Int) -> [UInt8]?) -> [Box]? {
        guard let top = boxes(from: 0, to: fileSize, read: read), let moov = top.first(where: { $0.type == "moov" }),
              let children = boxes(from: moov.contentStart, to: moov.end, read: read) else { return nil }
        var found: [Box] = []
        for child in children {
            if quickTimeMetadataBoxes.contains(child.type) { found.append(child) }
            guard child.type == "trak" else { continue }
            guard let trackChildren = boxes(from: child.contentStart, to: child.end, read: read) else { return nil }
            found += trackChildren.filter { quickTimeMetadataBoxes.contains($0.type) }
        }
        return found
    }

    /// Renames every metadata box (see `quickTimeMetadataTypeOffsets`) to
    /// `free` and zero-fills its contents in place, so the location bytes are
    /// gone rather than merely hidden: same sizes, so no offset moves and
    /// samples are untouched. Returns false when nothing was changed.
    @discardableResult
    static func patchQuickTimeMetadata(at url: URL) -> Bool {
        guard let reader = try? FileHandle(forReadingFrom: url) else { return false }
        let fileSize = (try? reader.seekToEnd()).map { Int($0) } ?? 0
        let found = quickTimeMetadataBoxList(fileSize: fileSize) { offset, count in
            guard (try? reader.seek(toOffset: UInt64(offset))) != nil, let data = try? reader.read(upToCount: count) else { return nil }
            return [UInt8](data)
        }
        try? reader.close()
        guard let found, !found.isEmpty, let writer = try? FileHandle(forWritingTo: url) else { return false }
        defer { try? writer.close() }
        let chunk = 64 * 1024
        do {
            for box in found {
                try writer.seek(toOffset: UInt64(box.start + 4))
                try writer.write(contentsOf: Data("free".utf8))
                try writer.seek(toOffset: UInt64(box.contentStart))
                var remaining = box.end - box.contentStart
                while remaining > 0 {
                    let count = min(chunk, remaining)
                    try writer.write(contentsOf: Data(count: count))
                    remaining -= count
                }
            }
            try writer.synchronize()
            return true
        } catch {
            return false
        }
    }

    /// A staged copy of an MP4/QuickTime original without its metadata
    /// boxes, or nil when there is nothing to remove (or it is not one).
    public static func strippedQuickTimeCopy(of url: URL, name: String) -> URL? {
        guard let directory = try? AttachmentStaging.newDirectory() else { return nil }
        let destination = directory.appendingPathComponent(AttachmentStaging.sanitized(name))
        guard (try? FileManager.default.copyItem(at: url, to: destination)) != nil, patchQuickTimeMetadata(at: destination) else {
            try? FileManager.default.removeItem(at: directory)
            return nil
        }
        return destination
    }

    // MARK: JPEG

    /// A minimal APP1 Exif segment holding only the Orientation tag.
    static func orientationExifSegment(_ orientation: Int) -> [UInt8] {
        var payload: [UInt8] = [0x45, 0x78, 0x69, 0x66, 0x00, 0x00] // "Exif\0\0"
        payload += [0x4D, 0x4D, 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08] as [UInt8] // Big-endian TIFF header, IFD0 at 8.
        payload += [0x00, 0x01] as [UInt8] // One entry.
        payload += [0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01] as [UInt8] // Orientation, SHORT, count 1.
        payload += [0x00, UInt8(clamping: orientation), 0x00, 0x00]
        payload += [0x00, 0x00, 0x00, 0x00] as [UInt8] // No next IFD.
        let length = payload.count + 2
        var segment: [UInt8] = [0xFF, 0xE1, UInt8(length >> 8), UInt8(length & 0xFF)]
        segment += payload
        return segment
    }

    /// The JPEG without APP1 (Exif/XMP) and other APPn segments, keeping
    /// APP0 (JFIF), APP2 `ICC_PROFILE` and APP14 (Adobe). An orientation
    /// other than 1 is kept as a minimal Exif segment. Entropy-coded data is
    /// copied unchanged. Nil when nothing was removed or it is not a JPEG.
    public static func strippedJPEG(_ data: Data, orientation: Int) -> Data? {
        let bytes = [UInt8](data)
        guard bytes.count > 4, bytes[0] == 0xFF, bytes[1] == 0xD8 else { return nil }
        var output: [UInt8] = [0xFF, 0xD8]
        var removed = false
        var wroteOrientation = false
        func writeOrientation() {
            guard !wroteOrientation else { return }
            wroteOrientation = true
            if (2...8).contains(orientation) { output += orientationExifSegment(orientation) }
        }
        var iccSignature = Array("ICC_PROFILE".utf8)
        iccSignature.append(0)
        var index = 2
        while index < bytes.count {
            guard bytes[index] == 0xFF else { return nil }
            var markerIndex = index
            while markerIndex < bytes.count && bytes[markerIndex] == 0xFF { markerIndex += 1 } // Fill bytes.
            guard markerIndex < bytes.count else { return nil }
            let marker = bytes[markerIndex]
            if marker == 0xDA || marker == 0xD9 {
                // Start of scan (or end of image): the rest is copied verbatim.
                writeOrientation()
                output += bytes[(markerIndex - 1)...]
                return removed ? Data(output) : nil
            }
            if (0xD0...0xD7).contains(marker) || marker == 0x01 {
                output += [0xFF, marker]
                index = markerIndex + 1
                continue
            }
            guard markerIndex + 2 < bytes.count else { return nil }
            let length = Int(bytes[markerIndex + 1]) << 8 | Int(bytes[markerIndex + 2])
            let end = markerIndex + 1 + length
            guard length >= 2, end <= bytes.count else { return nil }
            let payload = bytes[(markerIndex + 3)..<end]
            let keep: Bool
            switch marker {
            case 0xE0, 0xEE: keep = true
            case 0xE2: keep = payload.starts(with: iccSignature)
            case 0xE1...0xEF: keep = false
            default: keep = true
            }
            // Exif belongs right after JFIF's APP0 (or the SOI).
            if marker != 0xE0 { writeOrientation() }
            if keep {
                output.append(0xFF)
                output += bytes[markerIndex..<end]
            } else {
                removed = true
            }
            index = end
        }
        return nil
    }

    // MARK: PNG

    static let pngMetadataChunks: Set<String> = ["eXIf", "tEXt", "zTXt", "iTXt"]

    /// The PNG without `eXIf`, `tEXt`, `zTXt` and `iTXt` chunks; every other
    /// chunk (with its CRC) is copied unchanged. Nil when nothing was removed
    /// or it is not a PNG.
    public static func strippedPNG(_ data: Data) -> Data? {
        let bytes = [UInt8](data)
        let signature: [UInt8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        guard bytes.count >= 8, Array(bytes[0..<8]) == signature else { return nil }
        var output = signature
        var removed = false
        var index = 8
        while index + 12 <= bytes.count {
            let length = Int(bigEndian(bytes, index, 4))
            let type = String(decoding: bytes[(index + 4)..<(index + 8)], as: UTF8.self)
            let end = index + 12 + length
            guard end <= bytes.count else { return nil }
            if pngMetadataChunks.contains(type) { removed = true } else { output += bytes[index..<end] }
            index = end
            if type == "IEND" { return removed ? Data(output) : nil }
        }
        return nil
    }
}
