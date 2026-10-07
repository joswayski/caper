import Compression
import Foundation

/// Lossless palette ("screenshot") PNG encoder: colour type 3 with PLTE and,
/// when any colour is translucent, tRNS. Pixels are reproduced exactly. The
/// zlib stream wraps the Compression framework's raw DEFLATE output with a
/// zlib header and Adler-32 trailer.
public enum IndexedPNG {
    public struct Palette: Equatable, Sendable {
        public let width: Int
        public let height: Int
        /// RGBA colours packed as 0xRRGGBBAA, translucent colours first.
        public let colors: [UInt32]
        /// One palette index per pixel, row-major from the top.
        public let indices: [UInt8]
    }

    /// The exact palette of straight (non-premultiplied) RGBA8 pixels, or nil
    /// when the image has more than `maxColors` (at most 256) distinct colours.
    public static func palette(width: Int, height: Int, rgba: [UInt8], maxColors: Int = 256) -> Palette? {
        let limit = min(256, maxColors)
        guard width > 0, height > 0, limit > 0, rgba.count == width * height * 4 else { return nil }
        var lookup: [UInt32: UInt8] = [:]
        var colors: [UInt32] = []
        var indices = [UInt8](repeating: 0, count: width * height)
        var lastColor: UInt32 = 0
        var lastIndex: UInt8 = 0
        var hasLast = false
        let complete = rgba.withUnsafeBufferPointer { pixels -> Bool in
            for pixel in 0..<(width * height) {
                let offset = pixel * 4
                let color = UInt32(pixels[offset]) << 24 | UInt32(pixels[offset + 1]) << 16 | UInt32(pixels[offset + 2]) << 8 | UInt32(pixels[offset + 3])
                if hasLast && color == lastColor { indices[pixel] = lastIndex; continue }
                let index: UInt8
                if let known = lookup[color] { index = known }
                else {
                    guard colors.count < limit else { return false }
                    index = UInt8(colors.count)
                    lookup[color] = index
                    colors.append(color)
                }
                indices[pixel] = index
                lastColor = color; lastIndex = index; hasLast = true
            }
            return true
        }
        guard complete else { return nil }
        // Translucent entries first keep the tRNS chunk as short as possible.
        let order = colors.indices.sorted { lhs, rhs in
            let lhsOpaque = colors[lhs] & 0xFF == 0xFF, rhsOpaque = colors[rhs] & 0xFF == 0xFF
            return lhsOpaque == rhsOpaque ? lhs < rhs : !lhsOpaque
        }
        var remap = [UInt8](repeating: 0, count: colors.count)
        for (newIndex, oldIndex) in order.enumerated() { remap[oldIndex] = UInt8(newIndex) }
        return Palette(width: width, height: height, colors: order.map { colors[$0] }, indices: indices.map { remap[Int($0)] })
    }

    public static func bitDepth(colorCount: Int) -> Int {
        colorCount <= 2 ? 1 : colorCount <= 4 ? 2 : colorCount <= 16 ? 4 : 8
    }

    /// `iccProfile` embeds the source's colour profile (an `iCCP` chunk) so
    /// palette entries keep their exact values; nil writes an `sRGB` chunk.
    public static func encode(_ palette: Palette, iccProfile: Data? = nil) -> Data? {
        let depth = bitDepth(colorCount: palette.colors.count)
        let rowBytes = (palette.width * depth + 7) / 8
        var raw = [UInt8](repeating: 0, count: (rowBytes + 1) * palette.height)
        let perByte = 8 / depth
        for row in 0..<palette.height {
            let start = row * (rowBytes + 1)
            raw[start] = 0 // Filter type None, recommended for palette images.
            for column in 0..<palette.width {
                let index = palette.indices[row * palette.width + column]
                let byte = start + 1 + column / perByte
                let shift = 8 - depth * (column % perByte + 1)
                raw[byte] |= index << UInt8(shift)
            }
        }
        guard let compressed = zlib(raw) else { return nil }

        var png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        var header = Data()
        header.appendBigEndian(UInt32(palette.width))
        header.appendBigEndian(UInt32(palette.height))
        header.append(contentsOf: [UInt8(depth), 3, 0, 0, 0])
        png.appendChunk("IHDR", header)
        if let iccProfile, !iccProfile.isEmpty {
            // PNG allows either iCCP or sRGB, never both.
            guard let profile = zlib([UInt8](iccProfile)) else { return nil }
            png.appendChunk("iCCP", Data("ICC Profile".utf8) + Data([0, 0]) + profile)
        } else {
            png.appendChunk("sRGB", Data([0]))
        }
        png.appendChunk("PLTE", Data(palette.colors.flatMap { [UInt8($0 >> 24 & 0xFF), UInt8($0 >> 16 & 0xFF), UInt8($0 >> 8 & 0xFF)] }))
        let alphas = palette.colors.map { UInt8($0 & 0xFF) }
        if let lastTranslucent = alphas.lastIndex(where: { $0 != 0xFF }) {
            png.appendChunk("tRNS", Data(alphas[...lastTranslucent]))
        }
        png.appendChunk("IDAT", compressed)
        png.appendChunk("IEND", Data())
        return png
    }

    /// RFC 1950 stream around raw DEFLATE.
    static func zlib(_ bytes: [UInt8]) -> Data? {
        let capacity = bytes.count + bytes.count / 4 + 1024
        var output = [UInt8](repeating: 0, count: capacity)
        let written = bytes.withUnsafeBufferPointer { source in
            output.withUnsafeMutableBufferPointer { destination in
                compression_encode_buffer(destination.baseAddress!, capacity, source.baseAddress!, bytes.count, nil, COMPRESSION_ZLIB)
            }
        }
        guard written > 0 || bytes.isEmpty else { return nil }
        var data = Data([0x78, 0x9C])
        data.append(contentsOf: output[0..<written])
        data.appendBigEndian(adler32(bytes))
        return data
    }

    static func adler32(_ bytes: [UInt8]) -> UInt32 {
        var a: UInt32 = 1, b: UInt32 = 0
        var index = 0
        while index < bytes.count {
            // 5552 is the largest block that cannot overflow before the modulo.
            let end = min(index + 5552, bytes.count)
            for byte in bytes[index..<end] { a += UInt32(byte); b += a }
            a %= 65521; b %= 65521
            index = end
        }
        return b << 16 | a
    }

    private static let crcTable: [UInt32] = (0..<256).map { value in
        var crc = UInt32(value)
        for _ in 0..<8 { crc = crc & 1 == 1 ? 0xEDB8_8320 ^ (crc >> 1) : crc >> 1 }
        return crc
    }

    static func crc32<Bytes: Swift.Sequence>(_ bytes: Bytes) -> UInt32 where Bytes.Element == UInt8 {
        var crc: UInt32 = 0xFFFF_FFFF
        for byte in bytes { crc = crcTable[Int((crc ^ UInt32(byte)) & 0xFF)] ^ (crc >> 8) }
        return crc ^ 0xFFFF_FFFF
    }
}

private extension Data {
    mutating func appendBigEndian(_ value: UInt32) {
        append(contentsOf: [UInt8(value >> 24 & 0xFF), UInt8(value >> 16 & 0xFF), UInt8(value >> 8 & 0xFF), UInt8(value & 0xFF)])
    }

    mutating func appendChunk(_ type: String, _ body: Data) {
        appendBigEndian(UInt32(body.count))
        let typeBytes = Data(type.utf8)
        append(typeBytes)
        append(body)
        appendBigEndian(IndexedPNG.crc32(typeBytes + body))
    }
}
