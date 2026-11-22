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
            conversation: "#131316", raised: "#1b1b1f", composer: "#1d1d22", replyDraft: "#17171b",
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
            conversation: "#ffffff", raised: "#f5f5f8", composer: "#f6f6f9", replyDraft: "#f2f2f6",
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

/// Shared observable theme store backed by UserDefaults (`caper.theme.v1`).
@MainActor @Observable public final class CaperThemeStore {
    public static let shared = CaperThemeStore()
    public static let defaultsKey = "caper.theme.v1"

    public private(set) var active: CaperThemeDocument
    public var isCustom: Bool { active.id != "obsidian" && active.id != "paper" }

    private init() {
        if let raw = UserDefaults.standard.string(forKey: Self.defaultsKey),
           let parsed = try? Self.decode(raw) {
            active = parsed
        } else {
            active = .obsidian
        }
        applySystemAppearance()
    }

    public func selectPreset(_ id: String) {
        active = id == "paper" ? .paper : .obsidian
        UserDefaults.standard.set(active.id, forKey: Self.defaultsKey)
        applySystemAppearance()
    }

    public func importThemeJSON(_ json: String) throws {
        let doc = try Self.decode(json)
        active = doc
        let data = try JSONEncoder().encode(doc)
        if let encoded = String(data: data, encoding: .utf8) {
            UserDefaults.standard.set(encoded, forKey: Self.defaultsKey)
        }
        applySystemAppearance()
    }

    public func exportCurrentJSON() -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(active),
              let str = String(data: data, encoding: .utf8) else { return "" }
        return str + "\n"
    }

    private func applySystemAppearance() {
        #if os(macOS)
        let name: NSAppearance.Name = active.isLight ? .aqua : .darkAqua
        if let appearance = NSAppearance(named: name) {
            NSApp?.appearance = appearance
            for window in NSApp?.windows ?? [] {
                window.appearance = appearance
                window.contentView?.needsDisplay = true
            }
        }
        #endif
    }

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
