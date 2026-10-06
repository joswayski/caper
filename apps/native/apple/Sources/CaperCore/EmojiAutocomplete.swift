import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

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
            }.sorted { ($0.0, $0.1) < ($1.0, $1.1) }.prefix(6).map(\.2)
        }
        guard !choices.isEmpty else { return nil }
        return EmojiAutocompleteMatch(replacementRange: NSRange(colon..<caret, in: text), choices: Array(choices.prefix(6)))
    }

    static func inserting(_ entry: EmojiCatalogEntry, in text: String, match: EmojiAutocompleteMatch,
                          scalarLimit: Int = 4_000) -> (text: String, selection: NSRange)? {
        guard let range = Range(match.replacementRange, in: text) else { return nil }
        let result = text.replacingCharacters(in: range, with: entry.emoji)
        guard result.unicodeScalars.count <= scalarLimit else { return nil }
        let cursor = match.replacementRange.location + (entry.emoji as NSString).length
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
    func update(_ match: ComposerAutocompleteMatch?) {
        if self.match != match { self.match = match; selected = 0 }
    }
    func accept(_ index: Int) { acceptAction?(index) }
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
                            let member = candidate.kind == .member
                            HStack(spacing: 12) {
                                if member {
                                    Avatar(name: candidate.displayName, size: 24, avatarID: candidate.avatarId)
                                } else {
                                    Image(systemName: "at").font(.system(size: 13, weight: .semibold)).foregroundStyle(CaperTheme.muted)
                                        .frame(width: 24, height: 24)
                                }
                                // Member: display name, then a muted @username.
                                // Special: @everyone/@here, then its muted description.
                                HStack(spacing: 6) {
                                    Text(member ? candidate.displayName : "@\(candidate.username)")
                                        .font(CaperTheme.font(13, weight: .medium)).lineLimit(1)
                                    Text(member ? "@\(candidate.username)" : candidate.displayName)
                                        .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                                }
                                Spacer(minLength: 0)
                            }.modifier(SuggestionRow(selected: index == controller.selected))
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
        candidate.kind == .member ? "\(candidate.displayName), @\(candidate.username)" : "@\(candidate.username), \(candidate.displayName)"
    }
}

/// Flat, borderless 44pt rows; the selected row gets the terracotta tint.
private struct SuggestionRow: ViewModifier {
    let selected: Bool
    func body(content: Content) -> some View {
        content.padding(.horizontal, 10).frame(minHeight: 44)
            .background(selected ? CaperTheme.terracotta.opacity(0.32) : Color.clear)
            .contentShape(Rectangle())
    }
}

#if os(iOS)
final class ComposerTextView: UITextView {
    var autocompleteActive: (() -> Bool)?
    var handleKey: ((String) -> Void)?
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

struct NativeMessageComposer: UIViewRepresentable {
    @Binding var text: String
    let placeholder: String
    let controller: ComposerAutocompleteController
    /// Who `@` may suggest in this conversation.
    let mentions: MentionSource
    let submit: () -> Void
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeUIView(context: Context) -> UITextView {
        let view = ComposerTextView(); view.delegate = context.coordinator
        view.autocompleteActive = { [weak controller] in controller?.match != nil }
        view.handleKey = { [weak coordinator = context.coordinator] in coordinator?.handle($0) }
        view.backgroundColor = .clear; view.textColor = UIColor(CaperTheme.text); view.font = UIFont(name: "Satoshi-Regular", size: 14)
        view.textContainerInset = UIEdgeInsets(top: 11, left: 7, bottom: 11, right: 7)
        view.isScrollEnabled = true; view.accessibilityIdentifier = "message-composer"
        view.accessibilityLabel = placeholder
        context.coordinator.view = view; context.coordinator.refresh(view); return view
    }
    func updateUIView(_ view: UITextView, context: Context) {
        let mentionsChanged = context.coordinator.parent.mentions != mentions
        context.coordinator.parent = self
        if view.text != text { view.text = text; context.coordinator.dismissed = false; context.coordinator.refresh(view) }
        // Members can finish loading while an `@` query is open.
        else if mentionsChanged { context.coordinator.refresh(view) }
        view.accessibilityValue = text; view.accessibilityHint = "Use the Send button when composing with the iPhone keyboard."
    }
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        let height = uiView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
        return CGSize(width: width, height: min(174, max(42, height)))
    }
    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: NativeMessageComposer; weak var view: UITextView?; var dismissed = false
        init(_ parent: NativeMessageComposer) { self.parent = parent }
        func textViewDidBeginEditing(_ textView: UITextView) { refresh(textView) }
        func textViewDidEndEditing(_ textView: UITextView) { parent.controller.update(nil) }
        func textViewDidChange(_ textView: UITextView) { parent.text = textView.text; dismissed = false; refresh(textView) }
        func textViewDidChangeSelection(_ textView: UITextView) { dismissed = false; refresh(textView) }
        func refresh(_ textView: UITextView) {
            let match = dismissed || !textView.isFirstResponder ? nil : ComposerAutocomplete.match(text: textView.text, selection: textView.selectedRange, markedText: textView.markedTextRange != nil, mentions: parent.mentions)
            parent.controller.update(match)
            parent.controller.acceptAction = { [weak self] index in self?.accept(index) }
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
            parent.controller.update(nil); view.becomeFirstResponder()
        }
    }
}
#elseif os(macOS)
final class ComposerTextView: NSTextView {
    var handleKey: ((NSEvent) -> Bool)?
    var focusChanged: ((Bool) -> Void)?
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
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView(); let view = ComposerTextView()
        scroll.documentView = view; scroll.hasVerticalScroller = true; scroll.drawsBackground = false
        view.delegate = context.coordinator; view.drawsBackground = false; view.textColor = NSColor(CaperTheme.text)
        view.font = NSFont(name: "Satoshi-Regular", size: 14); view.textContainerInset = NSSize(width: 8, height: 10)
        view.isVerticallyResizable = true; view.isHorizontallyResizable = false; view.autoresizingMask = [.width]
        view.textContainer?.widthTracksTextView = true
        view.isRichText = false
        view.setAccessibilityIdentifier("message-composer"); view.setAccessibilityLabel(placeholder); context.coordinator.view = view
        view.handleKey = { [weak coordinator = context.coordinator] in coordinator?.handle($0) == true }
        view.focusChanged = { [weak coordinator = context.coordinator] focused in
            guard let coordinator, let view = coordinator.view else { return }
            if focused { coordinator.refresh(view) } else { coordinator.parent.controller.update(nil) }
        }
        context.coordinator.refresh(view); return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let mentionsChanged = context.coordinator.parent.mentions != mentions
        context.coordinator.parent = self
        guard let view = scroll.documentView as? ComposerTextView else { return }
        if view.string != text { view.string = text; context.coordinator.dismissed = false; context.coordinator.refresh(view) }
        // Members can finish loading while an `@` query is open.
        else if mentionsChanged { context.coordinator.refresh(view) }
        view.setAccessibilityValue(text); view.setAccessibilityHelp("Return sends. Shift-Return adds a new line.")
    }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSScrollView, context: Context) -> CGSize? {
        guard let width = proposal.width, let view = nsView.documentView as? NSTextView else { return nil }
        view.setFrameSize(NSSize(width: width, height: view.frame.height))
        view.textContainer?.containerSize = NSSize(width: width - 16, height: .greatestFiniteMagnitude)
        view.layoutManager?.ensureLayout(for: view.textContainer!)
        let height = (view.layoutManager?.usedRect(for: view.textContainer!).height ?? 20) + 20
        view.setFrameSize(NSSize(width: width, height: max(42, height)))
        return CGSize(width: width, height: min(174, max(42, height)))
    }
    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: NativeMessageComposer; weak var view: ComposerTextView?; var dismissed = false
        init(_ parent: NativeMessageComposer) { self.parent = parent }
        // Older macOS SDKs (15.x) don't mark NSTextViewDelegate main-actor, so these delegate
        // calls are nonisolated there. AppKit still delivers them on the main thread.
        func textDidChange(_ notification: Notification) { MainActor.assumeIsolated { guard let view else { return }; parent.text = view.string; dismissed = false; refresh(view) } }
        func textViewDidChangeSelection(_ notification: Notification) { MainActor.assumeIsolated { guard let view else { return }; dismissed = false; refresh(view) } }
        @MainActor func refresh(_ view: ComposerTextView) {
            let match = dismissed || view.window?.firstResponder !== view ? nil : ComposerAutocomplete.match(text: view.string, selection: view.selectedRange(), markedText: view.hasMarkedText(), mentions: parent.mentions)
            parent.controller.update(match); parent.controller.acceptAction = { [weak self] in self?.accept($0) }
        }
        @MainActor func handle(_ event: NSEvent) -> Bool {
            guard let view, !view.hasMarkedText() else { return false }
            let key = event.keyCode
            if parent.controller.match != nil {
                if key == 125 || key == 126 { parent.controller.move(down: key == 125); return true }
                if (key == 36 || key == 48) && !event.modifierFlags.contains(.shift) { accept(parent.controller.selected); return true }
                if key == 53 { dismissed = true; parent.controller.update(nil); return true }
            }
            if key == 36 && !event.modifierFlags.contains(.shift) { parent.submit(); return true }
            return false
        }
        @MainActor func accept(_ index: Int) {
            guard let view, let match = parent.controller.match,
                  let result = ComposerAutocomplete.inserting(choice: index, in: view.string, match: match) else { return }
            view.string = result.text; view.setSelectedRange(result.selection); parent.text = result.text
            parent.controller.update(nil); view.window?.makeFirstResponder(view)
        }
    }
}
#endif
