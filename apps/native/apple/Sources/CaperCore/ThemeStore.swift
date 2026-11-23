import Foundation
import Observation
import SwiftUI
#if os(macOS)
import AppKit
#elseif os(iOS)
import UIKit
#endif

/// Phase 2 theme document matching `shared/themes/schema.json` (`caper.theme.v1`).
public struct CaperThemeDocument: Codable, Equatable, Sendable {
    public struct Surfaces: Codable, Equatable, Sendable {
        public var frame: String
        public var base: String
        public var sidebar: String
        public var sidebarRaised: String
        public var conversation: String
        public var raised: String
        public var composer: String
        public var replyDraft: String
        public var hover: String
        public var hoverAlpha: Double
    }
    public struct Borders: Codable, Equatable, Sendable {
        public var `default`: String
        public var translucent: String
        public var subtle: String
        public var rim: String
    }
    public struct TextTokens: Codable, Equatable, Sendable {
        public var primary: String
        public var message: String
        public var muted: String
        public var link: String
        public var linkHover: String
    }
    public struct ActionTokens: Codable, Equatable, Sendable {
        public var primary: String
        public var primaryHover: String
        public var primaryPress: String
        public var onPrimary: String
    }
    public struct AccentTokens: Codable, Equatable, Sendable {
        public var primary: String
        public var bright: String
        public var softText: String
        public var surface: String
        public var border: String
    }
    public struct BrandTokens: Codable, Equatable, Sendable {
        public var caper: String
        public var caperBright: String
        public var resizeHighlight: String
    }
    public struct StatusTokens: Codable, Equatable, Sendable {
        public var voice: String
        public var pin: String
        public var pinWash: String
        public var idle: String
        public var errorText: String
        public var danger: String
        public var dangerHover: String
        public var dangerWash: String
        public var onDanger: String
        public var voiceOff: String
        public var voiceOffIcon: String
        public var voiceOffWash: String
    }
    public struct Radii: Codable, Equatable, Sendable {
        public var canvas: Double
        public var card: Double
        public var composer: Double
        public var control: Double
        public var pill: Double
        public var avatar: String
    }

    public var schema: String
    public var id: String
    public var name: String
    public var appearance: String
    public var surfaces: Surfaces
    public var borders: Borders
    public var text: TextTokens
    public var action: ActionTokens
    public var accent: AccentTokens
    public var brand: BrandTokens
    public var status: StatusTokens
    public var radii: Radii

    public var isLight: Bool { appearance == "light" }

    public static let obsidian = CaperThemeDocument(
        schema: "caper.theme.v1",
        id: "obsidian",
        name: "Obsidian",
        appearance: "dark",
        surfaces: .init(
            frame: "#0b0b0c", base: "#121214", sidebar: "#0f0f11", sidebarRaised: "#1c1c20",
            conversation: "#131316", raised: "#1b1b1f", composer: "#222228", replyDraft: "#17171b",
            hover: "rgb(255 255 255 / 6%)", hoverAlpha: 0.06
        ),
        borders: .init(default: "#29292f", translucent: "rgb(255 255 255 / 9%)", subtle: "rgb(255 255 255 / 6%)", rim: "rgb(255 255 255 / 8%)"),
        text: .init(primary: "#f4f3ef", message: "#e6e5e0", muted: "#9a99a2", link: "#7cc7ff", linkHover: "#b2dfff"),
        action: .init(primary: "#f4f3ef", primaryHover: "#ffffff", primaryPress: "#dfddd6", onPrimary: "#0b0b0c"),
        accent: .init(primary: "#557a38", bright: "#87bc5c", softText: "#c3e4a4", surface: "#1c2816", border: "#486832"),
        brand: .init(caper: "#6f9948", caperBright: "#8ec460", resizeHighlight: "#485d38"),
        status: .init(
            voice: "#4ade80", pin: "#e4c76a", pinWash: "rgb(228 199 106 / 6%)", idle: "#d6a44e",
            errorText: "#ff9b82", danger: "#c83c4e", dangerHover: "#e04c5f", dangerWash: "rgb(200 60 78 / 16%)",
            onDanger: "#ffffff", voiceOff: "#ff7182", voiceOffIcon: "#ff7182", voiceOffWash: "rgb(224 76 95 / 18%)"
        ),
        radii: .init(canvas: 16, card: 13, composer: 15, control: 10, pill: 999, avatar: "30%")
    )

    public static let paper = CaperThemeDocument(
        schema: "caper.theme.v1",
        id: "paper",
        name: "Paper",
        appearance: "light",
        surfaces: .init(
            frame: "#efeff2", base: "#f7f7f9", sidebar: "#efeff2", sidebarRaised: "#e5e5ea",
            conversation: "#ffffff", raised: "#f5f5f8", composer: "#f2f2f6", replyDraft: "#f2f2f6",
            hover: "rgb(11 11 12 / 5%)", hoverAlpha: 0.05
        ),
        borders: .init(default: "#dcdce2", translucent: "rgb(11 11 12 / 9%)", subtle: "rgb(11 11 12 / 6%)", rim: "rgb(255 255 255 / 85%)"),
        text: .init(primary: "#111114", message: "#25252b", muted: "#666672", link: "#1d6fe2", linkHover: "#1456b8"),
        action: .init(primary: "#111114", primaryHover: "#232329", primaryPress: "#000000", onPrimary: "#ffffff"),
        accent: .init(primary: "#496d2d", bright: "#3f6325", softText: "#2b4717", surface: "#e8f0e1", border: "#a4c28c"),
        brand: .init(caper: "#5a7e3a", caperBright: "#4c6e2f", resizeHighlight: "#b8cca6"),
        status: .init(
            voice: "#15803d", pin: "#b58918", pinWash: "rgb(181 137 24 / 10%)", idle: "#b47819",
            errorText: "#c23244", danger: "#c83c4e", dangerHover: "#ad2b3d", dangerWash: "rgb(200 60 78 / 12%)",
            onDanger: "#ffffff", voiceOff: "#d63b4e", voiceOffIcon: "#d63b4e", voiceOffWash: "rgb(214 59 78 / 12%)"
        ),
        radii: .init(canvas: 16, card: 13, composer: 15, control: 10, pill: 999, avatar: "30%")
    )
}

/// Thread-safe snapshot box so dynamic NSColor providers can read the active
/// theme synchronously without violating Swift 6 MainActor isolation.
private final class ThemeSnapshotBox: @unchecked Sendable {
    private let lock = NSLock()
    private var doc: CaperThemeDocument = .obsidian
    func get() -> CaperThemeDocument {
        lock.lock(); defer { lock.unlock() }
        return doc
    }
    func set(_ next: CaperThemeDocument) {
        lock.lock(); defer { lock.unlock() }
        doc = next
    }
}

/// Shared observable theme store backed by UserDefaults (`caper.theme.v1`) with
/// saved custom themes, live color editing, and cross-window repaint broadcast.
@MainActor @Observable public final class CaperThemeStore {
    public static let shared = CaperThemeStore()
    public static let defaultsKey = "caper.theme.v1"
    public static let savedThemesKey = "caper.customThemes.v1"
    public static let didChangeNotification = Notification.Name("CaperThemeDidChange")
    private static let snapshotBox = ThemeSnapshotBox()

    public private(set) var active: CaperThemeDocument
    public private(set) var savedThemes: [CaperThemeDocument] = []
    public var isCustom: Bool { active.id != "obsidian" && active.id != "paper" }
    public var allThemes: [CaperThemeDocument] { [.obsidian, .paper] + savedThemes }

    private init() {
        if let raw = UserDefaults.standard.string(forKey: Self.defaultsKey),
           let parsed = try? Self.decode(raw) {
            if parsed.id == "paper" || parsed.isLight {
                active = .paper
                if parsed.id != "paper" {
                    UserDefaults.standard.set("paper", forKey: Self.defaultsKey)
                }
            } else {
                active = .obsidian
                if parsed.id != "obsidian" {
                    UserDefaults.standard.set("obsidian", forKey: Self.defaultsKey)
                }
            }
        } else {
            active = .obsidian
        }
        Self.snapshotBox.set(active)
        applySystemAppearance()
    }

    public nonisolated static func currentSnapshot() -> CaperThemeDocument {
        snapshotBox.get()
    }

    public func selectPreset(_ id: String) {
        if id == "paper" {
            active = .paper
            UserDefaults.standard.set("paper", forKey: Self.defaultsKey)
        } else {
            active = .obsidian
            UserDefaults.standard.set("obsidian", forKey: Self.defaultsKey)
        }
        Self.snapshotBox.set(active)
        applySystemAppearance()
    }

    /// Applies an in-progress custom theme live while editing swatches.
    public func applyLiveTheme(_ doc: CaperThemeDocument, persist: Bool = true) {
        active = doc
        Self.snapshotBox.set(doc)
        if persist {
            if doc.id == "obsidian" || doc.id == "paper" {
                UserDefaults.standard.set(doc.id, forKey: Self.defaultsKey)
            } else {
                persistActiveDocument(doc)
            }
        }
        applySystemAppearance()
    }

    /// Saves a custom theme into the user's library and activates it.
    public func saveCustomTheme(_ draft: CaperThemeDocument) {
        var doc = draft
        let cleanName = doc.name.trimmingCharacters(in: .whitespacesAndNewlines)
        doc.name = cleanName.isEmpty ? "Custom Theme" : String(cleanName.prefix(48))
        let slug = doc.name.lowercased()
            .replacingOccurrences(of: "[^a-z0-9]+", with: "-", options: .regularExpression)
            .trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        if doc.id == "obsidian" || doc.id == "paper" || doc.id.isEmpty {
            doc.id = slug.isEmpty || slug == "obsidian" || slug == "paper" ? "custom-\(Int(Date().timeIntervalSince1970))" : slug
        }
        if let idx = savedThemes.firstIndex(where: { $0.id == doc.id }) {
            savedThemes[idx] = doc
        } else {
            savedThemes.append(doc)
        }
        persistSavedThemes()
        applyLiveTheme(doc, persist: true)
    }

    public func deleteCustomTheme(id: String) {
        savedThemes.removeAll { $0.id == id }
        persistSavedThemes()
        if active.id == id {
            selectPreset("obsidian")
        }
    }

    public func importThemeJSON(_ json: String) throws {
        let doc = try Self.decode(json)
        if doc.id != "obsidian" && doc.id != "paper" {
            saveCustomTheme(doc)
        } else {
            selectPreset(doc.id)
        }
    }

    public func exportCurrentJSON() -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(active),
              let str = String(data: data, encoding: .utf8) else { return "" }
        return str + "\n"
    }

    private func persistActiveDocument(_ doc: CaperThemeDocument) {
        if let data = try? JSONEncoder().encode(doc),
           let encoded = String(data: data, encoding: .utf8) {
            UserDefaults.standard.set(encoded, forKey: Self.defaultsKey)
        }
    }

    private func persistSavedThemes() {
        if let data = try? JSONEncoder().encode(savedThemes) {
            UserDefaults.standard.set(data, forKey: Self.savedThemesKey)
        }
    }

    public func applySystemAppearance() {
        #if os(macOS)
        let name: NSAppearance.Name = active.isLight ? .aqua : .darkAqua
        if let appearance = NSAppearance(named: name) {
            NSApp?.appearance = appearance
            let bg = Self.frameColor
            for window in NSApp?.windows ?? [] {
                window.appearance = appearance
                window.backgroundColor = bg
                Self.invalidateHierarchy(window.contentView)
            }
        }
        #endif
        NotificationCenter.default.post(name: Self.didChangeNotification, object: self)
    }

    #if os(macOS)
    private static func invalidateHierarchy(_ view: NSView?) {
        guard let view else { return }
        view.needsDisplay = true
        view.needsLayout = true
        for child in view.subviews {
            invalidateHierarchy(child)
        }
    }

    /// Creates an NSColor that dynamically resolves against the active theme
    /// document whenever AppKit draws text, icons, or controls.
    public nonisolated static func dynamicNSColor(_ read: @Sendable @escaping (CaperThemeDocument) -> String,
                                                  fallback: NSColor) -> NSColor {
        NSColor(name: nil) { _ in
            let doc = snapshotBox.get()
            return nsColor(fromHex: read(doc)) ?? fallback
        }
    }

    public nonisolated static func dynamicNSColor(_ resolve: @Sendable @escaping (CaperThemeDocument) -> NSColor) -> NSColor {
        NSColor(name: nil) { _ in resolve(snapshotBox.get()) }
    }

    public nonisolated static func nsColor(fromHex hex: String) -> NSColor? {
        guard hex.hasPrefix("#"), hex.count == 7, let value = UInt32(hex.dropFirst(), radix: 16) else { return nil }
        return NSColor(
            srgbRed: CGFloat((value >> 16) & 0xFF) / 255,
            green: CGFloat((value >> 8) & 0xFF) / 255,
            blue: CGFloat(value & 0xFF) / 255,
            alpha: 1
        )
    }

    public nonisolated static func hexString(from color: NSColor) -> String {
        let srgb = color.usingColorSpace(.sRGB) ?? color
        let r = Int((min(1, max(0, srgb.redComponent)) * 255).rounded())
        let g = Int((min(1, max(0, srgb.greenComponent)) * 255).rounded())
        let b = Int((min(1, max(0, srgb.blueComponent)) * 255).rounded())
        return String(format: "#%02x%02x%02x", r, g, b)
    }

    /// Computes a readable foreground hex (`#0b0b0c` or `#ffffff`) for any background hex.
    public nonisolated static func readableForegroundHex(forHex hex: String) -> String {
        guard let c = nsColor(fromHex: hex) else { return "#ffffff" }
        let luma = 0.2126 * c.redComponent + 0.7152 * c.greenComponent + 0.0722 * c.blueComponent
        return luma > 0.56 ? "#0b0b0c" : "#ffffff"
    }

    // Dynamic semantic NSColor tokens backed by the active CaperThemeDocument.
    public nonisolated static let frameColor = dynamicNSColor(\.surfaces.frame, fallback: NSColor(srgbRed: 11/255, green: 11/255, blue: 12/255, alpha: 1))
    public nonisolated static let baseColor = dynamicNSColor(\.surfaces.base, fallback: NSColor(srgbRed: 18/255, green: 18/255, blue: 20/255, alpha: 1))
    public nonisolated static let sidebarColor = dynamicNSColor(\.surfaces.sidebar, fallback: NSColor(srgbRed: 15/255, green: 15/255, blue: 17/255, alpha: 1))
    public nonisolated static let sidebarRaisedColor = dynamicNSColor(\.surfaces.sidebarRaised, fallback: NSColor(srgbRed: 28/255, green: 28/255, blue: 32/255, alpha: 1))
    public nonisolated static let conversationColor = dynamicNSColor(\.surfaces.conversation, fallback: NSColor(srgbRed: 19/255, green: 19/255, blue: 22/255, alpha: 1))
    public nonisolated static let raisedColor = dynamicNSColor(\.surfaces.raised, fallback: NSColor(srgbRed: 27/255, green: 27/255, blue: 31/255, alpha: 1))
    public nonisolated static let composerColor = dynamicNSColor(\.surfaces.composer, fallback: NSColor(srgbRed: 34/255, green: 34/255, blue: 40/255, alpha: 1))
    public nonisolated static let replyDraftColor = dynamicNSColor(\.surfaces.replyDraft, fallback: NSColor(srgbRed: 23/255, green: 23/255, blue: 27/255, alpha: 1))
    public nonisolated static let borderColor = dynamicNSColor(\.borders.default, fallback: NSColor(srgbRed: 41/255, green: 41/255, blue: 47/255, alpha: 1))
    public nonisolated static let textColor = dynamicNSColor(\.text.primary, fallback: NSColor(srgbRed: 244/255, green: 243/255, blue: 239/255, alpha: 1))
    public nonisolated static let messageColor = dynamicNSColor(\.text.message, fallback: NSColor(srgbRed: 230/255, green: 229/255, blue: 224/255, alpha: 1))
    public nonisolated static let mutedColor = dynamicNSColor(\.text.muted, fallback: NSColor(srgbRed: 154/255, green: 153/255, blue: 162/255, alpha: 1))
    public nonisolated static let linkColor = dynamicNSColor(\.text.link, fallback: NSColor(srgbRed: 124/255, green: 199/255, blue: 255/255, alpha: 1))
    public nonisolated static let actionColor = dynamicNSColor(\.action.primary, fallback: NSColor(srgbRed: 244/255, green: 243/255, blue: 239/255, alpha: 1))
    public nonisolated static let actionHoverColor = dynamicNSColor(\.action.primaryHover, fallback: .white)
    public nonisolated static let actionPressColor = dynamicNSColor(\.action.primaryPress, fallback: NSColor(srgbRed: 223/255, green: 221/255, blue: 214/255, alpha: 1))
    public nonisolated static let onActionColor = dynamicNSColor(\.action.onPrimary, fallback: NSColor(srgbRed: 11/255, green: 11/255, blue: 12/255, alpha: 1))
    public nonisolated static let accentColor = dynamicNSColor(\.accent.primary, fallback: NSColor(srgbRed: 85/255, green: 122/255, blue: 56/255, alpha: 1))
    public nonisolated static let accentBrightColor = dynamicNSColor(\.accent.bright, fallback: NSColor(srgbRed: 135/255, green: 188/255, blue: 92/255, alpha: 1))
    public nonisolated static let accentSurfaceColor = dynamicNSColor(\.accent.surface, fallback: NSColor(srgbRed: 28/255, green: 40/255, blue: 22/255, alpha: 1))
    public nonisolated static let caperColor = dynamicNSColor(\.brand.caper, fallback: NSColor(srgbRed: 111/255, green: 153/255, blue: 72/255, alpha: 1))
    public nonisolated static let resizeHighlightColor = dynamicNSColor(\.brand.resizeHighlight, fallback: NSColor(srgbRed: 72/255, green: 93/255, blue: 56/255, alpha: 1))
    public nonisolated static let voiceSessionGreenColor = dynamicNSColor(\.status.voice, fallback: NSColor(srgbRed: 74/255, green: 222/255, blue: 128/255, alpha: 1))
    public nonisolated static let pinGoldColor = dynamicNSColor(\.status.pin, fallback: NSColor(srgbRed: 228/255, green: 199/255, blue: 106/255, alpha: 1))
    public nonisolated static let errorColor = dynamicNSColor(\.status.errorText, fallback: NSColor(srgbRed: 255/255, green: 155/255, blue: 130/255, alpha: 1))
    public nonisolated static let voiceOffColor = dynamicNSColor(\.status.voiceOff, fallback: NSColor(srgbRed: 255/255, green: 113/255, blue: 130/255, alpha: 1))
    public nonisolated static let hoverWashColor = dynamicNSColor { doc in
        doc.isLight ? NSColor(srgbRed: 11/255, green: 11/255, blue: 12/255, alpha: 0.07)
                    : NSColor.white.withAlphaComponent(0.08)
    }
    public nonisolated static let surfaceRimColor = dynamicNSColor { doc in
        doc.isLight ? NSColor.white.withAlphaComponent(0.9)
                    : NSColor.white.withAlphaComponent(0.11)
    }
    public nonisolated static let composerBorderColor = dynamicNSColor { doc in
        doc.isLight ? NSColor(srgbRed: 11/255, green: 11/255, blue: 12/255, alpha: 0.14)
                    : NSColor.white.withAlphaComponent(0.13)
    }
    public nonisolated static let composerFocusBorderColor = dynamicNSColor { doc in
        let accent = nsColor(fromHex: doc.accent.primary) ?? NSColor(srgbRed: 85/255, green: 122/255, blue: 56/255, alpha: 1)
        let neutral = doc.isLight
            ? NSColor(srgbRed: 208/255, green: 208/255, blue: 216/255, alpha: 1)
            : NSColor(srgbRed: 58/255, green: 58/255, blue: 66/255, alpha: 1)
        return neutral.blended(withFraction: doc.isLight ? 0.45 : 0.48, of: accent) ?? accent.withAlphaComponent(0.6)
    }
    public nonisolated static let composerOuterBgColor = dynamicNSColor { doc in
        doc.isLight ? NSColor(srgbRed: 11/255, green: 11/255, blue: 12/255, alpha: 0.025)
                    : NSColor.white.withAlphaComponent(0.025)
    }
    public nonisolated static let composerOuterBorderColor = dynamicNSColor { doc in
        doc.isLight ? NSColor(srgbRed: 11/255, green: 11/255, blue: 12/255, alpha: 0.09)
                    : NSColor.white.withAlphaComponent(0.07)
    }
    public nonisolated static let callFillColor = dynamicNSColor { doc in
        doc.isLight ? NSColor(srgbRed: 226/255, green: 240/255, blue: 217/255, alpha: 1)
                    : NSColor(srgbRed: 34/255, green: 56/255, blue: 44/255, alpha: 1)
    }
    public nonisolated static let callBorderColor = dynamicNSColor { doc in
        doc.isLight ? NSColor(srgbRed: 111/255, green: 153/255, blue: 72/255, alpha: 1)
                    : NSColor(srgbRed: 75/255, green: 138/255, blue: 96/255, alpha: 1)
    }
    public nonisolated static let roomFillColor = dynamicNSColor { doc in
        doc.isLight ? NSColor(srgbRed: 238/255, green: 245/255, blue: 233/255, alpha: 1)
                    : NSColor(srgbRed: 24/255, green: 35/255, blue: 31/255, alpha: 1)
    }
    public nonisolated static let roomBorderColor = dynamicNSColor { doc in
        doc.isLight ? NSColor(srgbRed: 197/255, green: 218/255, blue: 184/255, alpha: 1)
                    : NSColor(srgbRed: 40/255, green: 54/255, blue: 47/255, alpha: 1)
    }
    #endif

    public static func decode(_ raw: String) throws -> CaperThemeDocument {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed == "obsidian" { return .obsidian }
        if trimmed == "paper" { return .paper }
        guard let data = trimmed.data(using: .utf8) else {
            throw ThemeValidationError("Theme text is not valid UTF-8.")
        }
        let doc = try JSONDecoder().decode(CaperThemeDocument.self, from: data)
        guard doc.schema == "caper.theme.v1" else {
            throw ThemeValidationError("Theme schema must be \"caper.theme.v1\".")
        }
        guard doc.appearance == "dark" || doc.appearance == "light" else {
            throw ThemeValidationError("Theme appearance must be \"dark\" or \"light\".")
        }
        for hex in [
            doc.surfaces.frame, doc.surfaces.base, doc.surfaces.conversation, doc.surfaces.raised,
            doc.surfaces.composer, doc.borders.default, doc.text.primary, doc.text.muted,
            doc.action.primary, doc.action.onPrimary, doc.accent.primary, doc.accent.bright, doc.brand.caper,
        ] {
            guard Self.color(fromHex: hex) != nil else {
                throw ThemeValidationError("Invalid hex color: \(hex)")
            }
        }
        return doc
    }

    public static func color(fromHex hex: String) -> Color? {
        guard hex.hasPrefix("#"), hex.count == 7, let value = UInt32(hex.dropFirst(), radix: 16) else { return nil }
        return Color(
            red: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255
        )
    }

    public struct ThemeValidationError: LocalizedError {
        public let message: String
        public init(_ message: String) { self.message = message }
        public var errorDescription: String? { message }
    }
}
