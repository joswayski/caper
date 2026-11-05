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

    public static func signInCode(_ input: String) -> String {
        let allowed = Set("ABCDEFGHJKMNPQRSTWXYZ23456789")
        return String(input.uppercased().filter { allowed.contains($0) }.prefix(6))
    }

    public static func grouped(_ message: ChatMessage, after previous: ChatMessage?) -> Bool {
        MessageGrouping.isGrouped(message, after: previous, inThread: false)
    }

    public static func time(_ value: String) -> String {
        guard let date = ChatTimestamp.date(value) else { return "" }
        return date.formatted(date: .omitted, time: .shortened)
    }

    public static func dayLabel(_ value: String) -> String? { ChatDateDivider.fullDateLabel(value) }

    public static func sameDay(_ first: String, _ second: String) -> Bool {
        ChatDateDivider.sameLocalDay(first, second)
    }

    public static func mentionsViewer(_ message: ChatMessage, viewerID: String?) -> Bool {
        MentionAutocomplete.mentionsCurrentUser(message, currentUserID: viewerID)
    }

    public static func resend(secondsSince sentAt: Date?, codesSent: Int, now: Date) -> (label: String, enabled: Bool, allowed: Bool) {
        let seconds = CodeResend.secondsRemaining(sentAt: sentAt, now: now)
        let allowed = codesSent <= CodeResend.maximumResends
        return (CodeResend.label(secondsRemaining: seconds), seconds == 0 && allowed, allowed)
    }

    public static func messageText(_ text: String, size: CGFloat = 14, mentions: [MessageMention] = []) -> NSAttributedString {
        let result = NSMutableAttributedString(string: "")
        let parsed = MessageFormatting.runs(text)
        // Match renderedText: token boundaries belong to the complete visible
        // text, not to individual bold/italic runs (for example **@ali**ce).
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
                let weight = run.bold ? "Bold" : segment.highlighted ? "Medium" : "Regular"
                #if os(macOS)
                let base = NSFont(name: "Satoshi-\(weight)", size: size) ?? NSFont.systemFont(ofSize: size)
                let font = run.italic ? NSFontManager.shared.convert(base, toHaveTrait: .italicFontMask) : base
                let color = NSColor(CaperTheme.text)
                let highlight = NSColor(CaperTheme.terracotta).withAlphaComponent(0.24)
                #else
                let base = UIFont(name: "Satoshi-\(weight)", size: size) ?? UIFont.systemFont(ofSize: size)
                let descriptor = run.italic ? base.fontDescriptor.withSymbolicTraits(.traitItalic) ?? base.fontDescriptor : base.fontDescriptor
                let font = UIFont(descriptor: descriptor, size: size)
                let color = UIColor(CaperTheme.text)
                let highlight = UIColor(CaperTheme.terracotta).withAlphaComponent(0.24)
                #endif
                var attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color]
                if segment.highlighted {
                    attributes[.backgroundColor] = highlight
                    attributes[mentionAttribute] = segment.user?.id ?? ""
                    result.append(NSAttributedString(string: text, attributes: attributes))
                } else {
                    for link in MessageLinks.segments(in: text) {
                        var linked = attributes
                        if let href = link.href, let url = URL(string: href) { linked[.link] = url }
                        result.append(NSAttributedString(string: link.text, attributes: linked))
                    }
                }
            }
            runStart = runEnd
        }
        return result
    }

    /// Core Text measures only a realized row, using the same attributed input
    /// as its selectable native text view. No whole-history framesetter cache.
    public static func textHeight(_ text: NSAttributedString, width: CGFloat) -> CGFloat {
        let framesetter = CTFramesetterCreateWithAttributedString(text)
        return ceil(CTFramesetterSuggestFrameSizeWithConstraints(framesetter, CFRange(location: 0, length: text.length),
            nil, CGSize(width: max(1, width), height: .greatestFiniteMagnitude), nil).height)
    }
}
