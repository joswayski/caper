import CoreText
import Foundation
#if os(macOS)
import AppKit
#elseif os(iOS)
import UIKit
#endif

/// Shared presentation rules for the AppKit/UIKit hosts. The parsers and
/// grouping rules remain the same ones used by the shipping Apple clients.
public enum NativePresentation {
    /// Resolved user ID, or an empty string for a non-person mention pill.
    public static let mentionAttribute = NSAttributedString.Key("CaperMention")
    /// A spoiler's characters: the group (an Int) of adjacent spoiler runs,
    /// such as one with a bold word, that reveal together.
    public static let spoilerAttribute = NSAttributedString.Key("CaperSpoiler")
    /// A spoiler still hidden by `concealSpoilers`: its group.
    public static let hiddenSpoilerAttribute = NSAttributedString.Key("CaperHiddenSpoiler")
    /// A quoted line's text starts after the 3pt bar and a 12pt gap, as web's quote padding.
    static let quoteIndent: CGFloat = 15

    public static func errorMessage(_ error: Error) -> String { FriendlyError.message(for: error) }

    public static func directMessageNotice(_ conversation: DirectMessageConversation) -> String? {
        if conversation.status == .incoming {
            let prompt = MessageRequests.prompt(for: conversation.peer)
            return prompt.name + prompt.detail
        }
        if conversation.blocked { return MessageRequests.blockedNotice(for: conversation.peer) }
        if conversation.status == .outgoing { return MessageRequests.waitingNotice(for: conversation.peer) }
        return nil
    }

    public static func signInCode(_ input: String) -> String {
        let allowed = Set("ABCDEFGHJKMNPQRSTWXYZ23456789")
        return String(input.uppercased().filter { allowed.contains($0) }.prefix(6))
    }

    public static func grouped(_ message: ChatMessage, after previous: ChatMessage?, inThread: Bool = false) -> Bool {
        MessageGrouping.isGrouped(message, after: previous, inThread: inThread)
    }

    public static func grouped(_ pending: PendingMessage, author: ChatAuthor?, after previous: ChatMessage?, inThread: Bool = false) -> Bool {
        MessageGrouping.isGrouped(pending, author: author, after: previous, inThread: inThread)
    }

    public static func time(_ value: String, includingDate: Bool = false) -> String {
        guard let date = ChatTimestamp.date(value) else { return "" }
        return date.formatted(date: includingDate ? .abbreviated : .omitted, time: .shortened)
    }

    public static func dayLabel(_ value: String) -> String? { ChatDateDivider.fullDateLabel(value) }

    public static func sameDay(_ first: String, _ second: String) -> Bool {
        ChatDateDivider.sameLocalDay(first, second)
    }

    public static func mentionsViewer(_ message: ChatMessage, viewerID: String?) -> Bool {
        MentionAutocomplete.mentionsCurrentUser(message, currentUserID: viewerID)
    }

    /// Both native hosts use the shipping composer's context, ranking and
    /// source UTF-16 ranges, including formatting delimiters and IME guards.
    @MainActor public static func autocomplete(text: String, selection: NSRange, markedText: Bool,
                                               model: AppModel) -> ComposerAutocompleteMatch? {
        ComposerAutocomplete.match(text: text, selection: selection, markedText: markedText, mentions: model.composerMentions)
    }

    public static func inserting(choice: Int, in text: String, match: ComposerAutocompleteMatch) -> (text: String, selection: NSRange)? {
        ComposerAutocomplete.inserting(choice: choice, in: text, match: match)
    }

    /// The same bundled artwork as SwiftUI, without exposing a SwiftUI view.
    public static func emojiImage(for entry: EmojiCatalogEntry) -> CGImage? { EmojiArtwork.image(for: entry) }

    @MainActor public static var quickReactions: [String] { MessageQuickReactions.emoji }

    /// Both Mac hosts show the same names, self-first ordering and count fallback.
    @MainActor static func reactionSummary(_ message: ChatMessage, emoji: String, chat: ChatModel, viewerID: String?) -> String {
        let name = EmojiArtwork.name(for: emoji)
        if case .loaded(let people) = chat.reactorsState(for: message, emoji: emoji, viewerID: viewerID), !people.isEmpty {
            return ReactionSummary.text(authors: people, selfID: viewerID, emojiName: name, emoji: emoji)
        }
        let ids = message.reactions?.first(where: { $0.emoji == emoji })?.authorIds ?? []
        return ReactionSummary.fallback(authorIDs: ids, selfID: viewerID, emojiName: name, emoji: emoji)
    }

    public static func emojiImage(_ emoji: String) -> CGImage? {
        EmojiArtwork.entry(for: emoji).flatMap(EmojiArtwork.image(for:))
    }

    /// The context menu uses the same link targets as the rendered message.
    public static func links(in text: String) -> [(text: String, url: URL)] {
        MessageLinks.segments(in: text).compactMap { segment in
            guard let href = segment.href, let url = URL(string: href) else { return nil }
            return (segment.text, url)
        }
    }

    public static func resend(secondsSince sentAt: Date?, codesSent: Int, now: Date) -> (label: String, enabled: Bool, allowed: Bool) {
        let seconds = CodeResend.secondsRemaining(sentAt: sentAt, now: now)
        let allowed = codesSent <= CodeResend.maximumResends
        return (CodeResend.label(secondsRemaining: seconds), seconds == 0 && allowed, allowed)
    }

    /// Theme-aware syntax highlighting color for `shared/messages/code.rs::TokenKind`.
    #if os(macOS)
    private static func tokenColor(_ kind: Int32) -> NSColor {
        let light = CaperThemeStore.isLightTheme
        switch kind {
        case 1: return light ? NSColor(srgbRed: 124/255, green: 58/255, blue: 237/255, alpha: 1) : NSColor(srgbRed: 210/255, green: 168/255, blue: 255/255, alpha: 1)
        case 2: return light ? NSColor(srgbRed: 46/255, green: 107/255, blue: 48/255, alpha: 1) : NSColor(srgbRed: 165/255, green: 214/255, blue: 167/255, alpha: 1)
        case 3: return light ? NSColor(srgbRed: 110/255, green: 110/255, blue: 120/255, alpha: 1) : NSColor(srgbRed: 122/255, green: 122/255, blue: 133/255, alpha: 1)
        case 4: return light ? NSColor(srgbRed: 180/255, green: 83/255, blue: 9/255, alpha: 1) : NSColor(srgbRed: 242/255, green: 184/255, blue: 128/255, alpha: 1)
        case 5: return light ? NSColor(srgbRed: 3/255, green: 105/255, blue: 161/255, alpha: 1) : NSColor(srgbRed: 121/255, green: 192/255, blue: 255/255, alpha: 1)
        case 6: return light ? NSColor(srgbRed: 15/255, green: 118/255, blue: 110/255, alpha: 1) : NSColor(srgbRed: 126/255, green: 231/255, blue: 200/255, alpha: 1)
        case 7: return light ? NSColor(srgbRed: 194/255, green: 65/255, blue: 12/255, alpha: 1) : NSColor(srgbRed: 255/255, green: 166/255, blue: 87/255, alpha: 1)
        case 8: return light ? NSColor(srgbRed: 21/255, green: 128/255, blue: 61/255, alpha: 1) : NSColor(srgbRed: 74/255, green: 222/255, blue: 128/255, alpha: 1)
        case 9: return light ? NSColor(srgbRed: 220/255, green: 38/255, blue: 38/255, alpha: 1) : NSColor(srgbRed: 248/255, green: 113/255, blue: 113/255, alpha: 1)
        default: return CaperThemeStore.textColor
        }
    }
    #else
    private static func tokenColor(_ kind: Int32) -> UIColor {
        let light = CaperThemeStore.isLightTheme
        switch kind {
        case 1: return light ? UIColor(red: 124/255, green: 58/255, blue: 237/255, alpha: 1) : UIColor(red: 210/255, green: 168/255, blue: 255/255, alpha: 1)
        case 2: return light ? UIColor(red: 46/255, green: 107/255, blue: 48/255, alpha: 1) : UIColor(red: 165/255, green: 214/255, blue: 167/255, alpha: 1)
        case 3: return light ? UIColor(red: 110/255, green: 110/255, blue: 120/255, alpha: 1) : UIColor(red: 122/255, green: 122/255, blue: 133/255, alpha: 1)
        case 4: return light ? UIColor(red: 180/255, green: 83/255, blue: 9/255, alpha: 1) : UIColor(red: 242/255, green: 184/255, blue: 128/255, alpha: 1)
        case 5: return light ? UIColor(red: 3/255, green: 105/255, blue: 161/255, alpha: 1) : UIColor(red: 121/255, green: 192/255, blue: 255/255, alpha: 1)
        case 6: return light ? UIColor(red: 15/255, green: 118/255, blue: 110/255, alpha: 1) : UIColor(red: 126/255, green: 231/255, blue: 200/255, alpha: 1)
        case 7: return light ? UIColor(red: 194/255, green: 65/255, blue: 12/255, alpha: 1) : UIColor(red: 255/255, green: 166/255, blue: 87/255, alpha: 1)
        case 8: return light ? UIColor(red: 21/255, green: 128/255, blue: 61/255, alpha: 1) : UIColor(red: 74/255, green: 222/255, blue: 128/255, alpha: 1)
        case 9: return light ? UIColor(red: 220/255, green: 38/255, blue: 38/255, alpha: 1) : UIColor(red: 248/255, green: 113/255, blue: 113/255, alpha: 1)
        default: return UIColor(CaperTheme.text)
        }
    }
    #endif

    /// Formatted message text: fonts, underline, strikethrough, inline code,
    /// syntax-highlighted fenced code blocks, person pills and links. Spoilers
    /// carry `spoilerAttribute` (hosts hide them with `concealSpoilers`);
    /// quoted lines carry `.quoteBar`, and code blocks carry `.codeBlockFill`.
    public static func messageText(_ text: String, size: CGFloat = 14, mentions: [MessageMention] = []) -> NSAttributedString {
        let result = NSMutableAttributedString(string: "")
        let blocks = MessageFormatting.codeBlocks(text)
        var spoilerGroup = -1, inSpoiler = false
        let codeBlockStyle = NSMutableParagraphStyle()
        codeBlockStyle.firstLineHeadIndent = 10
        codeBlockStyle.headIndent = 10
        codeBlockStyle.tailIndent = -10
        codeBlockStyle.lineSpacing = 2

        for (blockIndex, block) in blocks.enumerated() {
            if blockIndex > 0 {
                result.append(NSAttributedString(string: "\n"))
            }
            switch block {
            case let .code(codeText, language, _):
                inSpoiler = false
                #if os(macOS)
                let labelFont = NSFont.monospacedSystemFont(ofSize: max(10, size - 3), weight: .medium)
                let codeFont = NSFont.monospacedSystemFont(ofSize: max(12, size - 1), weight: .regular)
                let mutedColor = CaperThemeStore.mutedColor
                #else
                let labelFont = UIFont.monospacedSystemFont(ofSize: max(10, size - 3), weight: .medium)
                let codeFont = UIFont.monospacedSystemFont(ofSize: max(12, size - 1), weight: .regular)
                let mutedColor = UIColor(CaperTheme.muted)
                #endif
                let headerAttrs: [NSAttributedString.Key: Any] = [
                    .font: labelFont,
                    .foregroundColor: mutedColor,
                    .codeBlockFill: blockIndex + 1,
                    .paragraphStyle: codeBlockStyle,
                ]
                result.append(NSAttributedString(string: "\(MessageFormatting.languageName(language))\n", attributes: headerAttrs))
                let tokens = MessageFormatting.highlightCode(codeText, language: language)
                for token in tokens {
                    var attrs: [NSAttributedString.Key: Any] = [
                        .font: codeFont,
                        .foregroundColor: tokenColor(token.kind),
                        .codeBlockFill: blockIndex + 1,
                        .paragraphStyle: codeBlockStyle,
                    ]
                    if token.kind == 3 { attrs[.obliqueness] = 0.16 }
                    result.append(NSAttributedString(string: token.text, attributes: attrs))
                }
            case let .prose(prose):
                let parsed = MessageFormatting.runs(prose)
                // Keep mentions out of inline code spans while preserving UTF-16 offsets.
                let mentionScan = parsed.map { $0.code ? String(repeating: " ", count: $0.text.utf16.count) : $0.text }.joined()
                let segments = MentionAutocomplete.segments(in: mentionScan, mentions: mentions)
                var runStart = 0
                for run in parsed {
                    if run.spoiler && !inSpoiler { spoilerGroup += 1 }
                    inSpoiler = run.spoiler
                    let runEnd = runStart + run.text.utf16.count
                    var segmentStart = 0
                    for segment in segments {
                        let segmentEnd = segmentStart + segment.text.utf16.count
                        let start = max(runStart, segmentStart), end = min(runEnd, segmentEnd)
                        defer { segmentStart = segmentEnd }
                        guard start < end else { continue }
                        let slice = (run.text as NSString).substring(with: NSRange(location: start - runStart, length: end - start))
                        let weight = run.bold ? "Bold" : segment.highlighted ? "Medium" : "Regular"
                        #if os(macOS)
                        let font: NSFont
                        if run.code {
                            font = NSFont.monospacedSystemFont(ofSize: max(12, size - 1), weight: run.bold ? .bold : .regular)
                        } else {
                            let base = NSFont(name: "Figtree-\(weight)", size: size) ?? NSFont.systemFont(ofSize: size)
                            font = run.italic ? NSFontManager.shared.convert(base, toHaveTrait: .italicFontMask) : base
                        }
                        let color = CaperThemeStore.textColor
                        let highlight = CaperThemeStore.accentColor.withAlphaComponent(0.24)
                        #else
                        let font: UIFont
                        if run.code {
                            font = UIFont.monospacedSystemFont(ofSize: max(12, size - 1), weight: run.bold ? .bold : .regular)
                        } else {
                            let base = UIFont(name: "Figtree-\(weight)", size: size) ?? UIFont.systemFont(ofSize: size)
                            let descriptor = run.italic ? base.fontDescriptor.withSymbolicTraits(.traitItalic) ?? base.fontDescriptor : base.fontDescriptor
                            font = UIFont(descriptor: descriptor, size: size)
                        }
                        let color = UIColor(CaperTheme.text)
                        let highlight = UIColor(CaperTheme.terracotta).withAlphaComponent(0.24)
                        #endif
                        var attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color]
                        if run.italic && run.code { attributes[.obliqueness] = 0.2 }
                        if run.underline { attributes[.underlineStyle] = NSUnderlineStyle.single.rawValue }
                        if run.strikethrough { attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
                        if run.spoiler { attributes[spoilerAttribute] = spoilerGroup }
                        if run.quote { attributes[.quoteBar] = true }
                        if run.code {
                            attributes[.inlineCodeFill] = true
                            result.append(NSAttributedString(string: slice, attributes: attributes))
                        } else if segment.highlighted {
                            attributes[.backgroundColor] = highlight
                            attributes[mentionAttribute] = segment.user?.id ?? ""
                            result.append(NSAttributedString(string: slice, attributes: attributes))
                        } else {
                            for link in MessageLinks.segments(in: slice) {
                                var linked = attributes
                                if let href = link.href, let url = URL(string: href) { linked[.link] = url }
                                result.append(NSAttributedString(string: link.text, attributes: linked))
                            }
                        }
                    }
                    runStart = runEnd
                }
            }
        }
        // Quoted lines start after the 3pt bar and a 12pt gap, as web's quote padding.
        var quoted: [NSRange] = []
        result.enumerateAttribute(.quoteBar, in: NSRange(location: 0, length: result.length)) { value, range, _ in
            if value != nil { quoted.append(range) }
        }
        let quote = NSMutableParagraphStyle()
        quote.firstLineHeadIndent = quoteIndent
        quote.headIndent = quoteIndent
        for range in quoted {
            result.addAttribute(.paragraphStyle, value: quote, range: (result.string as NSString).paragraphRange(for: range))
        }
        return result
    }

    /// Hides each spoiler group of `messageText` not in `revealed`: its text
    /// and lines clear on a rounded border fill, without any link or person
    /// pill to activate. A revealed spoiler keeps a faint hover wash fill.
    public static func concealSpoilers(_ text: NSMutableAttributedString, revealed: Set<Int>) {
        #if os(macOS)
        let hidden = CaperThemeStore.borderColor, shown = CaperThemeStore.hoverWashColor, clear = NSColor.clear
        #else
        let hidden = UIColor(CaperTheme.border), shown = CaperTheme.hoverWash, clear = UIColor.clear
        #endif
        var spoilers: [(range: NSRange, group: Int)] = []
        text.enumerateAttribute(spoilerAttribute, in: NSRange(location: 0, length: text.length)) { value, range, _ in
            if let group = value as? Int { spoilers.append((range, group)) }
        }
        for spoiler in spoilers {
            if revealed.contains(spoiler.group) {
                text.addAttribute(.spoilerFill, value: shown, range: spoiler.range)
                continue
            }
            for key in [NSAttributedString.Key.link, .backgroundColor, mentionAttribute] { text.removeAttribute(key, range: spoiler.range) }
            text.addAttributes([.foregroundColor: clear, .underlineColor: clear, .strikethroughColor: clear,
                                .spoilerFill: hidden, hiddenSpoilerAttribute: spoiler.group], range: spoiler.range)
        }
    }

    /// The groups `concealSpoilers` left hidden.
    public static func hiddenSpoilers(in text: NSAttributedString) -> Set<Int> {
        var groups = Set<Int>()
        text.enumerateAttribute(hiddenSpoilerAttribute, in: NSRange(location: 0, length: text.length)) { value, _, _ in
            if let group = value as? Int { groups.insert(group) }
        }
        return groups
    }

    /// What VoiceOver reads: "Spoiler" in place of each hidden spoiler.
    public static func spokenText(_ text: NSAttributedString) -> String {
        let string = text.string as NSString
        var spoken = ""
        var last: Int?
        text.enumerateAttribute(hiddenSpoilerAttribute, in: NSRange(location: 0, length: text.length)) { value, range, _ in
            if let group = value as? Int {
                if group != last { spoken += "Spoiler" }
                last = group
            } else {
                spoken += string.substring(with: range)
                last = nil
            }
        }
        return spoken
    }

    /// Core Text measures only a realized row, using the same attributed input
    /// as its selectable native text view. No whole-history framesetter cache.
    public static func textHeight(_ text: NSAttributedString, width: CGFloat) -> CGFloat {
        let framesetter = CTFramesetterCreateWithAttributedString(text)
        return ceil(CTFramesetterSuggestFrameSizeWithConstraints(framesetter, CFRange(location: 0, length: text.length),
            nil, CGSize(width: max(1, width), height: .greatestFiniteMagnitude), nil).height)
    }
}

extension AppModel {
    /// Single context source for SwiftUI, AppKit and UIKit composers. The
    /// viewer is excluded; channel specials never leak into DMs or visits.
    public var composerMentions: MentionSource {
        let viewerID = chat.currentAuthor?.id ?? account?.id
        if let visit {
            return .visit(reporter: visit.reporter, messages: chat.messages, people: people, accountID: viewerID)
        }
        if let selectedDirectMessageID {
            return .direct(peer: directMessages.first { $0.id == selectedDirectMessageID }?.peer,
                people: people, accountID: viewerID)
        }
        return .space(members: detail?.members ?? [], excluding: viewerID)
    }
}
