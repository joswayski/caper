import Foundation
import SwiftUI
import ImageIO
@_implementationOnly import CaperMobile

public struct EmojiCatalogEntry: Codable, Identifiable, Sendable, Equatable {
    public let id: String
    public let emoji: String
    public let name: String
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
        // The shared autocomplete searches these same bytes (shared/messages/emoji.rs).
        _ = data.withUnsafeBytes { caper_mobile_emoji_catalog($0.bindMemory(to: UInt8.self).baseAddress, $0.count) }
        return entries
    }()
    /// The catalog entry at a position the shared autocomplete returned.
    static func entry(at index: Int) -> EmojiCatalogEntry? {
        entries.indices.contains(index) ? entries[index] : nil
    }
    static let choices = entries.filter(\.selectable)
    /// Picker order approximating web's: smileys (the emoticon block, then
    /// other faces) before people, then the other categories with flags last.
    /// The catalog itself is in code point order.
    static let pickerChoices: [EmojiCatalogEntry] = {
        let order = ["smileys_people", "animals_nature", "food_drink", "travel_places", "activities", "objects", "symbols", "flags"]
        func rank(_ entry: EmojiCatalogEntry) -> (Int, Int) {
            let category = order.firstIndex(of: entry.category) ?? order.count
            let point = entry.id.split(separator: "-").first.flatMap { UInt32($0, radix: 16) } ?? 0
            let face = entry.category != "smileys_people" || (0x1f600...0x1f64f).contains(point) ? 0
                : entry.name.contains("face") || entry.keywords.split(separator: " ").contains("face") ? 1 : 2
            return (category, face)
        }
        return choices.enumerated().sorted { left, right in
            let (a, b) = (rank(left.element), rank(right.element))
            return (a.0, a.1, left.offset) < (b.0, b.1, right.offset)
        }.map(\.element)
    }()
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

    /// UTF-16 ranges for the native text views; a joined emoji is one target.
    static func namedRanges(in text: String) -> [(range: NSRange, name: String)] {
        var result: [(range: NSRange, name: String)] = []
        text.enumerateSubstrings(in: text.startIndex..<text.endIndex, options: .byComposedCharacterSequences) { value, range, _, _ in
            if let value, let name = name(for: value) { result.append((NSRange(range, in: text), name)) }
        }
        return result
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
        /// Each emoji is its own 64 px bitmap (16 KB): a `cropping(to:)` image
        /// would keep its whole decoded 1024 px sheet (4 MB) alive. Only the
        /// last sheet used stays decoded, for runs of emoji from one sheet.
        private let cache: NSCache<NSString, CGImageBox> = { let cache = NSCache<NSString, CGImageBox>(); cache.countLimit = 1_024; return cache }()
        private var sheet: (index: Int, image: CGImage, pixels: CFData)?
        private let lock = NSLock()

        func image(for entry: EmojiCatalogEntry) -> CGImage? {
            if let cached = cache.object(forKey: entry.id as NSString)?.image { return cached }
            lock.lock(); defer { lock.unlock() }
            if let cached = cache.object(forKey: entry.id as NSString)?.image { return cached }
            if sheet?.index != entry.sheet {
                sheet = nil
                guard let url = EmojiArtwork.resourceURL(name: "sheet-\(entry.sheet)", extension: "png"),
                      let source = CGImageSourceCreateWithURL(url as CFURL, nil),
                      let loaded = CGImageSourceCreateImageAtIndex(source, 0, [kCGImageSourceShouldCache: false] as CFDictionary),
                      let pixels = loaded.dataProvider?.data else { return nil }
                sheet = (entry.sheet, loaded, pixels)
            }
            guard let (_, image, pixels) = sheet, let base = CFDataGetBytePtr(pixels) else { return nil }
            // The catalog contains CGImage pixel coordinates, not grid indices.
            // Copy that block's decoded bytes, in the sheet's own format.
            let bytesPerPixel = image.bitsPerPixel / 8, rowBytes = 64 * bytesPerPixel
            guard entry.x + 64 <= image.width, entry.y + 64 <= image.height else { return nil }
            var block = Data(count: rowBytes * 64)
            block.withUnsafeMutableBytes { out in
                for row in 0..<64 {
                    memcpy(out.baseAddress! + row * rowBytes, base + (entry.y + row) * image.bytesPerRow + entry.x * bytesPerPixel, rowBytes)
                }
            }
            guard let provider = CGDataProvider(data: block as CFData),
                  let emoji = CGImage(width: 64, height: 64, bitsPerComponent: image.bitsPerComponent, bitsPerPixel: image.bitsPerPixel,
                                      bytesPerRow: rowBytes, space: image.colorSpace ?? CGColorSpaceCreateDeviceRGB(), bitmapInfo: image.bitmapInfo,
                                      provider: provider, decode: nil, shouldInterpolate: image.shouldInterpolate, intent: image.renderingIntent) else { return nil }
            cache.setObject(CGImageBox(emoji), forKey: entry.id as NSString)
            return emoji
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
