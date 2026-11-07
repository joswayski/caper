import Foundation
@_implementationOnly import CaperMobile

/// The same lossless cleaner used by Android and Rust desktop. It reads and
/// writes files in bounded chunks; no video/audio payload crosses the C ABI.
public enum AttachmentMetadata {
    public static let refusalMessage = "Caper can’t remove hidden details (like location) from this file, so it can’t be sent."

    /// Keep the privacy explanation on a failed draft instead of replacing it
    /// with the generic transport failure label.
    static func refusalMessage(for error: Error) -> String? {
        guard let error = error as? APIError, error.code == "media_metadata" else { return nil }
        return error.message
    }

    /// Makes a private, cleaned upload copy without changing the picked or
    /// edited file. The caller owns the result until the upload finishes and
    /// removes it with `AttachmentStaging.remove`, including on failure.
    public static func cleanCopy(of source: URL, contentType: String) async throws -> LocalAttachmentFile {
        let task = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            let directory = try AttachmentStaging.newDirectory()
            let destination = directory.appendingPathComponent(AttachmentStaging.sanitized(source.lastPathComponent))
            do {
                let sourceBytes = Array(source.path.utf8)
                let destinationBytes = Array(destination.path.utf8)
                let mimeBytes = Array(contentType.utf8)
                let result = sourceBytes.withUnsafeBufferPointer { source in
                    destinationBytes.withUnsafeBufferPointer { destination in
                        mimeBytes.withUnsafeBufferPointer { mime in
                            caper_mobile_clean_metadata(source.baseAddress, source.count,
                                                        destination.baseAddress, destination.count,
                                                        mime.baseAddress, mime.count)
                        }
                    }
                }
                try Task.checkCancellation()
                switch result {
                case 0: break
                case 1, 2: throw APIError(status: 422, message: refusalMessage, code: "media_metadata")
                default: throw APIError(status: 500, message: "Caper couldn’t prepare this file. Check available storage and try again.")
                }
                let size = try destination.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
                return LocalAttachmentFile(url: destination, name: destination.lastPathComponent, contentType: contentType, size: size)
            } catch {
                try? FileManager.default.removeItem(at: directory)
                throw error
            }
        }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }

    /// Previews and profile pictures already have a small, bounded encoded
    /// buffer. Stage it so they pass through the identical file cleaner too.
    static func cleanCopy(of data: Data, contentType: String) async throws -> LocalAttachmentFile {
        try Task.checkCancellation()
        let directory = try AttachmentStaging.newDirectory()
        let source = directory.appendingPathComponent("preview")
        defer { AttachmentStaging.remove(source) }
        try data.write(to: source)
        return try await cleanCopy(of: source, contentType: contentType)
    }
}
