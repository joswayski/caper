import Foundation
import SwiftUI
import ImageIO

struct EmojiCatalogEntry: Codable, Identifiable, Sendable {
    let id: String
    let emoji: String
    let name: String
    let keywords: String
    let category: String
    let selectable: Bool
    let sheet: Int
    let x: Int
    let y: Int
}

private final class EmojiBundleToken {}

enum EmojiArtwork {
    private static let entries: [EmojiCatalogEntry] = {
        guard let url = resourceURL(name: "catalog", extension: "json"),
              let data = try? Data(contentsOf: url),
              let entries = try? JSONDecoder().decode([EmojiCatalogEntry].self, from: data) else { return [] }
        return entries
    }()
    static let choices = entries.filter(\.selectable)
    private static let entriesByID = Dictionary(uniqueKeysWithValues: entries.map { ($0.id, $0) })

    static func id(for emoji: String) -> String {
        emoji.unicodeScalars.filter { $0.value != 0xfe0f || emoji.unicodeScalars.contains(where: { $0.value == 0x200d }) }
            .map { String($0.value, radix: 16) }.joined(separator: "-")
    }

    static func entry(for emoji: String) -> EmojiCatalogEntry? {
        entriesByID[id(for: emoji)]
    }

    /// The catalog's dash-separated name, as web derives it ("thumbs-up",
    /// "red-heart"). Reactions are stored fully qualified, while the catalog
    /// keys some emoji without U+FE0F, so a miss retries with every U+FE0F
    /// removed from both sides. Entries without a real name (their name is
    /// only the code point ID) return nil, and callers show the glyph.
    static func name(for emoji: String) -> String? {
        guard let entry = EmojiArtwork.entry(for: emoji) ?? entriesByBareEmoji[bare(emoji)],
              !entry.name.isEmpty, entry.name != entry.id else { return nil }
        return entry.name
    }

    private static let entriesByBareEmoji: [String: EmojiCatalogEntry] = {
        var result: [String: EmojiCatalogEntry] = [:]
        for entry in EmojiArtwork.entries {
            let key = EmojiArtwork.bare(entry.emoji)
            if result[key] == nil { result[key] = entry }
        }
        return result
    }()

    private static func bare(_ emoji: String) -> String {
        var scalars = String.UnicodeScalarView()
        scalars.append(contentsOf: emoji.unicodeScalars.filter { $0.value != 0xfe0f })
        return String(scalars)
    }

    static func image(for entry: EmojiCatalogEntry) -> CGImage? {
        EmojiImageCache.shared.image(for: entry)
    }

    private static func resourceURL(name: String, extension ext: String) -> URL? {
        #if SWIFT_PACKAGE
        let bundle = Bundle.module
        #else
        let bundle = Bundle(for: EmojiBundleToken.self)
        #endif
        return bundle.url(forResource: name, withExtension: ext, subdirectory: "EmojiAssets")
            ?? bundle.url(forResource: name, withExtension: ext)
    }

    private final class EmojiImageCache: @unchecked Sendable {
        static let shared = EmojiImageCache()
        private let cache = NSCache<NSString, CGImageBox>()
        private var sheets: [Int: CGImage] = [:]
        private let lock = NSLock()

        func image(for entry: EmojiCatalogEntry) -> CGImage? {
            if let cached = cache.object(forKey: entry.id as NSString)?.image { return cached }
            lock.lock(); defer { lock.unlock() }
            if let cached = cache.object(forKey: entry.id as NSString)?.image { return cached }
            let sheet: CGImage
            if let existing = sheets[entry.sheet] { sheet = existing }
            else {
                guard let url = EmojiArtwork.resourceURL(name: "sheet-\(entry.sheet)", extension: "png"),
                      let source = CGImageSourceCreateWithURL(url as CFURL, nil),
                      let loaded = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return nil }
                sheets[entry.sheet] = loaded; sheet = loaded
            }
            // The catalog contains CGImage pixel coordinates, not grid indices.
            let rect = CGRect(x: entry.x, y: entry.y, width: 64, height: 64)
            guard let cropped = sheet.cropping(to: rect) else { return nil }
            cache.setObject(CGImageBox(cropped), forKey: entry.id as NSString)
            return cropped
        }
    }

    private final class CGImageBox { let image: CGImage; init(_ image: CGImage) { self.image = image } }
}

struct EmojiArtworkView: View {
    let emoji: String
    var size: CGFloat = 22
    var body: some View {
        if let entry = EmojiArtwork.entry(for: emoji), let image = EmojiArtwork.image(for: entry) {
            Image(decorative: image, scale: 1).resizable().interpolation(.high).frame(width: size, height: size)
        } else {
            Color.clear.frame(width: size, height: size).accessibilityHidden(true)
        }
    }
}
