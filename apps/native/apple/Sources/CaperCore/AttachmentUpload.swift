import Foundation
import UniformTypeIdentifiers

/// A picked file copied into the app's temporary staging area. It is
/// uploaded unchanged: the server's media worker does all compression.
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

/// Reserve, PUT the original straight to storage, then confirm. The
/// confirmed attachment is `processing` until the media worker finishes;
/// the message can be sent right away.
public enum AttachmentUploader {
    /// Waits between `complete` attempts while storage has not yet reported
    /// the object (`409`).
    public static let completeRetryDelays: [Duration] = [.milliseconds(500), .seconds(1), .seconds(2)]

    public static func upload(_ file: LocalAttachmentFile, channelID: String, api: APIClient,
                              completeRetryDelays: [Duration] = AttachmentUploader.completeRetryDelays,
                              progress: @escaping @Sendable (Double) -> Void) async throws -> ChatAttachment {
        let input = AssetCreateInput(channelId: channelID, filename: file.name, contentType: file.contentType, byteSize: file.size)
        let reservation = try await api.createAsset(input)
        try Task.checkCancellation()
        try await api.putToStorage(reservation.upload, file: file.url, progress: progress)
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
