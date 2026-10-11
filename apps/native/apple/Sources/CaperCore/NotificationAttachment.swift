import AVFoundation
import Foundation
import ImageIO
import UniformTypeIdentifiers
import UserNotifications

/// Downloads an existing original. Any PNG rendition lives only on this device
/// until UserNotifications takes ownership; nothing is uploaded or cached.
enum NotificationAttachment {
    static let maximumBytes = 10_000_000

    static func load(_ data: [AnyHashable: Any]) async -> UNNotificationAttachment? {
        guard let raw = data["attachmentUrl"] as? String, let url = URL(string: raw),
              url.scheme == "https", url.host != nil, url.user == nil, url.password == nil,
              url.path.range(of: "^/original/[A-Za-z0-9]{1,64}$", options: .regularExpression) != nil,
              let type = data["attachmentContentType"] as? String,
              let size = data["attachmentSize"] as? Int, (1...maximumBytes).contains(size),
              ["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif", "video/mp4", "video/webm", "video/quicktime"].contains(type) else { return nil }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 5
        configuration.timeoutIntervalForResource = 5
        configuration.httpShouldSetCookies = false
        configuration.urlCredentialStorage = nil
        let session = URLSession(configuration: configuration, delegate: NoRedirects(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        do {
            let (stream, response) = try await session.bytes(from: url)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200,
                  response.expectedContentLength <= Int64(maximumBytes) else { return nil }
            var bytes = Data()
            bytes.reserveCapacity(size)
            for try await byte in stream {
                guard bytes.count < size else { return nil }
                bytes.append(byte)
                if bytes.count % 65_536 == 0 { try Task.checkCancellation() }
            }
            try Task.checkCancellation()
            guard bytes.count == size else { return nil }
            let image: CGImage?
            if type.hasPrefix("image/") {
                guard let source = CGImageSourceCreateWithData(bytes as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) else { return nil }
                image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceCreateThumbnailWithTransform: true,
                    kCGImageSourceThumbnailMaxPixelSize: 1024,
                ] as CFDictionary)
            } else {
                let suffix = type == "video/webm" ? "webm" : type == "video/quicktime" ? "mov" : "mp4"
                let original = FileManager.default.temporaryDirectory.appendingPathComponent("caper-notification-\(UUID().uuidString).\(suffix)")
                try bytes.write(to: original)
                defer { try? FileManager.default.removeItem(at: original) }
                let generator = AVAssetImageGenerator(asset: AVURLAsset(url: original))
                generator.appliesPreferredTrackTransform = true
                generator.maximumSize = CGSize(width: 1024, height: 1024)
                image = try await generator.image(at: .zero).image
            }
            try Task.checkCancellation()
            guard let image else { return nil }
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
