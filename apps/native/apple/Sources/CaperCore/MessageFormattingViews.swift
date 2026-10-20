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

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if controller.formattingSelection {
                HStack(spacing: 4) {
                    Button { controller.formatAction?("**") } label: { Text("B").bold() }
                        .accessibilityLabel("Bold").help("Bold (⌘B)")
                    Button { controller.formatAction?("_") } label: { Text("I").italic() }
                        .accessibilityLabel("Italic").help("Italic (⌘I)")
                }
                .buttonStyle(.bordered).font(CaperTheme.font(12))
                .foregroundStyle(CaperTheme.text).padding(4).disabled(!editable)
            }
            NativeMessageTextEditor(text: $text, placeholder: placeholder, controller: controller,
                                    mentions: mentions, submit: submit, submitOnReturn: submitOnReturn,
                                    editable: editable, accessibilityID: accessibilityID,
                                    pasteAttachments: pasteAttachments, highlightSource: highlightSource)
        }
    }
}

extension MessageFormatting {
    static func renderedText(_ source: String, mentions: [MessageMention] = []) -> Text {
        var result = Text("")
        let parsed = runs(source)
        let segments = MentionAutocomplete.segments(in: parsed.map(\.text).joined(), mentions: mentions)
        var runStart = 0
        for run in parsed {
            let runEnd = runStart + run.text.utf16.count
            var segmentStart = 0
            for segment in segments {
                let segmentEnd = segmentStart + segment.text.utf16.count
                let start = max(runStart, segmentStart), end = min(runEnd, segmentEnd)
                defer { segmentStart = segmentEnd }
                guard start < end else { continue }
                let text = (segment.text as NSString).substring(with: NSRange(location: start - segmentStart, length: end - start))
                var part: Text
                if segment.highlighted {
                    var span = AttributedString(text)
                    span.foregroundColor = CaperTheme.text
                    span.backgroundColor = CaperTheme.terracotta.opacity(0.24)
                    if let user = segment.user { span.link = MentionCard.url(for: user) }
                    part = Text(span).fontWeight(.medium)
                } else {
                    part = MessageLinks.renderedText(text) ?? Text(text)
                }
                if run.bold { part = part.bold() }
                if run.italic { part = part.italic() }
                result = result + part
            }
            runStart = runEnd
        }
        return result
    }
}
