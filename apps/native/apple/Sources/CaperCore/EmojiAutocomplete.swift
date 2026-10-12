import SwiftUI
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

struct EmojiAutocompleteMatch: Equatable {
    let replacementRange: NSRange
    let choices: [EmojiCatalogEntry]
}

enum EmojiAutocomplete {
    private static let defaults = ["1f44d", "1f600", "2764", "1f389", "1f680", "1f440"]

    static func match(text: String, selection: NSRange, markedText: Bool,
                      catalog: [EmojiCatalogEntry] = EmojiArtwork.choices) -> EmojiAutocompleteMatch? {
        guard !markedText, selection.length == 0,
              let caret = Range(NSRange(location: selection.location, length: 0), in: text)?.lowerBound else { return nil }
        if caret < text.endIndex, isQuery(text[caret]) || text[caret] == ":" { return nil }

        var start = caret
        while start > text.startIndex {
            let previous = text.index(before: start)
            guard isQuery(text[previous]) else { break }
            start = previous
        }
        guard start > text.startIndex else { return nil }
        let colon = text.index(before: start)
        guard text[colon] == ":" else { return nil }
        if colon > text.startIndex {
            let before = text[text.index(before: colon)]
            guard before.isWhitespace || "([{".contains(before) else { return nil }
        }
        let raw = String(text[start..<caret])
        // One character is an emoticon (":D", ":P", ":3"), not a search; Return sends it as typed.
        guard raw.count != 1 else { return nil }
        let query = normalize(raw)
        let selectable = catalog.filter(\.selectable)
        let choices: [EmojiCatalogEntry]
        if raw.isEmpty {
            let byID = Dictionary(uniqueKeysWithValues: selectable.map { ($0.id, $0) })
            choices = defaults.compactMap { byID[$0] }
        } else {
            choices = selectable.enumerated().compactMap { offset, entry -> (Int, Int, EmojiCatalogEntry)? in
                let name = normalize(entry.name), keywords = normalize(entry.keywords)
                let rank: Int
                if name == query { rank = 0 }
                else if name.hasPrefix(query) { rank = 1 }
                else if keywords.hasPrefix(query) || keywords.contains(" \(query)") { rank = 2 }
                else if name.contains(query) || keywords.contains(query) { rank = 3 }
                else { return nil }
                return (rank, offset, entry)
            // Within a rank the shorter name is the closer match: ":fi" offers fire before film-frames.
            }.sorted { ($0.0, $0.2.name.count, $0.1) < ($1.0, $1.2.name.count, $1.1) }.prefix(6).map(\.2)
        }
        guard !choices.isEmpty else { return nil }
        return EmojiAutocompleteMatch(replacementRange: NSRange(colon..<caret, in: text), choices: Array(choices.prefix(6)))
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
        return (result, NSRange(location: cursor, length: 0))
    }

    private static func normalize(_ value: String) -> String {
        value.lowercased().replacingOccurrences(of: "_", with: " ").replacingOccurrences(of: "-", with: " ")
    }
    private static func isQuery(_ character: Character) -> Bool {
        character.unicodeScalars.count == 1 && character.unicodeScalars.allSatisfy {
            (65...90).contains($0.value) || (97...122).contains($0.value) || (48...57).contains($0.value) || $0 == "_" || $0 == "+" || $0 == "-"
        }
    }
}

/// The composer's active token. Emoji (`:`) and mention (`@`) tokens can never
/// both be active, so they share one popup with a row type per kind.
enum ComposerAutocompleteMatch: Equatable {
    case emoji(EmojiAutocompleteMatch)
    case mention(MentionAutocompleteMatch)

    var count: Int {
        switch self {
        case let .emoji(match): return match.choices.count
        case let .mention(match): return match.choices.count
        }
    }
    var isMention: Bool {
        if case .mention = self { return true }
        return false
    }
    /// Row identities for scrolling the selected row into view.
    var ids: [String] {
        switch self {
        case let .emoji(match): return match.choices.map(\.id)
        case let .mention(match): return match.choices.map(\.id)
        }
    }
}

enum ComposerAutocomplete {
    static func match(text: String, selection: NSRange, markedText: Bool, mentions: MentionSource) -> ComposerAutocompleteMatch? {
        if let mention = MentionAutocomplete.match(text: text, selection: selection, markedText: markedText, source: mentions) {
            return .mention(mention)
        }
        return EmojiAutocomplete.match(text: text, selection: selection, markedText: markedText).map { ComposerAutocompleteMatch.emoji($0) }
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
    var acceptAction: ((Int) -> Void)?
    var insertAction: ((String) -> Void)?
    var restoreFocusAction: (() -> Void)?
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
            }.frame(maxWidth: 260)
                .background(CaperTheme.surface).clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.border))
                .accessibilityIdentifier(match.isMention ? "mention-suggestions" : "emoji-suggestions")
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

extension ComposerSuggestionsView {
    static func mentionLabel(_ candidate: MentionCandidate) -> String {
        candidate.kind == .member
            ? "\(candidate.displayName), @\(candidate.username)\(candidate.invited ? ", already invited" : "")"
            : "@\(candidate.username), \(candidate.displayName)"
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
                Avatar(name: candidate.displayName, size: 28, avatarID: candidate.avatarId)
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
/// Attributes only: never replace the text, register an undo edit, or change glyph metrics.
@MainActor private func highlightDraft(_ storage: NSTextStorage, source: MentionSource) {
    let ranges = MentionAutocomplete.draftHighlights(in: storage.string, source: source)
    #if os(iOS)
    let color = UIColor(CaperTheme.terracotta).withAlphaComponent(0.24)
    #else
    let color = NSColor(CaperTheme.terracotta).withAlphaComponent(0.24)
    #endif
    storage.beginEditing()
    storage.removeAttribute(.backgroundColor, range: NSRange(location: 0, length: storage.length))
    for range in ranges { storage.addAttribute(.backgroundColor, value: color, range: range) }
    #if os(macOS)
    storage.removeAttribute(.toolTip, range: NSRange(location: 0, length: storage.length))
    for emoji in EmojiArtwork.namedRanges(in: storage.string) {
        storage.addAttribute(.toolTip, value: ":\(emoji.name):", range: emoji.range)
    }
    #endif
    storage.endEditing()
}
#endif

#if os(iOS)
final class ComposerTextView: UITextView {
    var autocompleteActive: (() -> Bool)?
    var handleKey: ((String) -> Void)?
    var pasteAttachments: (() -> Bool)?
    private var emojiTooltip: UILabel?
    private var emojiHover: Task<Void, Never>?
    private var inspectedRange: NSRange?
    private var holdPoint: CGPoint?

    func hideEmojiName() {
        emojiHover?.cancel(); emojiHover = nil; inspectedRange = nil
        emojiTooltip?.removeFromSuperview(); emojiTooltip = nil
    }
    override func didMoveToWindow() {
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
    @objc func inspectEmojiHold(_ gesture: UILongPressGestureRecognizer) {
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
    @objc func inspectEmojiHover(_ gesture: UIHoverGestureRecognizer) {
        guard gesture.state != .ended, gesture.state != .cancelled,
              let emoji = emoji(at: gesture.location(in: self)) else { hideEmojiName(); return }
        guard inspectedRange != emoji.range else { return }
        hideEmojiName(); inspectedRange = emoji.range
        emojiHover = Task { [weak self] in
            do { try await Task.sleep(for: .milliseconds(400)) } catch { return }
            self?.showEmojiName(emoji.name, rect: emoji.rect)
        }
    }
    override func paste(_ sender: Any?) {
        if pasteAttachments?() != true { super.paste(sender) }
    }
    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(paste(_:)), isEditable, pasteAttachments != nil,
           UIPasteboard.general.hasImages || UIPasteboard.general.hasURLs { return true }
        return super.canPerformAction(action, withSender: sender)
    }
    /// Once Browse has covered the conversation, UIKit gave the composer its
    /// earlier focus back as the conversation returned, reopening the keyboard
    /// over it. Focus waits for a touch (or VoiceOver) until a moment after.
    private var focusHeld = false
    private var covered = false
    private var focusRelease: Task<Void, Never>?
    override var canBecomeFirstResponder: Bool { !focusHeld && super.canBecomeFirstResponder }
    func setCovered(_ covered: Bool) {
        guard covered != self.covered else { return }
        self.covered = covered
        focusRelease?.cancel(); focusRelease = nil
        if covered { hideEmojiName(); focusHeld = true; return }
        focusRelease = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(1)) } catch { return }
            self?.focusHeld = false
        }
    }
    func touched() { hideEmojiName(); if !covered { focusRelease?.cancel(); focusRelease = nil; focusHeld = false } }
    override func accessibilityActivate() -> Bool { touched(); return super.accessibilityActivate() }
    override var keyCommands: [UIKeyCommand]? {
        guard markedTextRange == nil, autocompleteActive?() == true else { return super.keyCommands }
        return [UIKeyCommand.inputUpArrow, UIKeyCommand.inputDownArrow, "\r", "\t", UIKeyCommand.inputEscape].map {
            let command = UIKeyCommand(input: $0, modifierFlags: [], action: #selector(autocompleteKey(_:)))
            command.wantsPriorityOverSystemBehavior = true
            return command
        }
    }
    @objc private func autocompleteKey(_ command: UIKeyCommand) { if let input = command.input { handleKey?(input) } }
}

/// Sees a finger land on the composer, then steps aside for its own gestures.
private final class TouchDownObserver: UIGestureRecognizer {
    var touched: (() -> Void)?
    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) { touched?(); state = .failed }
}

struct NativeMessageComposer: UIViewRepresentable {
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
    @Environment(\.composerCovered) private var covered
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeUIView(context: Context) -> UITextView {
        let view = ComposerTextView(); view.delegate = context.coordinator
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
        view.backgroundColor = .clear; view.textColor = UIColor(CaperTheme.text); view.font = UIFont(name: "Satoshi-Regular", size: 14)
        view.textContainerInset = UIEdgeInsets(top: 11, left: 7, bottom: 11, right: 7)
        view.isScrollEnabled = true; view.accessibilityIdentifier = "message-composer"
        view.accessibilityLabel = placeholder
        context.coordinator.view = view; context.coordinator.refresh(view); return view
    }
    func updateUIView(_ view: UITextView, context: Context) {
        (view as? ComposerTextView)?.setCovered(covered)
        (view as? ComposerTextView)?.pasteAttachments = pasteAttachments
        view.isEditable = editable
        if !editable { (view as? ComposerTextView)?.hideEmojiName() }
        view.accessibilityIdentifier = accessibilityID
        let mentionsChanged = context.coordinator.parent.mentions != mentions || context.coordinator.parent.highlightSource != highlightSource
        context.coordinator.parent = self
        if view.text != text { (view as? ComposerTextView)?.hideEmojiName(); view.text = text; context.coordinator.dismissed = false; context.coordinator.refresh(view) }
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
        var parent: NativeMessageComposer; weak var view: UITextView?; var dismissed = false
        private var highlighting = false
        init(_ parent: NativeMessageComposer) { self.parent = parent }
        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer) -> Bool { true }
        func scrollViewDidScroll(_ scrollView: UIScrollView) { (scrollView as? ComposerTextView)?.hideEmojiName() }
        func textViewDidBeginEditing(_ textView: UITextView) { refresh(textView) }
        func textViewDidEndEditing(_ textView: UITextView) { (textView as? ComposerTextView)?.hideEmojiName(); parent.controller.update(nil) }
        func textViewDidChange(_ textView: UITextView) { guard !highlighting else { return }; (textView as? ComposerTextView)?.hideEmojiName(); parent.text = textView.text; dismissed = false; refresh(textView) }
        func textViewDidChangeSelection(_ textView: UITextView) { guard !highlighting else { return }; dismissed = false; refresh(textView) }
        func refresh(_ textView: UITextView) {
            guard !highlighting else { return }
            highlight(textView)
            let match = dismissed || !textView.isFirstResponder ? nil : ComposerAutocomplete.match(text: textView.text, selection: textView.selectedRange, markedText: textView.markedTextRange != nil, mentions: parent.mentions)
            parent.controller.update(match)
            parent.controller.acceptAction = { [weak self] index in self?.accept(index) }
            parent.controller.insertAction = { [weak self, selection = textView.selectedRange] emoji in self?.insert(emoji, selection: selection) }
            parent.controller.restoreFocusAction = { [weak self] in
                guard let self, self.parent.editable else { return }
                (self.view as? ComposerTextView)?.touched()
                self.view?.becomeFirstResponder()
            }
        }
        func highlight(_ view: UITextView) {
            guard !highlighting, view.markedTextRange == nil else { return }
            highlighting = true
            highlightDraft(view.textStorage, source: parent.highlightSource ?? parent.mentions)
            view.typingAttributes.removeValue(forKey: .backgroundColor)
            highlighting = false
        }
        func insert(_ emoji: String, selection: NSRange) {
            guard parent.editable, let view,
                  let result = EmojiAutocomplete.inserting(emoji, in: view.text, selection: selection) else { return }
            view.text = result.text; view.selectedRange = result.selection; parent.text = result.text
            highlight(view)
            parent.controller.update(nil)
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
final class ComposerTextView: NSTextView {
    var handleKey: ((NSEvent) -> Bool)?
    var focusChanged: ((Bool) -> Void)?
    /// Only text drags edit the draft. File drags fall through to the
    /// conversation's drop target, which attaches them, instead of typing
    /// their paths (NSTextView's default for file URLs).
    override var acceptableDragTypes: [NSPasteboard.PasteboardType] { [.string] }
    /// A drag carrying text and files together still reaches the text view;
    /// refuse it rather than insert a path.
    private func carriesFiles(_ drag: NSDraggingInfo) -> Bool {
        drag.draggingPasteboard.canReadObject(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])
    }
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation { carriesFiles(sender) ? [] : super.draggingEntered(sender) }
    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation { carriesFiles(sender) ? [] : super.draggingUpdated(sender) }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool { carriesFiles(sender) ? false : super.performDragOperation(sender) }
    var pasteAttachments: (() -> Bool)?
    override func paste(_ sender: Any?) {
        if pasteAttachments?() != true { super.paste(sender) }
    }
    override func keyDown(with event: NSEvent) { if handleKey?(event) != true { super.keyDown(with: event) } }
    override func becomeFirstResponder() -> Bool { let result = super.becomeFirstResponder(); if result { focusChanged?(true) }; return result }
    override func resignFirstResponder() -> Bool { let result = super.resignFirstResponder(); if result { focusChanged?(false) }; return result }
}

struct NativeMessageComposer: NSViewRepresentable {
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
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView(); let view = ComposerTextView()
        scroll.documentView = view; scroll.hasVerticalScroller = true; scroll.drawsBackground = false
        view.delegate = context.coordinator; view.drawsBackground = false; view.textColor = NSColor(CaperTheme.text)
        view.font = NSFont(name: "Satoshi-Regular", size: 14); view.textContainerInset = NSSize(width: 8, height: 10)
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
        view.focusChanged = { [weak coordinator = context.coordinator] focused in
            guard let coordinator, let view = coordinator.view else { return }
            if focused { coordinator.refresh(view) } else { coordinator.parent.controller.update(nil) }
        }
        context.coordinator.refresh(view); return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let mentionsChanged = context.coordinator.parent.mentions != mentions || context.coordinator.parent.highlightSource != highlightSource
        context.coordinator.parent = self
        guard let view = scroll.documentView as? ComposerTextView else { return }
        view.pasteAttachments = pasteAttachments
        view.isEditable = editable
        view.setAccessibilityIdentifier(accessibilityID)
        view.setAccessibilityHelp(submitOnReturn ? "Return sends. Shift-Return adds a new line." : "Use the Send button. Return adds a new line.")
        // The text view reports its own text as its accessibility value; setting
        // it would replace the text on every update.
        if view.string != text { view.string = text; context.coordinator.dismissed = false; context.coordinator.refresh(view) }
        // Members can finish loading while an `@` query is open.
        else if mentionsChanged { context.coordinator.refresh(view) }
    }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSScrollView, context: Context) -> CGSize? {
        guard let width = proposal.width, let view = nsView.documentView as? NSTextView else { return nil }
        // Measure on a scratch text system. Resizing the live text view here
        // invalidates SwiftUI's layout, which measures again, without end.
        var attributes: [NSAttributedString.Key: Any] = [:]
        if let font = view.font { attributes[.font] = font }
        let storage = NSTextStorage(string: view.string, attributes: attributes)
        let layout = NSLayoutManager(); storage.addLayoutManager(layout)
        let container = NSTextContainer(size: NSSize(width: width.isFinite ? max(40, width - 16) : CGFloat.greatestFiniteMagnitude,
                                                     height: CGFloat.greatestFiniteMagnitude))
        layout.addTextContainer(container); layout.ensureLayout(for: container)
        let height = layout.usedRect(for: container).height + 20
        return CGSize(width: width, height: min(174, max(42, height)))
    }
    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: NativeMessageComposer; weak var view: ComposerTextView?; var dismissed = false
        private var highlighting = false
        init(_ parent: NativeMessageComposer) { self.parent = parent }
        // Older macOS SDKs (15.x) don't mark NSTextViewDelegate main-actor, so these delegate
        // calls are nonisolated there. AppKit still delivers them on the main thread.
        func textDidChange(_ notification: Notification) {
            MainActor.assumeIsolated {
                guard !highlighting, let view else { return }
                if parent.text != view.string { parent.text = view.string }
                dismissed = false; refresh(view)
            }
        }
        func textViewDidChangeSelection(_ notification: Notification) { MainActor.assumeIsolated { guard !highlighting, let view else { return }; dismissed = false; refresh(view) } }
        @MainActor func refresh(_ view: ComposerTextView) {
            guard !highlighting else { return }
            highlight(view)
            let match = dismissed || view.window?.firstResponder !== view ? nil : ComposerAutocomplete.match(text: view.string, selection: view.selectedRange(), markedText: view.hasMarkedText(), mentions: parent.mentions)
            parent.controller.update(match); parent.controller.acceptAction = { [weak self] in self?.accept($0) }
            parent.controller.insertAction = { [weak self, selection = view.selectedRange()] in self?.insert($0, selection: selection) }
            parent.controller.restoreFocusAction = { [weak self] in
                guard let self, self.parent.editable, let view = self.view else { return }
                view.window?.makeFirstResponder(view)
            }
        }
        @MainActor func highlight(_ view: ComposerTextView) {
            guard !highlighting, !view.hasMarkedText(), let storage = view.textStorage else { return }
            highlighting = true
            highlightDraft(storage, source: parent.highlightSource ?? parent.mentions)
            view.typingAttributes.removeValue(forKey: .backgroundColor)
            view.typingAttributes.removeValue(forKey: .toolTip)
            highlighting = false
        }
        @MainActor func insert(_ emoji: String, selection: NSRange) {
            guard parent.editable, let view,
                  let result = EmojiAutocomplete.inserting(emoji, in: view.string, selection: selection) else { return }
            view.string = result.text; view.setSelectedRange(result.selection); parent.text = result.text
            highlight(view)
            parent.controller.update(nil)
        }
        @MainActor func handle(_ event: NSEvent) -> Bool {
            guard let view, !view.hasMarkedText() else { return false }
            let key = event.keyCode
            if parent.controller.match != nil {
                if key == 125 || key == 126 { parent.controller.move(down: key == 125); return true }
                if (key == 36 || key == 48) && !event.modifierFlags.contains(.shift) { accept(parent.controller.selected); return true }
                if key == 53 { dismissed = true; parent.controller.update(nil); return true }
            }
            if key == 36 && parent.submitOnReturn && !event.modifierFlags.contains(.shift) { parent.submit(); return true }
            return false
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
