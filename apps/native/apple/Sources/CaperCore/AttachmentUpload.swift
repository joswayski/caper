import Foundation
import UniformTypeIdentifiers

/// A picked file copied into the app's temporary staging area, before
/// `AttachmentPreparer` compresses it.
public struct LocalAttachmentFile: Equatable, Sendable {
    public let url: URL
    public let name: String
    public let contentType: String
    public let size: Int
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

/// Reserve, PUT the preview and then the compressed file straight to storage
/// (presigned headers only, no app credentials), then confirm.
public enum AttachmentUploader {
    /// Waits between `complete` attempts while storage has not yet reported
    /// the object (`409`).
    public static let completeRetryDelays: [Duration] = [.milliseconds(500), .seconds(1), .seconds(2)]

    public static func upload(_ file: PreparedAttachment, channelID: String, api: APIClient,
                              completeRetryDelays: [Duration] = AttachmentUploader.completeRetryDelays,
                              progress: @escaping @Sendable (Double) -> Void) async throws -> ChatAttachment {
        func dimension(_ value: Int?) -> Int? { value.flatMap { (1...32_768).contains($0) ? $0 : nil } }
        let input = AssetCreateInput(
            channelId: channelID, filename: file.name, contentType: file.contentType, byteSize: file.byteSize,
            sourceByteSize: file.sourceSize > 0 ? file.sourceSize : nil,
            width: dimension(file.width), height: dimension(file.height),
            durationMs: file.durationMs.flatMap { (0...86_400_000).contains($0) ? $0 : nil },
            preview: file.preview.map { AssetCreateInput.Preview(contentType: $0.contentType, byteSize: $0.data.count) }
        )
        let reservation = try await api.createAsset(input)
        try Task.checkCancellation()
        let previewSize = Double(file.preview?.data.count ?? 0)
        let originalSize = Double(file.byteSize)
        let total = max(1, previewSize + originalSize)
        if let preview = file.preview {
            // A declared preview must be stored too, or `complete` reports 409.
            guard let previewUpload = reservation.previewUpload else {
                throw APIError(status: 502, message: "The upload service returned an invalid response.")
            }
            try await api.putToStorage(previewUpload, body: .data(preview.data))
            try Task.checkCancellation()
        }
        try await api.putToStorage(reservation.upload, body: .file(file.fileURL)) { fraction in
            progress((previewSize + fraction * originalSize) / total)
        }
        try Task.checkCancellation()
        var delays = completeRetryDelays[...]
        while true {
            do {
                let attachment = try await api.completeAsset(id: reservation.id)
                progress(1)
                return attachment
            } catch let error as APIError where error.status == 409 {
                guard let delay = delays.popFirst() else { throw error }
                try await Task.sleep(for: delay)
            }
        }
    }
}
