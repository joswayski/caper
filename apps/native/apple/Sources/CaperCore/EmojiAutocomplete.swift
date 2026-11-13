import SwiftUI
@_implementationOnly import CaperMobile
#if os(iOS)
import UIKit
import UIKit.UIGestureRecognizerSubclass
#elseif os(macOS)
import AppKit
#endif

private struct ComposerCoveredKey: EnvironmentKey {
    static var defaultValue: Bool { false }
}

extension EnvironmentValues {
    /// True while Browse covers the phone conversation and its composers.
    var composerCovered: Bool {
        get { self[ComposerCoveredKey.self] }
        set { self[ComposerCoveredKey.self] = newValue }
    }
}

public struct EmojiAutocompleteMatch: Equatable {
    public let replacementRange: NSRange
    public let choices: [EmojiCatalogEntry]
}

enum EmojiAutocomplete {
    /// Web's `:` token at the caret and the entries it offers, through the
    /// shared Rust autocomplete (shared/messages/emoji.rs), which reads the
    /// token in the visible text and returns its source range.
    static func match(text: String, selection: NSRange, markedText: Bool) -> EmojiAutocompleteMatch? {
        guard !markedText, selection.length == 0, let token = token(text, caret: selection.location) else { return nil }
        let choices = suggestions(token.query)
        guard !choices.isEmpty else { return nil }
        return EmojiAutocompleteMatch(replacementRange: token.range, choices: choices)
    }

    /// Up to six entries web offers for `query`, best first.
    static func suggestions(_ query: String) -> [EmojiCatalogEntry] {
        guard EmojiArtwork.entry(at: 0) != nil else { return [] }
        var query = query
        return query.withUTF8 { bytes in
            let found = caper_mobile_emoji_suggestions(bytes.baseAddress, bytes.count)
            defer { caper_mobile_ints_free(found) }
            return UnsafeBufferPointer(start: found.data, count: found.len).compactMap { EmojiArtwork.entry(at: Int($0)) }
        }
    }

    /// The open `:` token with the caret at UTF-16 offset `caret`: its source range and visible query.
    static func token(_ text: String, caret: Int) -> (range: NSRange, query: String)? {
        guard caret >= 0, caret <= text.utf16.count else { return nil }
        var source = text
        let token: [Int32] = source.withUTF8 { bytes in
            let token = caper_mobile_emoji_token(bytes.baseAddress, bytes.count, Int32(caret))
            defer { caper_mobile_ints_free(token) }
            return Array(UnsafeBufferPointer(start: token.data, count: token.len))
        }
        guard token.count >= 2 else { return nil }
        let query = String(String.UnicodeScalarView(token.dropFirst(2).compactMap { Unicode.Scalar(UInt32($0)) }))
        return (NSRange(location: Int(token[0]), length: Int(token[1] - token[0])), query)
    }

    static func inserting(_ entry: EmojiCatalogEntry, in text: String, match: EmojiAutocompleteMatch,
                          scalarLimit: Int = 4_000) -> (text: String, selection: NSRange)? {
        inserting(entry.emoji, in: text, selection: match.replacementRange, scalarLimit: scalarLimit)
    }

    /// Picker selection replaces the editor's saved UTF-16 range, not the end of the draft.
    static func inserting(_ emoji: String, in text: String, selection: NSRange,
                          scalarLimit: Int = 4_000) -> (text: String, selection: NSRange)? {
        guard let range = Range(selection, in: text) else { return nil }
        let result = text.replacingCharacters(in: range, with: emoji)
        guard result.unicodeScalars.count <= scalarLimit else { return nil }
        let cursor = selection.location + (emoji as NSString).length
        return MessageFormatting.repairEdit(text, text: result, selection: NSRange(location: cursor, length: 0))
    }
}

/// The composer's active token. Emoji (`:`) and mention (`@`) tokens can never
/// both be active, so they share one popup with a row type per kind.
public enum ComposerAutocompleteMatch: Equatable {
    case emoji(EmojiAutocompleteMatch)
    case mention(MentionAutocompleteMatch)

    public var count: Int {
        switch self {
        case let .emoji(match): return match.choices.count
        case let .mention(match): return match.choices.count
        }
    }
    public var isMention: Bool {
        if case .mention = self { return true }
        return false
    }
    /// Row identities for scrolling the selected row into view.
    public var ids: [String] {
        switch self {
        case let .emoji(match): return match.choices.map(\.id)
        case let .mention(match): return match.choices.map(\.id)
        }
    }
}

enum ComposerAutocomplete {
    static func match(text: String, selection: NSRange, markedText: Bool, mentions: MentionSource) -> ComposerAutocompleteMatch? {
        guard !markedText, selection.length == 0, selection.location <= text.utf16.count else { return nil }
        let projection = MessageFormatting.projection(text)
        let displayed = projection.runs.map(\.text).joined()
        let caret = NSRange(location: projection.sourceIndices.filter { $0 < selection.location }.count, length: 0)
        func original(_ range: NSRange) -> NSRange {
            let start = projection.sourceIndices[range.location]
            return NSRange(location: start, length: projection.sourceIndices[NSMaxRange(range) - 1] + 1 - start)
        }
        if let mention = MentionAutocomplete.match(text: displayed, selection: caret, markedText: false, source: mentions) {
            return .mention(MentionAutocompleteMatch(replacementRange: original(mention.replacementRange), choices: mention.choices))
        }
        return EmojiAutocomplete.match(text: text, selection: selection, markedText: false).map { .emoji($0) }
    }

    /// Accepts the row at `index`, enforcing the same 4,000-scalar limit for both kinds.
    static func inserting(choice index: Int, in text: String, match: ComposerAutocompleteMatch) -> (text: String, selection: NSRange)? {
        switch match {
        case let .emoji(emoji):
            guard emoji.choices.indices.contains(index) else { return nil }
            return EmojiAutocomplete.inserting(emoji.choices[index], in: text, match: emoji)
        case let .mention(mention):
            guard mention.choices.indices.contains(index) else { return nil }
            return MentionAutocomplete.inserting(mention.choices[index], in: text, match: mention)
        }
    }
}

@MainActor final class ComposerAutocompleteController: ObservableObject {
    @Published var match: ComposerAutocompleteMatch?
    @Published var selected = 0
    @Published var formattingSelection = false
    /// The styles the selection already has: the toolbar's pressed buttons.
    @Published var formattingStyles: MessageStyle = []
    var formatAction: ((MessageStyle) -> Void)?
    var acceptAction: ((Int) -> Void)?
    var insertAction: ((String) -> Void)?
    /// Puts dictated words at the caret, spaced from the words around them;
    /// false when the draft would pass the length limit.
    var dictateAction: ((String) -> Bool)?
    var restoreFocusAction: (() -> Void)?
    var requestFocusAction: (() -> Bool)?
    func update(_ match: ComposerAutocompleteMatch?) {
        if self.match != match { self.match = match; selected = 0 }
    }
    func accept(_ index: Int) { acceptAction?(index) }
    func restoreFocus() { restoreFocusAction?() }
    func move(down: Bool) {
        guard let count = match?.count, count > 0 else { return }
        selected = (selected + (down ? 1 : count - 1)) % count
    }
}

struct ComposerSuggestionsView: View {
    @ObservedObject var controller: ComposerAutocompleteController
    var body: some View {
        if let match = controller.match {
            let ids = match.ids
            ScrollViewReader { proxy in
            ScrollView {
            VStack(spacing: 0) {
                switch match {
                case let .emoji(emoji):
                    ForEach(Array(emoji.choices.enumerated()), id: \.element.id) { index, entry in
                        Button { controller.accept(index) } label: {
                            HStack(spacing: 12) {
                                EmojiArtworkView(emoji: entry.emoji, size: 24)
                                Text(":\(entry.name.replacingOccurrences(of: " ", with: "_")):").font(CaperTheme.font(13, weight: .medium)).lineLimit(1)
                                Spacer()
                            }.modifier(SuggestionRow(selected: index == controller.selected))
                        }.buttonStyle(.plain).accessibilityLabel("\(entry.name), emoji")
                            .accessibilityIdentifier("emoji-suggestion-\(entry.id)")
                            .id(entry.id)
                    }
                case let .mention(mention):
                    ForEach(Array(mention.choices.enumerated()), id: \.element.id) { index, candidate in
                        Button { controller.accept(index) } label: {
                            MentionSuggestionLabel(candidate: candidate)
                                .modifier(SuggestionRow(selected: index == controller.selected))
                        }.buttonStyle(.plain)
                            .accessibilityLabel(Self.mentionLabel(candidate))
                            .accessibilityIdentifier("mention-suggestion-\(candidate.username.lowercased())")
                            .id(candidate.id)
                    }
                }
            }
            }.frame(height: min(CGFloat(match.count) * 44, 176))
                .onChange(of: controller.selected) { _, index in
                    if ids.indices.contains(index) { proxy.scrollTo(ids[index], anchor: .center) }
                }
            }.frame(maxWidth: match.isMention ? .infinity : 260)
                .background(CaperTheme.surface).clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.border))
                .accessibilityIdentifier(match.isMention ? "mention-suggestions" : "emoji-suggestions")
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

extension ComposerSuggestionsView {
    static func mentionLabel(_ candidate: MentionCandidate) -> String {
        candidate.accessibilityLabel
    }
}

/// A suggestion's avatar beside two lines: a person's display name over a
/// small muted `@username`, or `@everyone`/`@here` over its description.
/// Shared by the composer's `@` popup and the exact-username fields.
struct MentionSuggestionLabel: View {
    let candidate: MentionCandidate
    var body: some View {
        // Specials (`@everyone`, `@here`, a team name) stack the name over its description.
        let member = candidate.kind == .member
        HStack(spacing: 10) {
            if member {
                Avatar(name: candidate.displayName, size: 28, avatarID: candidate.avatarId, picture: candidate.picture)
            } else {
                Image(systemName: "at").font(.system(size: 13, weight: .semibold)).foregroundStyle(CaperTheme.muted)
                    .frame(width: 28, height: 28)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(member ? candidate.displayName : "@\(candidate.username)")
                    .font(CaperTheme.font(13, weight: .medium)).foregroundStyle(CaperTheme.text).lineLimit(1)
                Text(member ? "@\(candidate.username)\(candidate.invited ? " · Already invited" : "")" : candidate.displayName)
                    .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).lineLimit(1)
            }
            Spacer(minLength: 0)
        }
    }
}

/// Flat, borderless 44pt rows; the selected row gets the terracotta tint.
struct SuggestionRow: ViewModifier {
    let selected: Bool
    func body(content: Content) -> some View {
        content.padding(.horizontal, 10).frame(minHeight: 44)
            .background(selected ? CaperTheme.terracotta.opacity(0.32) : Color.clear)
            .contentShape(Rectangle())
    }
}

#if os(iOS) || os(macOS)
extension NSAttributedString.Key {
    public static let composerDelimiter = NSAttributedString.Key("CaperComposerDelimiter")
}

/// TextKit keeps the original string (and native selection/IME/undo coordinates),
/// but recognized Markdown delimiters have no drawn glyph or advance.
public final class ComposerGlyphs: NSObject, NSLayoutManagerDelegate {
    public func layoutManager(_ layoutManager: NSLayoutManager, shouldGenerateGlyphs glyphs: UnsafePointer<CGGlyph>,
                       properties props: UnsafePointer<NSLayoutManager.GlyphProperty>,
                       characterIndexes charIndexes: UnsafePointer<Int>, font: PlatformComposerFont,
                       forGlyphRange glyphRange: NSRange) -> Int {
        guard let storage = layoutManager.textStorage else { return 0 }
        var properties = Array(UnsafeBufferPointer(start: props, count: glyphRange.length))
        for index in properties.indices where charIndexes[index] < storage.length {
            if storage.attribute(.composerDelimiter, at: charIndexes[index], effectiveRange: nil) as? Bool == true {
                properties[index] = .null
            }
        }
        properties.withUnsafeBufferPointer {
            layoutManager.setGlyphs(glyphs, properties: $0.baseAddress!, characterIndexes: charIndexes,
                                   font: font, forGlyphRange: glyphRange)
        }
        return glyphRange.length
    }
}

/// Formatting attributes `highlightDraft` owns; composers drop them from
/// their typing attributes so new text starts plain until it is highlighted.
public let composerFormattingAttributes: [NSAttributedString.Key] = [.underlineStyle, .strikethroughStyle, .paragraphStyle, .spoilerFill, .quoteBar]

#if os(iOS)
public typealias PlatformComposerFont = UIFont
#else
public typealias PlatformComposerFont = NSFont
#endif

/// Attribute-only rendering preserves the wire draft and native editing transactions.
/// First-character kerning reserves space for the drawing-only URL icon; the
/// glyph delegate independently hides delimiters in the same source coordinates.
@MainActor public func highlightDraft(_ storage: NSTextStorage, source: MentionSource) {
    let projection = MessageFormatting.projection(storage.string)
    let visible = projection.runs.map(\.text).joined()
    let ranges = MentionAutocomplete.draftHighlights(in: visible, source: source).map { range in
        NSRange(location: projection.sourceIndices[range.location],
                length: projection.sourceIndices[NSMaxRange(range) - 1] + 1 - projection.sourceIndices[range.location])
    }
    #if os(iOS)
    let color = UIColor(CaperTheme.terracotta).withAlphaComponent(0.24)
    let textColor = UIColor(CaperTheme.text)
    let linkTextColor = UIColor(CaperTheme.link)
    let spoilerFill = UIColor(CaperTheme.border)
    #else
    let color = NSColor(CaperTheme.terracotta).withAlphaComponent(0.24)
    let textColor = NSColor(CaperTheme.text)
    let linkTextColor = NSColor(CaperTheme.link)
    let spoilerFill = NSColor(CaperTheme.border)
    #endif
    storage.beginEditing()
    storage.removeAttribute(.backgroundColor, range: NSRange(location: 0, length: storage.length))
    storage.removeAttribute(.obliqueness, range: NSRange(location: 0, length: storage.length))
    storage.removeAttribute(.composerDelimiter, range: NSRange(location: 0, length: storage.length))
    storage.removeAttribute(.draftLink, range: NSRange(location: 0, length: storage.length))
    storage.removeAttribute(.kern, range: NSRange(location: 0, length: storage.length))
    for key in composerFormattingAttributes { storage.removeAttribute(key, range: NSRange(location: 0, length: storage.length)) }
    storage.addAttribute(.foregroundColor, value: textColor, range: NSRange(location: 0, length: storage.length))
    let baseFont = PlatformComposerFont(name: "Satoshi-Regular", size: 14) ?? PlatformComposerFont.systemFont(ofSize: 14)
    storage.addAttribute(.font, value: baseFont, range: NSRange(location: 0, length: storage.length))
    let visibleIndices = Set(projection.sourceIndices)
    for index in 0..<storage.length where !visibleIndices.contains(index) {
        storage.addAttribute(.composerDelimiter, value: true, range: NSRange(location: index, length: 1))
    }
    var displayed = 0
    var quoted: [Int] = []
    for run in projection.runs {
        let font = PlatformComposerFont(name: run.bold ? "Satoshi-Bold" : "Satoshi-Regular", size: 14) ?? baseFont
        for index in displayed..<(displayed + run.text.utf16.count) {
            let character = NSRange(location: projection.sourceIndices[index], length: 1)
            storage.addAttribute(.font, value: font, range: character)
            // The bundled Satoshi faces are upright; TextKit synthesizes the same
            // slant on both Apple clients instead of silently falling back to upright.
            if run.italic { storage.addAttribute(.obliqueness, value: 0.2, range: character) }
            if run.underline { storage.addAttribute(.underlineStyle, value: NSUnderlineStyle.single.rawValue, range: character) }
            if run.strikethrough { storage.addAttribute(.strikethroughStyle, value: NSUnderlineStyle.single.rawValue, range: character) }
            // Drafts show spoilers, marked by DraftLinkLayoutManager's rounded fill.
            if run.spoiler { storage.addAttribute(.spoilerFill, value: spoilerFill, range: character) }
            if run.quote {
                storage.addAttribute(.quoteBar, value: true, range: character)
                quoted.append(character.location)
            }
        }
        displayed += run.text.utf16.count
    }
    // A quoted line is indented past the bar DraftLinkLayoutManager draws
    // (3pt bar and 12pt gap, as web's quote padding).
    let quote = NSMutableParagraphStyle()
    quote.firstLineHeadIndent = NativePresentation.quoteIndent
    quote.headIndent = NativePresentation.quoteIndent
    var paragraph = NSRange(location: NSNotFound, length: 0)
    for location in quoted where !NSLocationInRange(location, paragraph) {
        paragraph = (storage.string as NSString).paragraphRange(for: NSRange(location: location, length: 0))
        storage.addAttribute(.paragraphStyle, value: quote, range: paragraph)
    }
    var offset = 0
    for segment in MessageLinks.segments(in: storage.string) {
        let range = NSRange(location: offset, length: segment.text.utf16.count)
        offset += range.length
        if segment.href != nil, !ranges.contains(where: { NSIntersectionRange($0, range).length > 0 }) {
            storage.addAttributes([.draftLink: range.location, .foregroundColor: linkTextColor], range: range)
            storage.addAttribute(.kern, value: DraftLinkLayoutManager.iconAdvance,
                                 range: NSRange(location: range.location, length: 1))
        }
    }
    for range in ranges { storage.addAttribute(.backgroundColor, value: color, range: range) }
    #if os(macOS)
    storage.removeAttribute(.toolTip, range: NSRange(location: 0, length: storage.length))
    for emoji in EmojiArtwork.namedRanges(in: storage.string) {
        storage.addAttribute(.toolTip, value: ":\(emoji.name):", range: emoji.range)
    }
    #endif
    storage.endEditing()
    for layout in storage.layoutManagers {
        layout.invalidateGlyphs(forCharacterRange: NSRange(location: 0, length: storage.length), changeInLength: 0, actualCharacterRange: nil)
    }
}
#endif

#if os(iOS)
public final class ComposerTextView: UITextView {
    public let composerGlyphs = ComposerGlyphs()
    private var navigationRange: NSRange?
    private var navigationAnchor = 0
    @objc private func moveVisible(_ command: UIKeyCommand) {
        guard markedTextRange == nil else { return }
        let extending = command.modifierFlags.contains(.shift)
        if !extending || navigationRange != selectedRange { navigationAnchor = selectedRange.location }
        let next = MessageFormatting.movedSelection(text, selection: selectedRange,
            backward: command.input == UIKeyCommand.inputLeftArrow, extending: extending,
            activeAtStart: selectedRange.length > 0 && navigationAnchor == NSMaxRange(selectedRange))
        selectedRange = next; navigationRange = next
    }
    public override func deleteBackward() {
        guard markedTextRange == nil else { super.deleteBackward(); return }
        guard let range = MessageFormatting.deletionRange(text, selection: selectedRange, backward: true) else { return }
        selectedRange = range
        super.deleteBackward()
    }
    public override func copy(_ sender: Any?) {
        guard selectedRange.length > 0 else { return }
        UIPasteboard.general.string = MessageFormatting.copiedText(text, selection: selectedRange)
    }
    public override func cut(_ sender: Any?) {
        guard isEditable, selectedRange.length > 0, markedTextRange == nil else { return }
        copy(sender); deleteBackward()
    }
    public var autocompleteActive: (() -> Bool)?
    public var handleKey: ((String) -> Void)?
    public var pasteAttachments: (() -> Bool)?
    public var formatSelection: ((MessageStyle) -> Void)?
    @objc func formatBold(_ sender: Any?) { formatSelection?(.bold) }
    @objc func formatItalic(_ sender: Any?) { formatSelection?(.italic) }
    @objc func formatUnderline(_ sender: Any?) { formatSelection?(.underline) }
    private var emojiTooltip: UILabel?
    private var emojiHover: Task<Void, Never>?
    private var inspectedRange: NSRange?
    private var holdPoint: CGPoint?

    public func hideEmojiName() {
        emojiHover?.cancel(); emojiHover = nil; inspectedRange = nil
        emojiTooltip?.removeFromSuperview(); emojiTooltip = nil
    }
    public override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil { hideEmojiName() }
    }
    private func emoji(at point: CGPoint) -> (range: NSRange, name: String, rect: CGRect)? {
        guard isEditable, !covered else { return nil }
        let containerPoint = CGPoint(x: point.x - textContainerInset.left, y: point.y - textContainerInset.top)
        let glyph = layoutManager.glyphIndex(for: containerPoint, in: textContainer)
        guard glyph < layoutManager.numberOfGlyphs else { return nil }
        let character = layoutManager.characterIndexForGlyph(at: glyph)
        guard let emoji = EmojiArtwork.namedRanges(in: text).first(where: { NSLocationInRange(character, $0.range) }) else { return nil }
        let glyphs = layoutManager.glyphRange(forCharacterRange: emoji.range, actualCharacterRange: nil)
        let rect = layoutManager.boundingRect(forGlyphRange: glyphs, in: textContainer)
            .offsetBy(dx: textContainerInset.left, dy: textContainerInset.top)
        guard rect.contains(point) else { return nil }
        return (emoji.range, emoji.name, rect)
    }
    private func showEmojiName(_ name: String, rect: CGRect) {
        guard let window else { return }
        emojiTooltip?.removeFromSuperview()
        let label = UILabel()
        label.text = ":\(name):"; label.font = UIFont(name: "Satoshi-Bold", size: 12)
        label.textColor = UIColor(CaperTheme.text); label.backgroundColor = UIColor(CaperTheme.blackout)
        label.textAlignment = .center; label.numberOfLines = 0; label.isUserInteractionEnabled = false
        label.layer.cornerRadius = 8; label.layer.masksToBounds = true
        label.layer.borderWidth = 1; label.layer.borderColor = UIColor(CaperTheme.border).cgColor
        label.accessibilityIdentifier = "emoji-name-tooltip"
        let safe = window.bounds.inset(by: window.safeAreaInsets).insetBy(dx: 12, dy: 12)
        let size = label.sizeThatFits(CGSize(width: min(260, safe.width - 18), height: .greatestFiniteMagnitude))
        let anchor = convert(rect, to: window)
        let width = size.width + 18, height = size.height + 14
        let above = anchor.minY - height - 6
        label.frame = CGRect(x: min(max(safe.minX, anchor.midX - width / 2), safe.maxX - width),
                             y: min(max(safe.minY, above >= safe.minY ? above : anchor.maxY + 6), safe.maxY - height), width: width, height: height)
        window.addSubview(label); emojiTooltip = label
        if !UIAccessibility.isReduceMotionEnabled {
            label.alpha = 0; label.transform = CGAffineTransform(translationX: 0, y: 4)
            UIView.animate(withDuration: 0.14, delay: 0, options: .curveEaseOut) { label.alpha = 1; label.transform = .identity }
        }
    }
    @objc public func inspectEmojiHold(_ gesture: UILongPressGestureRecognizer) {
        let point = gesture.location(in: self)
        switch gesture.state {
        case .began:
            hideEmojiName(); holdPoint = point
            if let emoji = emoji(at: point) { inspectedRange = emoji.range; showEmojiName(emoji.name, rect: emoji.rect) }
        case .changed:
            if let start = holdPoint, hypot(point.x - start.x, point.y - start.y) > 10 { hideEmojiName() }
        case .ended:
            emojiHover = Task { [weak self] in
                do { try await Task.sleep(for: .milliseconds(1500)) } catch { return }
                self?.hideEmojiName()
            }
        default: hideEmojiName()
        }
    }
    @objc public func inspectEmojiHover(_ gesture: UIHoverGestureRecognizer) {
        guard gesture.state != .ended, gesture.state != .cancelled,
              let emoji = emoji(at: gesture.location(in: self)) else { hideEmojiName(); return }
        guard inspectedRange != emoji.range else { return }
        hideEmojiName(); inspectedRange = emoji.range
        emojiHover = Task { [weak self] in
            do { try await Task.sleep(for: .milliseconds(400)) } catch { return }
            self?.showEmojiName(emoji.name, rect: emoji.rect)
        }
    }
    public override func paste(_ sender: Any?) {
        if pasteAttachments?() != true { super.paste(sender) }
    }
    public override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(paste(_:)), isEditable, pasteAttachments != nil,
           NativeAttachmentPaste.hasAttachments { return true }
        return super.canPerformAction(action, withSender: sender)
    }
    /// Once Browse has covered the conversation, UIKit gave the composer its
    /// earlier focus back as the conversation returned, reopening the keyboard
    /// over it. Focus waits for a touch (or VoiceOver) until a moment after.
    private var focusHeld = false
    private var covered = false
    private var focusRelease: Task<Void, Never>?
    public override var canBecomeFirstResponder: Bool { !focusHeld && super.canBecomeFirstResponder }
    public func setCovered(_ covered: Bool) {
        guard covered != self.covered else { return }
        self.covered = covered
        focusRelease?.cancel(); focusRelease = nil
        if covered { hideEmojiName(); focusHeld = true; return }
        focusRelease = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(1)) } catch { return }
            self?.focusHeld = false
        }
    }
    public func touched() { hideEmojiName(); if !covered { focusRelease?.cancel(); focusRelease = nil; focusHeld = false } }
    public override func accessibilityActivate() -> Bool { touched(); return super.accessibilityActivate() }
    public override var keyCommands: [UIKeyCommand]? {
        var formatting = [UIKeyCommand(input: "b", modifierFlags: .command, action: #selector(formatBold(_:))),
                          UIKeyCommand(input: "i", modifierFlags: .command, action: #selector(formatItalic(_:))),
                          UIKeyCommand(input: "u", modifierFlags: .command, action: #selector(formatUnderline(_:)))]
        if markedTextRange == nil {
            for input in [UIKeyCommand.inputLeftArrow, UIKeyCommand.inputRightArrow] {
                for modifiers in [UIKeyModifierFlags(), .shift] {
                    let command = UIKeyCommand(input: input, modifierFlags: modifiers, action: #selector(moveVisible(_:)))
                    command.wantsPriorityOverSystemBehavior = true
                    formatting.append(command)
                }
            }
        }
        guard markedTextRange == nil, autocompleteActive?() == true else { return (super.keyCommands ?? []) + formatting }
        return [UIKeyCommand.inputUpArrow, UIKeyCommand.inputDownArrow, "\r", "\t", UIKeyCommand.inputEscape].map {
            let command = UIKeyCommand(input: $0, modifierFlags: [], action: #selector(autocompleteKey(_:)))
            command.wantsPriorityOverSystemBehavior = true
            return command
        } + formatting
    }
    @objc private func autocompleteKey(_ command: UIKeyCommand) { if let input = command.input { handleKey?(input) } }
}

extension MessageFormattingAction {
    /// The edit menu's formatting groups, in the toolbar's order: the inline
    /// styles, then (after the menu's divider) Quote and Spoiler. A style the
    /// selection already has is on.
    @MainActor public static func editMenus(text: String, selection: NSRange, format: @escaping (MessageStyle) -> Void) -> [UIMenuElement] {
        let pressed = MessageFormatting.selectionStyles(text, selection: selection)
        return [false, true].map { block -> UIMenuElement in
            let actions = all.filter { $0.block == block }.map { item -> UIMenuElement in
                UIAction(title: item.title, image: UIImage(systemName: item.symbol), discoverabilityTitle: item.title,
                         state: pressed.contains(item.style) ? .on : .off) { _ in format(item.style) }
            }
            return UIMenu(title: "", options: .displayInline, children: actions)
        }
    }
}

/// Sees a finger land on the composer, then steps aside for its own gestures.
public final class TouchDownObserver: UIGestureRecognizer {
    public var touched: (() -> Void)?
    public override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) { touched?(); state = .failed }
}

struct NativeMessageTextEditor: UIViewRepresentable {
    @Binding var text: String
    let placeholder: String
    let controller: ComposerAutocompleteController
    /// Who `@` may suggest in this conversation.
    let mentions: MentionSource
    let submit: () -> Void
    var submitOnReturn = true
    var editable = true
    var accessibilityID = "message-composer"
    var pasteAttachments: (() -> Bool)? = nil
    /// Quick-message cards suppress suggestions but still preview their recipient and team tags.
    var highlightSource: MentionSource? = nil
    var conversation = ""
    @Environment(\.composerCovered) private var covered
    /// False while a workspace dialog covers the conversation.
    @Environment(\.isEnabled) private var isEnabled
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeUIView(context: Context) -> UITextView {
        let storage = NSTextStorage()
        let layout = DraftLinkLayoutManager(); storage.addLayoutManager(layout)
        let container = NSTextContainer(size: CGSize(width: 0, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true; layout.addTextContainer(container)
        let view = ComposerTextView(frame: .zero, textContainer: container); view.delegate = context.coordinator
        layout.delegate = view.composerGlyphs
        let touchDown = TouchDownObserver(target: nil, action: nil)
        touchDown.cancelsTouchesInView = false; touchDown.delaysTouchesEnded = false
        touchDown.touched = { [weak view] in view?.touched() }
        view.addGestureRecognizer(touchDown)
        let hold = UILongPressGestureRecognizer(target: view, action: #selector(ComposerTextView.inspectEmojiHold(_:)))
        hold.minimumPressDuration = 0.5; hold.allowableMovement = 10
        hold.cancelsTouchesInView = false; hold.delegate = context.coordinator
        view.addGestureRecognizer(hold)
        view.addGestureRecognizer(UIHoverGestureRecognizer(target: view, action: #selector(ComposerTextView.inspectEmojiHover(_:))))
        view.autocompleteActive = { [weak controller] in controller?.match != nil }
        view.handleKey = { [weak coordinator = context.coordinator] in coordinator?.handle($0) }
        view.formatSelection = { [weak coordinator = context.coordinator] in coordinator?.format($0) }
        view.backgroundColor = .clear; view.textColor = UIColor(CaperTheme.text); view.font = UIFont(name: "Satoshi-Regular", size: 14)
        // The text container already supplies 5 points of line-fragment padding.
        view.textContainerInset = UIEdgeInsets(top: 11, left: 0, bottom: 11, right: 7)
        view.isScrollEnabled = true; view.accessibilityIdentifier = "message-composer"
        view.accessibilityLabel = placeholder
        context.coordinator.view = view; context.coordinator.refresh(view); return view
    }
    func updateUIView(_ view: UITextView, context: Context) {
        (view as? ComposerTextView)?.setCovered(covered)
        (view as? ComposerTextView)?.pasteAttachments = pasteAttachments
        // A disabled composer (under a dialog) must not keep taking keystrokes.
        view.isEditable = editable && isEnabled
        if !editable { (view as? ComposerTextView)?.hideEmojiName() }
        if !isEnabled, view.isFirstResponder {
            // After this update, and only if nothing else (the dialog's own field) took focus first.
            Task { @MainActor [weak view] in
                if let view, view.isFirstResponder { _ = view.resignFirstResponder() }
            }
        }
        view.accessibilityIdentifier = accessibilityID
        // The conversation can change under the same editor (a channel, then a DM).
        view.accessibilityLabel = placeholder
        let mentionsChanged = context.coordinator.parent.mentions != mentions || context.coordinator.parent.highlightSource != highlightSource
        context.coordinator.adopt(self)
        if view.text != text { (view as? ComposerTextView)?.hideEmojiName(); view.text = text; context.coordinator.typing.end(); context.coordinator.dismissed = false; context.coordinator.refresh(view) }
        // Members can finish loading while an `@` query is open.
        else if mentionsChanged { context.coordinator.refresh(view) }
        view.accessibilityValue = text; view.accessibilityHint = "Use the Send button when composing with the iPhone keyboard."
    }
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        let height = uiView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
        return CGSize(width: width, height: min(174, max(42, height)))
    }
    final class Coordinator: NSObject, UITextViewDelegate, UIGestureRecognizerDelegate {
        var parent: NativeMessageTextEditor; weak var view: UITextView?; var dismissed = false
        private var highlighting = false
        var typing = ComposerTyping()
        init(_ parent: NativeMessageTextEditor) { self.parent = parent }
        /// The editor's next configuration. The view is reused across
        /// conversations and threads, so another one ends the style typing
        /// carries, even when its draft and caret are identical.
        func adopt(_ next: NativeMessageTextEditor) {
            if parent.conversation != next.conversation { typing.end() }
            parent = next
        }
        func textView(_ view: UITextView, shouldChangeTextIn range: NSRange, replacementText replacement: String) -> Bool {
            guard !highlighting, view.markedTextRange == nil,
                  view.undoManager?.isUndoing != true, view.undoManager?.isRedoing != true else { return true }
            let proposed = (view.text as NSString).replacingCharacters(in: range, with: replacement)
            let result = typing.repairEdit(view.text, text: proposed, selection: NSRange(location: range.location + replacement.utf16.count, length: 0), replacing: range)
            guard result.text != proposed,
                  let all = view.textRange(from: view.beginningOfDocument, to: view.endOfDocument) else { return true }
            highlighting = true
            view.replace(all, withText: result.text); view.selectedRange = result.selection
            highlighting = false
            parent.text = result.text; dismissed = false; refresh(view)
            return false
        }
        func format(_ style: MessageStyle) {
            guard let view, view.isEditable, view.markedTextRange == nil,
                  let result = MessageFormatting.toggle(view.text, selection: view.selectedRange, style: style),
                  result.text.unicodeScalars.count <= 4000 else { return }
            guard result.text != view.text,
                  let range = view.textRange(from: view.beginningOfDocument, to: view.endOfDocument) else { return }
            view.replace(range, withText: result.text); view.selectedRange = result.selection
            parent.text = result.text; dismissed = false; refresh(view)
        }
        func textView(_ textView: UITextView, editMenuForTextIn range: NSRange, suggestedActions: [UIMenuElement]) -> UIMenu? {
            guard textView.isEditable else { return nil }
            return UIMenu(children: MessageFormattingAction.editMenus(text: textView.text, selection: range) { [weak self] in self?.format($0) }
                + suggestedActions)
        }
        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer) -> Bool { true }
        func scrollViewDidScroll(_ scrollView: UIScrollView) { (scrollView as? ComposerTextView)?.hideEmojiName() }
        func textViewDidBeginEditing(_ textView: UITextView) { refresh(textView) }
        func textViewDidEndEditing(_ textView: UITextView) { typing.end(); (textView as? ComposerTextView)?.hideEmojiName(); parent.controller.update(nil) }
        func textViewDidChange(_ textView: UITextView) { guard !highlighting else { return }; (textView as? ComposerTextView)?.hideEmojiName(); parent.text = textView.text; dismissed = false; refresh(textView) }
        func textViewDidChangeSelection(_ textView: UITextView) { guard !highlighting else { return }; typing.observe(textView.text, selection: textView.selectedRange); dismissed = false; refresh(textView) }
        func refresh(_ textView: UITextView) {
            guard !highlighting else { return }
            if textView.markedTextRange == nil {
                highlighting = true
                textView.selectedRange = MessageFormatting.normalizedSelection(textView.text, selection: textView.selectedRange)
                highlighting = false
            }
            highlight(textView)
            parent.controller.formattingSelection = textView.selectedRange.length > 0
            let styles = MessageFormatting.selectionStyles(textView.text, selection: textView.selectedRange)
            if parent.controller.formattingStyles != styles { parent.controller.formattingStyles = styles }
            parent.controller.formatAction = { [weak self] in self?.format($0) }
            let match = dismissed || !textView.isFirstResponder ? nil : ComposerAutocomplete.match(text: textView.text, selection: textView.selectedRange, markedText: textView.markedTextRange != nil, mentions: parent.mentions)
            parent.controller.update(match)
            parent.controller.acceptAction = { [weak self] index in self?.accept(index) }
            parent.controller.insertAction = { [weak self, selection = textView.selectedRange] emoji in self?.insert(emoji, selection: selection) }
            parent.controller.dictateAction = { [weak self] in self?.dictate($0) ?? false }
            parent.controller.restoreFocusAction = { [weak self] in
                guard let self, self.parent.editable else { return }
                (self.view as? ComposerTextView)?.touched()
                self.view?.becomeFirstResponder()
            }
            parent.controller.requestFocusAction = { [weak self] in
                guard let self, self.parent.editable, self.parent.isEnabled, !self.parent.covered,
                      let view = self.view, let window = view.window, !window.isHidden else { return false }
                var top = window.rootViewController
                while let presented = top?.presentedViewController { top = presented }
                guard let top, !top.isBeingDismissed, !top.isBeingPresented,
                      view.isDescendant(of: top.view) else { return false }
                (view as? ComposerTextView)?.touched()
                return view.becomeFirstResponder()
            }
        }
        func highlight(_ view: UITextView) {
            guard !highlighting, view.markedTextRange == nil else { return }
            highlighting = true
            highlightDraft(view.textStorage, source: parent.highlightSource ?? parent.mentions)
            view.typingAttributes.removeValue(forKey: .composerDelimiter)
            view.typingAttributes.removeValue(forKey: .obliqueness)
            view.typingAttributes[.font] = UIFont(name: "Satoshi-Regular", size: 14)
            view.typingAttributes.removeValue(forKey: .backgroundColor)
            view.typingAttributes.removeValue(forKey: .draftLink)
            view.typingAttributes.removeValue(forKey: .kern)
            for key in composerFormattingAttributes { view.typingAttributes.removeValue(forKey: key) }
            view.typingAttributes[.foregroundColor] = UIColor(CaperTheme.text)
            highlighting = false
        }
        func insert(_ emoji: String, selection: NSRange) {
            guard parent.editable, let view,
                  let result = EmojiAutocomplete.inserting(emoji, in: view.text, selection: selection) else { return }
            view.text = result.text; view.selectedRange = result.selection; parent.text = result.text
            highlight(view)
            parent.controller.update(nil)
        }
        func dictate(_ words: String) -> Bool {
            guard parent.editable, let view else { return false }
            let selection = view.selectedRange
            let text = VoiceMessage.dictationText(in: view.text, selection: selection, words: words)
            guard !text.isEmpty, let result = EmojiAutocomplete.inserting(text, in: view.text, selection: selection) else { return false }
            view.text = result.text; view.selectedRange = result.selection; parent.text = result.text
            highlight(view)
            parent.controller.update(nil)
            return true
        }
        func handle(_ key: String) {
            guard parent.controller.match != nil, view?.markedTextRange == nil else { return }
            if key == UIKeyCommand.inputUpArrow || key == UIKeyCommand.inputDownArrow {
                parent.controller.move(down: key == UIKeyCommand.inputDownArrow)
            } else if key == UIKeyCommand.inputEscape { dismissed = true; parent.controller.update(nil) }
            else { accept(parent.controller.selected) }
        }
        func accept(_ index: Int) {
            guard let view, let match = parent.controller.match,
                  let result = ComposerAutocomplete.inserting(choice: index, in: view.text, match: match) else { return }
            view.text = result.text; view.selectedRange = result.selection; parent.text = result.text
            highlight(view)
            parent.controller.update(nil); view.becomeFirstResponder()
        }
    }
}
#elseif os(macOS)
public final class ComposerTextView: NSTextView {
    public let composerGlyphs = ComposerGlyphs()
    /// Native hosts draw their placeholder here; SwiftUI hosts leave it empty.
    public var placeholder = "" {
        didSet { if oldValue != placeholder { needsDisplay = true } }
    }
    public override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        if string.isEmpty, !placeholder.isEmpty {
            (placeholder as NSString).draw(at: NSPoint(x: textContainerInset.width + 5, y: textContainerInset.height),
                withAttributes: [.font: font ?? NSFont.systemFont(ofSize: 14), .foregroundColor: NSColor(CaperTheme.muted)])
        }
    }
    private func moveVisible(backward: Bool, extending: Bool) {
        guard !hasMarkedText() else { return }
        let previous = selectedRange()
        let activeAtStart = selectionAffinity == .upstream
        let anchor = activeAtStart ? NSMaxRange(previous) : previous.location
        let next = MessageFormatting.movedSelection(string, selection: previous, backward: backward, extending: extending, activeAtStart: activeAtStart)
        setSelectedRange(next, affinity: extending && next.location < anchor ? .upstream : .downstream, stillSelecting: false)
    }
    public override func moveLeft(_ sender: Any?) { if hasMarkedText() { super.moveLeft(sender) } else { moveVisible(backward: true, extending: false) } }
    public override func moveRight(_ sender: Any?) { if hasMarkedText() { super.moveRight(sender) } else { moveVisible(backward: false, extending: false) } }
    public override func moveLeftAndModifySelection(_ sender: Any?) { if hasMarkedText() { super.moveLeftAndModifySelection(sender) } else { moveVisible(backward: true, extending: true) } }
    public override func moveRightAndModifySelection(_ sender: Any?) { if hasMarkedText() { super.moveRightAndModifySelection(sender) } else { moveVisible(backward: false, extending: true) } }
    public override func deleteBackward(_ sender: Any?) {
        guard !hasMarkedText() else { super.deleteBackward(sender); return }
        guard let range = MessageFormatting.deletionRange(string, selection: selectedRange(), backward: true) else { return }
        setSelectedRange(range); super.deleteBackward(sender)
    }
    public override func deleteForward(_ sender: Any?) {
        guard !hasMarkedText() else { super.deleteForward(sender); return }
        guard let range = MessageFormatting.deletionRange(string, selection: selectedRange(), backward: false) else { return }
        setSelectedRange(range); super.deleteForward(sender)
    }
    public override func copy(_ sender: Any?) {
        guard selectedRange().length > 0 else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(MessageFormatting.copiedText(string, selection: selectedRange()), forType: .string)
    }
    public override func cut(_ sender: Any?) {
        guard isEditable, selectedRange().length > 0, !hasMarkedText() else { return }
        copy(sender); deleteBackward(sender)
    }
    public var handleKey: ((NSEvent) -> Bool)?
    public var focusChanged: ((Bool) -> Void)?
    public var formatSelection: ((MessageStyle) -> Void)?
    @objc public func formatBold(_ sender: Any?) { formatSelection?(.bold) }
    @objc public func formatItalic(_ sender: Any?) { formatSelection?(.italic) }
    @objc public func formatUnderline(_ sender: Any?) { formatSelection?(.underline) }
    @objc public func formatStrikethrough(_ sender: Any?) { formatSelection?(.strikethrough) }
    @objc public func formatQuote(_ sender: Any?) { formatSelection?(.quote) }
    @objc public func formatSpoiler(_ sender: Any?) { formatSelection?(.spoiler) }
    /// The action each formatting menu item sends.
    nonisolated public static func formatSelector(_ style: MessageStyle) -> Selector {
        switch style {
        case .bold: return #selector(ComposerTextView.formatBold(_:))
        case .italic: return #selector(ComposerTextView.formatItalic(_:))
        case .underline: return #selector(ComposerTextView.formatUnderline(_:))
        case .strikethrough: return #selector(ComposerTextView.formatStrikethrough(_:))
        case .quote: return #selector(ComposerTextView.formatQuote(_:))
        default: return #selector(ComposerTextView.formatSpoiler(_:))
        }
    }
    /// "B — Bold" for styles with a shortcut, otherwise the style's name.
    nonisolated public static func formatMenuTitle(_ item: MessageFormattingAction) -> String {
        item.key.map { "\($0.uppercased()) — \(item.title)" } ?? item.title
    }
    /// The Edit menu's formatting items show which styles the selection has.
    public override func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        guard let item = MessageFormattingAction.all.first(where: { Self.formatSelector($0.style) == menuItem.action }) else {
            return super.validateMenuItem(menuItem)
        }
        menuItem.state = MessageFormatting.selectionStyles(string, selection: selectedRange()).contains(item.style) ? .on : .off
        return isEditable
    }
    public override func menu(for event: NSEvent) -> NSMenu? {
        let menu = super.menu(for: event) ?? NSMenu()
        if isEditable {
            // The toolbar's buttons and order; a checkmark marks a style the selection has.
            let pressed = MessageFormatting.selectionStyles(string, selection: selectedRange())
            var index = 0
            for item in MessageFormattingAction.all {
                if item.style == .quote { menu.insertItem(.separator(), at: index); index += 1 }
                let entry = NSMenuItem(title: Self.formatMenuTitle(item), action: Self.formatSelector(item.style), keyEquivalent: item.key ?? "")
                entry.target = self
                entry.image = NSImage(systemSymbolName: item.symbol, accessibilityDescription: nil)
                entry.state = pressed.contains(item.style) ? .on : .off
                menu.insertItem(entry, at: index); index += 1
            }
            menu.insertItem(.separator(), at: index)
        }
        return menu
    }
    /// The AppKit host forwards files, including mixed text/file drags. The
    /// shipping SwiftUI host leaves this nil and owns its own drop destination.
    public weak var attachmentDropDestination: AppKitFileDropView? {
        didSet { updateDragTypeRegistration() }
    }
    public override var acceptableDragTypes: [NSPasteboard.PasteboardType] {
        [.string] + (attachmentDropDestination == nil ? [] : AppKitFileDropView.dragTypes)
    }
    /// A drag carrying text and files together still reaches the text view;
    /// refuse it rather than insert a path.
    private func carriesFiles(_ drag: NSDraggingInfo) -> Bool {
        if attachmentDropDestination != nil, AppKitFileDropView.carriesFiles(drag.draggingPasteboard) { return true }
        return drag.draggingPasteboard.canReadObject(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])
    }
    public override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        carriesFiles(sender) ? attachmentDropDestination?.draggingEntered(sender) ?? [] : super.draggingEntered(sender)
    }
    public override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        carriesFiles(sender) ? attachmentDropDestination?.draggingUpdated(sender) ?? [] : super.draggingUpdated(sender)
    }
    public override func draggingExited(_ sender: NSDraggingInfo?) {
        attachmentDropDestination?.draggingExited(sender)
        super.draggingExited(sender)
    }
    public override func draggingEnded(_ sender: NSDraggingInfo) {
        attachmentDropDestination?.draggingEnded(sender)
        super.draggingEnded(sender)
    }
    public override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool {
        carriesFiles(sender) ? attachmentDropDestination?.prepareForDragOperation(sender) ?? false : super.prepareForDragOperation(sender)
    }
    public override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        carriesFiles(sender) ? attachmentDropDestination?.performDragOperation(sender) ?? false : super.performDragOperation(sender)
    }
    public override func concludeDragOperation(_ sender: NSDraggingInfo?) {
        attachmentDropDestination?.concludeDragOperation(sender)
        super.concludeDragOperation(sender)
    }
    public var pasteAttachments: (() -> Bool)?
    public override func paste(_ sender: Any?) {
        if pasteAttachments?() != true { super.paste(sender) }
    }
    public override func keyDown(with event: NSEvent) { if handleKey?(event) != true { super.keyDown(with: event) } }
    public override func becomeFirstResponder() -> Bool { let result = super.becomeFirstResponder(); if result { focusChanged?(true) }; return result }
    public override func resignFirstResponder() -> Bool { let result = super.resignFirstResponder(); if result { focusChanged?(false) }; return result }
}

struct NativeMessageTextEditor: NSViewRepresentable {
    @Binding var text: String
    let placeholder: String
    let controller: ComposerAutocompleteController
    /// Who `@` may suggest in this conversation.
    let mentions: MentionSource
    let submit: () -> Void
    var submitOnReturn = true
    var editable = true
    var accessibilityID = "message-composer"
    var pasteAttachments: (() -> Bool)? = nil
    /// Quick-message cards suppress suggestions but still preview their recipient and team tags.
    var highlightSource: MentionSource? = nil
    var conversation = ""
    /// False while a workspace dialog covers the conversation.
    @Environment(\.isEnabled) private var isEnabled
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView(); let view = ComposerTextView()
        view.textContainer?.replaceLayoutManager(DraftLinkLayoutManager())
        view.layoutManager?.delegate = view.composerGlyphs
        scroll.documentView = view; scroll.hasVerticalScroller = true; scroll.drawsBackground = false
        view.delegate = context.coordinator; view.drawsBackground = false; view.textColor = NSColor(CaperTheme.text)
        view.font = NSFont(name: "Satoshi-Regular", size: 14); view.textContainerInset = NSSize(width: 0, height: 10)
        // AppKit sizes the text view: its width follows the scroll view and its height the text.
        view.minSize = NSSize(width: 0, height: 42)
        view.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        view.isVerticallyResizable = true; view.isHorizontallyResizable = false; view.autoresizingMask = [.width]
        view.textContainer?.containerSize = NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        view.textContainer?.widthTracksTextView = true
        view.isRichText = false
        view.updateDragTypeRegistration()
        view.setAccessibilityIdentifier("message-composer"); view.setAccessibilityLabel(placeholder)
        view.setAccessibilityHelp("Return sends. Shift-Return adds a new line."); context.coordinator.view = view
        view.handleKey = { [weak coordinator = context.coordinator] in coordinator?.handle($0) == true }
        view.formatSelection = { [weak coordinator = context.coordinator] in coordinator?.format($0) }
        view.focusChanged = { [weak coordinator = context.coordinator] focused in
            guard let coordinator, let view = coordinator.view else { return }
            if focused { coordinator.refresh(view) } else { coordinator.parent.controller.update(nil) }
        }
        context.coordinator.refresh(view); return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let mentionsChanged = context.coordinator.parent.mentions != mentions || context.coordinator.parent.highlightSource != highlightSource
        context.coordinator.adopt(self)
        guard let view = scroll.documentView as? ComposerTextView else { return }
        view.pasteAttachments = pasteAttachments
        // A disabled composer (under a dialog) must not keep taking keystrokes.
        view.isEditable = editable && isEnabled
        if !isEnabled, view.window?.firstResponder === view {
            // After this update, and only if nothing else (the dialog's own field) took focus first.
            Task { @MainActor [weak view] in
                if let view, let window = view.window, window.firstResponder === view { _ = window.makeFirstResponder(nil) }
            }
        }
        view.setAccessibilityIdentifier(accessibilityID)
        // The conversation can change under the same editor (a channel, then a DM).
        view.setAccessibilityLabel(placeholder)
        view.setAccessibilityHelp(submitOnReturn ? "Return sends. Shift-Return adds a new line." : "Use the Send button. Return adds a new line.")
        // The text view reports its own text as its accessibility value; setting
        // it would replace the text on every update.
        if view.string != text { view.string = text; context.coordinator.typing.end(); context.coordinator.dismissed = false; context.coordinator.refresh(view) }
        // Members can finish loading while an `@` query is open.
        else if mentionsChanged { context.coordinator.refresh(view) }
    }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSScrollView, context: Context) -> CGSize? {
        guard let width = proposal.width, let view = nsView.documentView as? ComposerTextView else { return nil }
        // Measure on a scratch text system. Resizing the live text view here
        // invalidates SwiftUI's layout, which measures again, without end.
        var attributes: [NSAttributedString.Key: Any] = [:]
        if let font = view.font { attributes[.font] = font }
        // Include the icon's reserved width and hide delimiters just as in the live editor.
        let storage = view.textStorage.map { NSTextStorage(attributedString: $0) }
            ?? NSTextStorage(string: view.string, attributes: attributes)
        let layout = NSLayoutManager(); storage.addLayoutManager(layout)
        layout.delegate = view.composerGlyphs
        let container = NSTextContainer(size: NSSize(width: width.isFinite ? max(40, width - 16) : CGFloat.greatestFiniteMagnitude,
                                                     height: CGFloat.greatestFiniteMagnitude))
        layout.addTextContainer(container); layout.ensureLayout(for: container)
        let height = layout.usedRect(for: container).height + 20
        return CGSize(width: width, height: min(174, max(42, height)))
    }
    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: NativeMessageTextEditor; weak var view: ComposerTextView?; var dismissed = false
        private var highlighting = false
        var typing = ComposerTyping()
        init(_ parent: NativeMessageTextEditor) { self.parent = parent }
        /// The editor's next configuration. The view is reused across
        /// conversations and threads, so another one ends the style typing
        /// carries, even when its draft and caret are identical.
        func adopt(_ next: NativeMessageTextEditor) {
            if parent.conversation != next.conversation { typing.end() }
            parent = next
        }
        func textView(_ view: NSTextView, shouldChangeTextIn affectedCharRange: NSRange, replacementString: String?) -> Bool {
            MainActor.assumeIsolated {
                guard !highlighting, !view.hasMarkedText(), let replacement = replacementString,
                      view.undoManager?.isUndoing != true, view.undoManager?.isRedoing != true else { return true }
                let proposed = (view.string as NSString).replacingCharacters(in: affectedCharRange, with: replacement)
                let result = typing.repairEdit(view.string, text: proposed, selection: NSRange(location: affectedCharRange.location + replacement.utf16.count, length: 0), replacing: affectedCharRange)
                guard result.text != proposed else { return true }
                highlighting = true
                view.insertText(result.text, replacementRange: NSRange(location: 0, length: view.string.utf16.count))
                view.setSelectedRange(result.selection)
                highlighting = false
                parent.text = result.text; dismissed = false
                if let composer = view as? ComposerTextView { refresh(composer) }
                return false
            }
        }
        // Older macOS SDKs (15.x) don't mark NSTextViewDelegate main-actor, so these delegate
        // calls are nonisolated there. AppKit still delivers them on the main thread.
        func textDidChange(_ notification: Notification) {
            MainActor.assumeIsolated {
                guard !highlighting, let view else { return }
                if parent.text != view.string { parent.text = view.string }
                dismissed = false; refresh(view)
            }
        }
        func textViewDidChangeSelection(_ notification: Notification) { MainActor.assumeIsolated { guard !highlighting, let view else { return }; typing.observe(view.string, selection: view.selectedRange()); dismissed = false; refresh(view) } }
        func textDidEndEditing(_ notification: Notification) { MainActor.assumeIsolated { typing.end() } }
        @MainActor func refresh(_ view: ComposerTextView) {
            guard !highlighting else { return }
            if !view.hasMarkedText() {
                highlighting = true
                let range = MessageFormatting.normalizedSelection(view.string, selection: view.selectedRange())
                if range != view.selectedRange() { view.setSelectedRange(range, affinity: view.selectionAffinity, stillSelecting: false) }
                highlighting = false
            }
            highlight(view)
            parent.controller.formattingSelection = view.selectedRange().length > 0
            let styles = MessageFormatting.selectionStyles(view.string, selection: view.selectedRange())
            if parent.controller.formattingStyles != styles { parent.controller.formattingStyles = styles }
            parent.controller.formatAction = { [weak self] in self?.format($0) }
            let match = dismissed || view.window?.firstResponder !== view ? nil : ComposerAutocomplete.match(text: view.string, selection: view.selectedRange(), markedText: view.hasMarkedText(), mentions: parent.mentions)
            parent.controller.update(match); parent.controller.acceptAction = { [weak self] in self?.accept($0) }
            parent.controller.insertAction = { [weak self, selection = view.selectedRange()] in self?.insert($0, selection: selection) }
            parent.controller.dictateAction = { [weak self] in self?.dictate($0) ?? false }
            parent.controller.restoreFocusAction = { [weak self] in
                guard let self, self.parent.editable, let view = self.view else { return }
                view.window?.makeFirstResponder(view)
            }
            parent.controller.requestFocusAction = { [weak self] in
                guard let self, self.parent.editable, self.parent.isEnabled,
                      let view = self.view, let window = view.window,
                      window.isKeyWindow, window.attachedSheet == nil else { return false }
                return window.makeFirstResponder(view)
            }
        }
        @MainActor func highlight(_ view: ComposerTextView) {
            guard !highlighting, !view.hasMarkedText(), let storage = view.textStorage else { return }
            highlighting = true
            highlightDraft(storage, source: parent.highlightSource ?? parent.mentions)
            view.typingAttributes.removeValue(forKey: .composerDelimiter)
            view.typingAttributes.removeValue(forKey: .obliqueness)
            view.typingAttributes[.font] = NSFont(name: "Satoshi-Regular", size: 14)
            view.typingAttributes.removeValue(forKey: .backgroundColor)
            view.typingAttributes.removeValue(forKey: .toolTip)
            view.typingAttributes.removeValue(forKey: .draftLink)
            view.typingAttributes.removeValue(forKey: .kern)
            for key in composerFormattingAttributes { view.typingAttributes.removeValue(forKey: key) }
            view.typingAttributes[.foregroundColor] = NSColor(CaperTheme.text)
            highlighting = false
        }
        @MainActor func insert(_ emoji: String, selection: NSRange) {
            guard parent.editable, let view,
                  let result = EmojiAutocomplete.inserting(emoji, in: view.string, selection: selection) else { return }
            view.string = result.text; view.setSelectedRange(result.selection); parent.text = result.text
            highlight(view)
            parent.controller.update(nil)
        }
        @MainActor func dictate(_ words: String) -> Bool {
            guard parent.editable, let view else { return false }
            let selection = view.selectedRange()
            let text = VoiceMessage.dictationText(in: view.string, selection: selection, words: words)
            guard !text.isEmpty, let result = EmojiAutocomplete.inserting(text, in: view.string, selection: selection) else { return false }
            view.string = result.text; view.setSelectedRange(result.selection); parent.text = result.text
            highlight(view)
            parent.controller.update(nil)
            return true
        }
        @MainActor func handle(_ event: NSEvent) -> Bool {
            guard let view, !view.hasMarkedText() else { return false }
            if event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,
               let key = event.charactersIgnoringModifiers, let item = MessageFormattingAction.shortcut(key) {
                format(item.style); return true
            }
            let key = event.keyCode
            if parent.controller.match != nil {
                if key == 125 || key == 126 { parent.controller.move(down: key == 125); return true }
                if (key == 36 || key == 48) && !event.modifierFlags.contains(.shift) { accept(parent.controller.selected); return true }
                if key == 53 { dismissed = true; parent.controller.update(nil); return true }
            }
            if key == 36 && parent.submitOnReturn && !event.modifierFlags.contains(.shift) { parent.submit(); return true }
            return false
        }
        @MainActor func format(_ style: MessageStyle) {
            guard let view, view.isEditable, !view.hasMarkedText(),
                  let result = MessageFormatting.toggle(view.string, selection: view.selectedRange(), style: style),
                  result.text.unicodeScalars.count <= 4000,
                  result.text != view.string,
                  view.shouldChangeText(in: NSRange(location: 0, length: (view.string as NSString).length), replacementString: result.text) else { return }
            view.textStorage?.replaceCharacters(in: NSRange(location: 0, length: (view.string as NSString).length), with: result.text)
            view.setSelectedRange(result.selection); view.didChangeText()
            parent.text = result.text; dismissed = false; refresh(view)
        }
        @MainActor func accept(_ index: Int) {
            guard let view, let match = parent.controller.match,
                  let result = ComposerAutocomplete.inserting(choice: index, in: view.string, match: match) else { return }
            view.string = result.text; view.setSelectedRange(result.selection); parent.text = result.text
            highlight(view)
            parent.controller.update(nil); view.window?.makeFirstResponder(view)
        }
    }
}
#endif
