import SwiftUI

struct NativeMessageComposer: View {
    @Binding var text: String
    let placeholder: String
    @ObservedObject var controller: ComposerAutocompleteController
    let mentions: MentionSource
    let submit: () -> Void
    var submitOnReturn = true
    var editable = true
    var accessibilityID = "message-composer"
    var pasteAttachments: (() -> Bool)? = nil
    var highlightSource: MentionSource? = nil
    /// The conversation (and thread) the draft belongs to: another one ends the
    /// style typing carries, even with an identical draft.
    var conversation = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if controller.formattingSelection {
                HStack(spacing: 0) {
                    ForEach(MessageFormattingAction.all, id: \.title) { item in
                        if item.style == .quote {
                            Color.clear.frame(width: 12, height: 1) // A gap before Quote, as Discord
                                .accessibilityHidden(true)
                        }
                        let pressed = controller.formattingStyles.contains(item.style)
                        Button { controller.formatAction?(item.style) } label: { Image(systemName: item.symbol) }
                            .buttonStyle(FormattingButtonStyle(pressed: pressed))
                            .accessibilityLabel(item.title)
                            .accessibilityAddTraits(pressed ? .isSelected : [])
                            .help(item.key.map { "\(item.title) (⌘\($0.uppercased()))" } ?? item.title)
                    }
                }
                .font(.system(size: 15, weight: .medium))
                .background(CaperTheme.surface).clipShape(RoundedRectangle(cornerRadius: 8))
                .disabled(!editable)
            }
            NativeMessageTextEditor(text: $text, placeholder: placeholder, controller: controller,
                                    mentions: mentions, submit: submit, submitOnReturn: submitOnReturn,
                                    editable: editable, accessibilityID: accessibilityID,
                                    pasteAttachments: pasteAttachments, highlightSource: highlightSource,
                                    conversation: conversation)
        }
    }
}

private struct FormattingButtonStyle: ButtonStyle {
    /// The selection already has this style.
    let pressed: Bool
    @Environment(\.isEnabled) private var enabled
    @Environment(\.isFocused) private var focused
    @State private var hovered = false

    init(pressed: Bool = false) { self.pressed = pressed }

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(CaperTheme.text)
            #if os(iOS)
            .frame(width: 44, height: 44)
            #else
            .frame(width: 36, height: 36)
            #endif
            .contentShape(Rectangle())
            .background(configuration.isPressed || pressed ? CaperTheme.border : (hovered || focused) && enabled ? CaperTheme.raised : Color.clear)
            .opacity(enabled ? 1 : 0.4)
            .focusEffectDisabled()
            .onHover { hovered = $0 }
            .modifier(ControlPointer())
    }
}

/// Message text as web renders it: the formatting subset, mentions and
/// links, quoted lines as blocks with a bar, and spoilers hidden until
/// activated. SwiftUI `Text` cannot hit-test one of its runs, so activating a
/// message's text reveals all of its spoilers together. Revealing is local to
/// this view.
struct FormattedMessageText: View {
    let source: String
    let mentions: [MessageMention]
    @State private var revealed = false

    init(source: String, mentions: [MessageMention] = []) {
        self.source = source
        self.mentions = mentions
    }

    var body: some View {
        let parsed = MessageFormatting.runs(source)
        let segments = MentionAutocomplete.segments(in: parsed.map(\.text).joined(), mentions: mentions)
        let hidden = !revealed && parsed.contains { $0.spoiler }
        let blocks = MessageFormatting.stackedBlocks(parsed)
        Group {
            if blocks.count == 1, let block = blocks.first, !block.quote {
                MessageFormatting.renderedText(parsed, segments: segments, in: block.range, revealed: !hidden)
            } else {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                        let text = block.range.isEmpty ? Text(" ")
                            : MessageFormatting.renderedText(parsed, segments: segments, in: block.range, revealed: !hidden)
                        if block.quote {
                            text.padding(.leading, 15).frame(maxWidth: .infinity, alignment: .leading)
                                .overlay(alignment: .leading) { Rectangle().fill(CaperTheme.border).frame(width: 3) }
                        } else {
                            text
                        }
                    }
                }
            }
        }
        .modifier(SpoilerReveal(hidden: hidden, label: MessageFormatting.spokenText(parsed)) { revealed = true })
    }
}

/// While spoilers are hidden, a tap or click reveals them; VoiceOver reads
/// "Spoiler" in their place and offers "Spoiler, reveal".
private struct SpoilerReveal: ViewModifier {
    let hidden: Bool
    let label: String
    let reveal: () -> Void

    @ViewBuilder func body(content: Content) -> some View {
        if hidden {
            content
                .contentShape(Rectangle())
                .onTapGesture { if !SwipeTapGuard.swiping { reveal() } }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(Text(label))
                .accessibilityAction { reveal() }
                .accessibilityAction(named: Text("Spoiler, reveal")) { reveal() }
                .modifier(ControlPointer())
        } else {
            content
        }
    }
}

extension MessageFormatting {
    /// Quote blocks as stacked views show them. Each block starts its own
    /// line, so the newline ending the line before a block, or the block's own
    /// line, is dropped, as web's full-width quote blocks absorb it. An empty
    /// range is a blank line.
    static func stackedBlocks(_ runs: [FormattedMessageRun]) -> [(range: Range<Int>, quote: Bool)] {
        let units = Array(runs.map(\.text).joined().utf16)
        let blocks = quoteBlocks(runs)
        var result: [(range: Range<Int>, quote: Bool)] = []
        for (index, block) in blocks.enumerated() {
            var start = block.range.lowerBound, end = block.range.upperBound
            if index > 0, start < end, units[start] == 10 { start += 1 }
            var trailing = false
            if index < blocks.count - 1, start < end, units[end - 1] == 10 { end -= 1; trailing = true }
            if start < end || trailing { result.append((start..<end, block.quote)) }
        }
        return result
    }

    /// The text VoiceOver reads while spoilers are hidden: "Spoiler" in place of each.
    static func spokenText(_ runs: [FormattedMessageRun]) -> String {
        var spoken = ""
        var inSpoiler = false
        for run in runs {
            if run.spoiler { if !inSpoiler { spoken += "Spoiler" } }
            else { spoken += run.text }
            inSpoiler = run.spoiler
        }
        return spoken
    }

    /// The displayed range `block` of parsed runs as one `Text`. A hidden
    /// spoiler keeps its glyphs' width, invisible on the #34383B fill, and
    /// has no link or person pill to activate.
    static func renderedText(_ parsed: [FormattedMessageRun], segments: [MentionSegment], in block: Range<Int>, revealed: Bool) -> Text {
        var result = Text("")
        var runStart = 0
        for run in parsed {
            let runEnd = runStart + run.text.utf16.count
            defer { runStart = runEnd }
            var segmentStart = 0
            for segment in segments {
                let segmentEnd = segmentStart + segment.text.utf16.count
                let start = max(runStart, segmentStart, block.lowerBound), end = min(runEnd, segmentEnd, block.upperBound)
                defer { segmentStart = segmentEnd }
                guard start < end else { continue }
                let text = (segment.text as NSString).substring(with: NSRange(location: start - segmentStart, length: end - start))
                var part: Text
                if run.spoiler && !revealed {
                    // Colour emoji ignore the clear text colour: an em space keeps their place.
                    var span = AttributedString(String(text.map { character -> Character in
                        character.unicodeScalars.contains { $0.properties.isEmojiPresentation }
                            || (character.unicodeScalars.count > 1 && character.unicodeScalars.contains { $0.properties.isEmoji })
                            ? "\u{2003}" : character
                    }))
                    span.foregroundColor = Color.clear
                    span.backgroundColor = CaperTheme.border
                    part = Text(span)
                    if segment.highlighted { part = part.fontWeight(.medium) }
                } else if segment.highlighted {
                    var span = AttributedString(text)
                    span.foregroundColor = CaperTheme.text
                    span.backgroundColor = CaperTheme.terracotta.opacity(0.24)
                    if let user = segment.user { span.link = MentionCard.url(for: user) }
                    part = Text(span).fontWeight(.medium)
                } else if let linked = MessageLinks.renderedText(text) {
                    part = linked
                } else if run.spoiler {
                    var span = AttributedString(text)
                    span.backgroundColor = Color.white.opacity(0.06)
                    part = Text(span)
                } else {
                    part = Text(text)
                }
                if run.bold { part = part.bold() }
                if run.italic { part = part.italic() }
                if run.underline { part = part.underline() }
                if run.strikethrough { part = part.strikethrough() }
                result = result + part
            }
        }
        return result
    }
}
