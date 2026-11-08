import Foundation
import UniformTypeIdentifiers
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

/// A picked file copied into the app's temporary staging area, before
/// `AttachmentPreparer` compresses it.
public struct LocalAttachmentFile: Equatable, Sendable {
    public let url: URL
    public let name: String
    public let contentType: String
    public let size: Int
    /// The photo-library item it was copied from (the attach drawer's
    /// selection), or nil for files from anywhere else.
    public var source: String? = nil
}

public enum AttachmentStaging {
    static var root: URL { FileManager.default.temporaryDirectory.appendingPathComponent("caper-attachments", isDirectory: true) }

    static func newDirectory() throws -> URL {
        let directory = root.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    /// The MIME type for the file's extension, or `application/octet-stream`.
    public static func contentType(forFilename name: String) -> String {
        let ext = (name as NSString).pathExtension
        guard !ext.isEmpty, let type = UTType(filenameExtension: ext) else { return "application/octet-stream" }
        return type.preferredMIMEType ?? "application/octet-stream"
    }

    /// Copies a picked, dropped or imported file so it outlives the picker's
    /// access window. Security-scoped URLs (file importer) are opened here.
    public static func stage(copying source: URL, filename: String? = nil) throws -> LocalAttachmentFile {
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        let name = sanitized(filename ?? source.lastPathComponent)
        let destination = try newDirectory().appendingPathComponent(name)
        try FileManager.default.copyItem(at: source, to: destination)
        return try file(at: destination, name: name)
    }

    public static func stage(data: Data, filename: String) throws -> LocalAttachmentFile {
        let name = sanitized(filename)
        let destination = try newDirectory().appendingPathComponent(name)
        try data.write(to: destination)
        return try file(at: destination, name: name)
    }

    static func file(at url: URL, name: String) throws -> LocalAttachmentFile {
        let size = (try url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
        return LocalAttachmentFile(url: url, name: name, contentType: contentType(forFilename: name), size: size)
    }

    /// Removes a staged file's private directory; ignores files elsewhere.
    public static func remove(_ url: URL) {
        let directory = url.deletingLastPathComponent()
        guard directory.deletingLastPathComponent().standardizedFileURL.path == root.standardizedFileURL.path else { return }
        try? FileManager.default.removeItem(at: directory)
    }

    static func sanitized(_ name: String) -> String {
        let cleaned = name.replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: ":", with: "_")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return cleaned.isEmpty || cleaned == "." || cleaned == ".." ? "file" : String(cleaned.prefix(200))
    }
}

/// Called only by an explicit native Paste action. File URLs and file-like bytes
/// become private staging copies; ordinary text/links stay with the text view.
@MainActor enum NativeAttachmentPaste {
    /// Match staging's type policy without reading payloads while building the edit menu.
    #if os(iOS)
    static var hasAttachments: Bool {
        UIPasteboard.general.hasImages || UIPasteboard.general.types.contains {
            $0 == UTType.fileURL.identifier || attachmentType($0) != nil
        }
    }
    #endif

    private static func attachmentType(_ identifier: String) -> UTType? {
        guard let type = UTType(identifier), type.conforms(to: .data),
              (!type.conforms(to: .text) || type.conforms(to: .image)), !type.conforms(to: .url),
              type.preferredFilenameExtension != nil || type == .data else { return nil }
        return type
    }

    private static func preferredTypes(_ identifiers: [String]) -> [String] {
        identifiers.filter { attachmentType($0) != nil }.sorted {
            if ($0 == UTType.gif.identifier) != ($1 == UTType.gif.identifier) {
                return $0 == UTType.gif.identifier
            }
            // Prefer a named format over an opaque alternative of the same item.
            if ($0 == UTType.data.identifier) != ($1 == UTType.data.identifier) {
                return $1 == UTType.data.identifier
            }
            return $0 < $1
        }
    }

    static func stage() throws -> [LocalAttachmentFile] {
        var files: [LocalAttachmentFile] = []
        do {
            #if os(iOS)
            for item in UIPasteboard.general.items.prefix(AttachmentPolicy.maxAttachments) {
                if let value = item[UTType.fileURL.identifier], let url = fileURL(value) {
                    files.append(try AttachmentStaging.stage(copying: url))
                    continue
                }
                if let (identifier, data) = preferredTypes(Array(item.keys)).compactMap({ identifier in
                    (item[identifier] as? Data).map { (identifier, $0) }
                }).first {
                    files.append(try stageData(data, identifier: identifier))
                } else if let image = item.values.compactMap({ $0 as? UIImage }).first, let data = image.pngData() {
                    files.append(try stageData(data, identifier: UTType.png.identifier))
                }
            }
            #elseif os(macOS)
            for item in (NSPasteboard.general.pasteboardItems ?? []).prefix(AttachmentPolicy.maxAttachments) {
                if let value = item.string(forType: .fileURL), let url = fileURL(value) {
                    files.append(try AttachmentStaging.stage(copying: url))
                    continue
                }
                for identifier in preferredTypes(item.types.map(\.rawValue)) {
                    guard let data = item.data(forType: NSPasteboard.PasteboardType(identifier)) else { continue }
                    files.append(try stageData(data, identifier: identifier))
                    break
                }
            }
            #endif
            return files
        } catch {
            files.forEach { AttachmentStaging.remove($0.url) }
            throw error
        }
    }

    private static func fileURL(_ value: Any) -> URL? {
        let url: URL?
        if let value = value as? URL { url = value }
        else if let value = value as? String { url = URL(string: value) }
        else if let value = value as? Data, let string = String(data: value, encoding: .utf8) { url = URL(string: string) }
        else { url = nil }
        return url?.isFileURL == true ? url : nil
    }

    private static func stageData(_ data: Data, identifier: String) throws -> LocalAttachmentFile {
        let type = UTType(identifier)
        let ext = type?.preferredFilenameExtension ?? "bin"
        let name = type?.conforms(to: .image) == true ? "pasted-image" : "pasted-file"
        return try AttachmentStaging.stage(data: data, filename: "\(name).\(ext)")
    }
}

/// Reserve, PUT the preview and then the compressed file straight to storage
/// (presigned headers only, no app credentials; in parts when the server
/// asks), then confirm.
public enum AttachmentUploader {
    /// Allow ten queued files to finish metadata processing (`409` while queued).
    public static let completeRetryDelays: [Duration] = Array(repeating: .seconds(5), count: 240)
    /// A failed part is sent again with a fresh URL after each of these pauses.
    public static let partRetryDelays: [Duration] = [.milliseconds(500), .seconds(2), .seconds(5), .seconds(15)]
    /// Parts in flight at once, and part URLs asked for at a time (web
    /// `PART_CONCURRENCY`, `PART_BATCH`). Asking as parts go keeps a long
    /// upload alive on the server.
    static let partConcurrency = 4
    static let partBatch = 4

    public static func upload(_ file: PreparedAttachment, channelID: String, api: APIClient,
                              completeRetryDelays: [Duration] = AttachmentUploader.completeRetryDelays,
                              partRetryDelays: [Duration] = AttachmentUploader.partRetryDelays,
                              progress: @escaping @Sendable (Double) -> Void) async throws -> ChatAttachment {
        // Clean the final encoded/edited bytes, not just picked originals.
        // Recordings alone still go through the API's voice re-encoder.
        let originalURL = file.fileURL
        var file = file
        if !file.voice {
            let cleaned = try await AttachmentMetadata.cleanCopy(of: file.fileURL, contentType: file.contentType)
            file.fileURL = cleaned.url
            file.byteSize = cleaned.size
        }
        defer { if file.fileURL != originalURL { AttachmentStaging.remove(file.fileURL) } }
        let preview: LocalAttachmentFile?
        if let image = file.preview {
            preview = try await AttachmentMetadata.cleanCopy(of: image.data, contentType: image.contentType)
        } else {
            preview = nil
        }
        defer { if let preview { AttachmentStaging.remove(preview.url) } }
        try Task.checkCancellation()
        func dimension(_ value: Int?) -> Int? { value.flatMap { (1...32_768).contains($0) ? $0 : nil } }
        var input = AssetCreateInput(
            channelId: channelID, filename: file.name, contentType: file.contentType, byteSize: file.byteSize,
            sourceByteSize: file.sourceSize > 0 ? file.sourceSize : nil,
            width: dimension(file.width), height: dimension(file.height),
            durationMs: file.durationMs.flatMap { (0...86_400_000).contains($0) ? $0 : nil },
            preview: preview.map { AssetCreateInput.Preview(contentType: $0.contentType, byteSize: $0.size) },
            voice: file.voice ? true : nil, waveform: file.voice ? file.waveform : nil
        )
        input.multipart = true
        let reservation = try await api.createAsset(input)
        do {
            try Task.checkCancellation()
            let previewSize = Double(preview?.size ?? 0)
            let originalSize = Double(file.byteSize)
            let total = max(1, previewSize + originalSize)
            if let preview {
                // A declared preview must be stored too, or `complete` reports 409.
                guard let previewUpload = reservation.previewUpload else {
                    throw APIError(status: 502, message: "The upload service returned an invalid response.")
                }
                try await api.putToStorage(previewUpload, body: .file(preview.url))
                try Task.checkCancellation()
            }
            let report: @Sendable (Double) -> Void = { sent in progress((previewSize + sent) / total) }
            if let plan = reservation.multipart {
                guard plan.partBytes > 0, plan.partCount == (file.byteSize + plan.partBytes - 1) / plan.partBytes else {
                    throw APIError(status: 502, message: "The upload service returned an invalid response.")
                }
                try await uploadParts(file.fileURL, size: file.byteSize, plan: plan, id: reservation.id, api: api,
                                      retryDelays: partRetryDelays, progress: report)
            } else {
                guard let upload = reservation.upload else {
                    throw APIError(status: 502, message: "The upload service returned an invalid response.")
                }
                try await api.putToStorage(upload, body: .file(file.fileURL)) { fraction in report(fraction * originalSize) }
            }
            try Task.checkCancellation()
            var delays = completeRetryDelays[...]
            while true {
                do {
                    let attachment = try await api.completeAsset(id: reservation.id)
                    try Task.checkCancellation()
                    progress(1)
                    return attachment
                } catch let error as APIError where error.status == 409 && error.code != "upload_incomplete" {
                    // Storage has not seen it yet or cleaning is queued; missing parts won't arrive by waiting.
                    guard let delay = delays.popFirst() else { throw error }
                    try await Task.sleep(for: delay)
                }
            }
        } catch {
            await api.cancelAsset(id: reservation.id)
            throw error
        }
    }

    /// Sends a large file in parts, each straight to storage (web
    /// `uploadParts`): `partConcurrency` at a time, URLs asked for as they are
    /// needed, and a failed part sent again with a fresh URL after each of
    /// `retryDelays`. The first part that can't be sent stops the rest.
    static func uploadParts(_ source: URL, size: Int, plan: MultipartPlan, id: String, api: APIClient,
                            retryDelays: [Duration], progress: @escaping @Sendable (Double) -> Void) async throws {
        let state = PartState(count: plan.partCount)
        try await withThrowingTaskGroup(of: Void.self) { group in
            for _ in 0..<min(partConcurrency, plan.partCount) {
                group.addTask {
                    while let number = state.claim() {
                        let offset = (number - 1) * plan.partBytes
                        let length = min(plan.partBytes, size - offset)
                        // Each part is read into its own file so at most a few parts sit on disk, none in memory.
                        let part = try partFile(source, offset: offset, length: length)
                        defer { try? FileManager.default.removeItem(at: part) }
                        var attempt = 0
                        while true {
                            let upload = try await state.url(for: number, batch: partBatch) { numbers in
                                try await api.assetParts(id: id, parts: numbers)
                            }
                            do {
                                try await api.putToStorage(upload, body: .file(part)) { fraction in
                                    progress(state.sent(number, Double(length) * fraction))
                                }
                                progress(state.sent(number, Double(length)))
                                break
                            } catch where !Task.isCancelled && (error is StorageUploadError || error is URLError) {
                                guard attempt < retryDelays.count else { throw error }
                                progress(state.sent(number, 0))
                                try await Task.sleep(for: retryDelays[attempt])
                                attempt += 1
                            }
                        }
                    }
                }
            }
            try await group.waitForAll()
        }
    }

    /// Exactly `length` bytes of `source` from `offset`, copied to a new temporary file.
    private static func partFile(_ source: URL, offset: Int, length: Int) throws -> URL {
        let target = FileManager.default.temporaryDirectory.appendingPathComponent("caper-part-\(UUID().uuidString)")
        guard FileManager.default.createFile(atPath: target.path, contents: nil) else { throw CocoaError(.fileWriteUnknown) }
        do {
            let input = try FileHandle(forReadingFrom: source)
            defer { try? input.close() }
            let output = try FileHandle(forWritingTo: target)
            defer { try? output.close() }
            try input.seek(toOffset: UInt64(offset))
            var left = length
            while left > 0 {
                guard let chunk = try input.read(upToCount: min(left, 1 << 20)), !chunk.isEmpty else {
                    throw APIError(status: 422, message: "The file changed while uploading.")
                }
                try output.write(contentsOf: chunk)
                left -= chunk.count
            }
            return target
        } catch {
            try? FileManager.default.removeItem(at: target)
            throw error
        }
    }
}

/// Shared by a multipart upload's workers: the next part to send, fetched
/// URLs not yet used, parts whose URL a worker has taken (a batch only asks
/// ahead for parts no worker has started, so a retry re-signs just the
/// failed part), and bytes sent per part.
private final class PartState: @unchecked Sendable {
    private let lock = NSLock()
    private let count: Int
    private var next = 1
    private var ready: [Int: PresignedUpload] = [:]
    private var taken = Set<Int>()
    private var bytes: [Double]
    /// Serialises part URL requests, like web's single request per batch.
    private let asking = AsyncSerial()

    init(count: Int) { self.count = count; bytes = Array(repeating: 0, count: count) }

    func claim() -> Int? {
        lock.withLock {
            guard next <= count else { return nil }
            defer { next += 1 }
            return next
        }
    }

    /// Records part `number`'s bytes sent and returns the total so far.
    func sent(_ number: Int, _ value: Double) -> Double {
        lock.withLock { bytes[number - 1] = value; return bytes.reduce(0, +) }
    }

    func url(for number: Int, batch: Int,
             fetch: @escaping @Sendable ([Int]) async throws -> [Int: PresignedUpload]) async throws -> PresignedUpload {
        try await asking.run {
            if let upload = self.take(number) { return upload }
            let numbers = self.lock.withLock {
                [number] + (number + 1...max(number + 1, self.count)).filter {
                    $0 <= self.count && self.ready[$0] == nil && !self.taken.contains($0)
                }
            }.prefix(batch)
            let fetched = try await fetch(Array(numbers))
            self.lock.withLock { self.ready.merge(fetched) { _, new in new } }
            guard let upload = self.take(number) else {
                throw APIError(status: 502, message: "The upload service returned an invalid response.")
            }
            return upload
        }
    }

    private func take(_ number: Int) -> PresignedUpload? {
        lock.withLock {
            guard let upload = ready.removeValue(forKey: number) else { return nil }
            taken.insert(number)
            return upload
        }
    }
}

/// Runs one async operation at a time, in arrival order.
private actor AsyncSerial {
    private var tail: Task<Void, Never>?

    func run<T: Sendable>(_ operation: @escaping @Sendable () async throws -> T) async throws -> T {
        let previous = tail
        let task = Task<T, Error> {
            await previous?.value
            return try await operation()
        }
        tail = Task { _ = try? await task.value }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }
}
