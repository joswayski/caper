import Foundation
import ImageIO
import UniformTypeIdentifiers
import UserNotifications

/// Downloads an image original or existing video poster, never video bytes.
/// Any PNG rendition lives only on this device
/// until UserNotifications takes ownership; nothing is uploaded or cached.
enum NotificationAttachment {
    static let maximumBytes = 50_000_000

    static func load(_ data: [AnyHashable: Any]) async -> UNNotificationAttachment? {
        guard let raw = data["attachmentUrl"] as? String, let url = URL(string: raw),
              url.scheme == "https", url.host != nil, url.user == nil, url.password == nil,
              url.path.range(of: "^/(original|preview)/[A-Za-z0-9]{1,64}$", options: .regularExpression) != nil,
              let type = data["attachmentContentType"] as? String,
              let size = data["attachmentSize"] as? Int, (1...maximumBytes).contains(size),
              ["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"].contains(type) else { return nil }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 5
        configuration.timeoutIntervalForResource = 5
        configuration.httpShouldSetCookies = false
        configuration.urlCredentialStorage = nil
        configuration.urlCache = nil
        let session = URLSession(configuration: configuration, delegate: NoRedirects(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        do {
            let (stream, response) = try await session.bytes(from: url)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200,
                  response.expectedContentLength <= Int64(size) else { return nil }
            let original = FileManager.default.temporaryDirectory.appendingPathComponent("caper-notification-source-\(UUID().uuidString).image")
            guard FileManager.default.createFile(atPath: original.path, contents: nil) else { return nil }
            defer { try? FileManager.default.removeItem(at: original) }
            let outputHandle = try FileHandle(forWritingTo: original)
            defer { try? outputHandle.close() }
            var chunk = Data(capacity: 65_536)
            var total = 0
            for try await byte in stream {
                guard total < size else { return nil }
                chunk.append(byte); total += 1
                if chunk.count == 65_536 {
                    try outputHandle.write(contentsOf: chunk)
                    chunk.removeAll(keepingCapacity: true)
                    try Task.checkCancellation()
                }
            }
            try outputHandle.write(contentsOf: chunk)
            try Task.checkCancellation()
            guard total == size,
                  let source = CGImageSourceCreateWithURL(original as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
                  let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceCreateThumbnailWithTransform: true,
                    kCGImageSourceThumbnailMaxPixelSize: 1024,
                ] as CFDictionary) else { return nil }
            try Task.checkCancellation()
            let output = FileManager.default.temporaryDirectory.appendingPathComponent("caper-notification-\(UUID().uuidString).png")
            guard let destination = CGImageDestinationCreateWithURL(output as CFURL, UTType.png.identifier as CFString, 1, nil) else { return nil }
            CGImageDestinationAddImage(destination, image, nil)
            guard CGImageDestinationFinalize(destination) else {
                try? FileManager.default.removeItem(at: output); return nil
            }
            do {
                try Task.checkCancellation()
                return try UNNotificationAttachment(identifier: "upload", url: output, options: nil)
            } catch {
                try? FileManager.default.removeItem(at: output); return nil
            }
        } catch { return nil }
    }

    private final class NoRedirects: NSObject, URLSessionTaskDelegate {
        func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                        newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
            completionHandler(nil)
        }
    }
}
