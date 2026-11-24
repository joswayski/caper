import Foundation
import UniformTypeIdentifiers

/// Reads what another app shared into a share folder ([ShareInbox]): files are
/// copied as they are (Caper compresses them and strips their metadata before
/// uploading), and text and web links become the message text.
struct ShareCollector {
    /// The most a message carries (AttachmentPolicy.maxAttachments).
    static let maxFiles = 10

    struct Collected: Sendable {
        var files: [String] = []
        var texts: [String] = []
        var skipped = 0
        var text: String { texts.joined(separator: "\n") }
    }

    static func collect(_ items: [NSExtensionItem], into folder: URL) async -> Collected {
        var collected = Collected()
        for item in items {
            var itemTexts: [String] = []
            for provider in item.attachments ?? [] {
                if provider.hasItemConformingToTypeIdentifier(UTType.url.identifier),
                   !provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
                    if let url = await loadURL(provider), !url.isFileURL { itemTexts.append(url.absoluteString) }
                } else if provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier),
                          !provider.hasItemConformingToTypeIdentifier(UTType.image.identifier),
                          !provider.hasItemConformingToTypeIdentifier(UTType.audiovisualContent.identifier),
                          !provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
                    if let text = await loadText(provider) { itemTexts.append(text) }
                } else if collected.files.count >= maxFiles {
                    collected.skipped += 1
                } else if let name = await copyFile(provider, into: folder) {
                    collected.files.append(name)
                }
            }
            // Some apps (Safari, Notes) put the text on the item rather than an attachment.
            if itemTexts.isEmpty, let text = item.attributedContentText?.string { itemTexts.append(text) }
            for text in itemTexts.map({ $0.trimmingCharacters(in: .whitespacesAndNewlines) }) where !text.isEmpty && !collected.texts.contains(text) {
                collected.texts.append(text)
            }
        }
        return collected
    }

    private static func loadURL(_ provider: NSItemProvider) async -> URL? {
        await withCheckedContinuation { continuation in
            provider.loadItem(forTypeIdentifier: UTType.url.identifier, options: nil) { value, _ in
                continuation.resume(returning: (value as? URL) ?? (value as? Data).flatMap { URL(dataRepresentation: $0, relativeTo: nil) })
            }
        }
    }

    private static func loadText(_ provider: NSItemProvider) async -> String? {
        await withCheckedContinuation { continuation in
            provider.loadItem(forTypeIdentifier: UTType.plainText.identifier, options: nil) { value, _ in
                continuation.resume(returning: (value as? String) ?? (value as? NSAttributedString)?.string
                    ?? (value as? Data).flatMap { String(data: $0, encoding: .utf8) })
            }
        }
    }

    /// Copies the provider's file, preferring its most specific type (the
    /// original HEIC or video rather than a converted copy). Returns its name.
    private static func copyFile(_ provider: NSItemProvider, into folder: URL) async -> String? {
        let type = provider.registeredTypeIdentifiers.first { identifier in
            guard let type = UTType(identifier) else { return false }
            return type.conforms(to: .data) || type.conforms(to: .package) || type.conforms(to: .fileURL)
        }
        guard let type else { return nil }
        if type == UTType.fileURL.identifier {
            // A file shared by URL (Files, Finder): copy it from where it is.
            guard let url = await loadFileURL(provider) else { return nil }
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            let destination = ShareInbox.destination(for: url.lastPathComponent, in: folder)
            return (try? FileManager.default.copyItem(at: url, to: destination)).map { destination.lastPathComponent }
        }
        return await withCheckedContinuation { continuation in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type) { url, _ in
                // The file only lasts until this handler returns.
                guard let url else { continuation.resume(returning: nil); return }
                var name = provider.suggestedName ?? url.lastPathComponent
                if (name as NSString).pathExtension.isEmpty, let ext = UTType(type)?.preferredFilenameExtension { name += ".\(ext)" }
                let destination = ShareInbox.destination(for: name, in: folder)
                continuation.resume(returning: (try? FileManager.default.copyItem(at: url, to: destination)).map { destination.lastPathComponent })
            }
        }
    }

    private static func loadFileURL(_ provider: NSItemProvider) async -> URL? {
        await withCheckedContinuation { continuation in
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { value, _ in
                continuation.resume(returning: (value as? URL) ?? (value as? Data).flatMap { URL(dataRepresentation: $0, relativeTo: nil) })
            }
        }
    }
}
