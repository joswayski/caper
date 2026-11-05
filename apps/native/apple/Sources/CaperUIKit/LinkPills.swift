import CaperCore
import UIKit

extension NSAttributedString.Key {
    /// One link occurrence (its icon and text), painted as a single pill per line.
    static let caperLinkPill = NSAttributedString.Key("CaperLinkPill")
}

/// SwiftUI's message links: a small chain-link symbol before each URL and a
/// rounded, faintly glowing tint behind both, drawn by TextKit 2. The symbol is
/// decoration: copying and VoiceOver read the message text without it.
enum LinkPills {
    static func decorate(_ text: NSMutableAttributedString, size: CGFloat) {
        var links: [NSRange] = []
        text.enumerateAttribute(.link, in: NSRange(location: 0, length: text.length)) { value, range, _ in
            if value != nil { links.append(range) }
        }
        let configuration = UIImage.SymbolConfiguration(pointSize: size * 10 / 14)
        guard !links.isEmpty, let symbol = UIImage(systemName: "link", withConfiguration: configuration)?
            .withTintColor(UIKitStyle.link, renderingMode: .alwaysOriginal) else { return }
        // Back to front, so earlier ranges stay valid.
        for (occurrence, range) in links.enumerated().reversed() {
            let attachment = NSTextAttachment(image: symbol)
            let font = text.attribute(.font, at: range.location, effectiveRange: nil) as? UIFont ?? UIKitStyle.font(size, scaled: false)
            attachment.bounds = CGRect(x: 0, y: (font.capHeight - symbol.size.height) / 2, width: symbol.size.width, height: symbol.size.height)
            let icon = NSMutableAttributedString(attachment: attachment)
            icon.addAttributes([.kern: 3, .font: font], range: NSRange(location: 0, length: icon.length))
            text.insert(icon, at: range.location)
            text.addAttribute(.caperLinkPill, value: occurrence, range: NSRange(location: range.location, length: range.length + icon.length))
        }
    }

    /// The text a reader copies: no decorative symbols.
    static func plain(_ text: String) -> String {
        text.replacingOccurrences(of: "\u{FFFC}", with: "")
    }
}

/// Paints link pills behind a paragraph's glyphs, never across a line wrap.
final class LinkPillFragment: NSTextLayoutFragment {
    private static let padding = UIEdgeInsets(top: -1, left: -2, bottom: -1, right: -2)

    override var renderingSurfaceBounds: CGRect {
        super.renderingSurfaceBounds.inset(by: UIEdgeInsets(top: -3, left: -4, bottom: -3, right: -4))
    }

    override func draw(at point: CGPoint, in context: CGContext) {
        context.saveGState()
        for line in textLineFragments {
            let string = line.attributedString
            let lineRange = line.characterRange
            string.enumerateAttribute(.caperLinkPill, in: lineRange) { value, range, _ in
                guard value != nil else { return }
                let start = line.locationForCharacter(at: range.location).x
                let end = line.locationForCharacter(at: NSMaxRange(range)).x
                let bounds = line.typographicBounds
                let rect = CGRect(x: point.x + bounds.minX + start, y: point.y + bounds.minY,
                                  width: end - start, height: bounds.height).inset(by: Self.padding)
                let path = UIBezierPath(roundedRect: rect, cornerRadius: 4).cgPath
                context.saveGState()
                context.setShadow(offset: .zero, blur: 2, color: UIKitStyle.link.withAlphaComponent(0.08).cgColor)
                context.addPath(path)
                context.setFillColor(UIKitStyle.link.withAlphaComponent(0.10).cgColor)
                context.fillPath()
                context.restoreGState()
            }
        }
        context.restoreGState()
        super.draw(at: point, in: context)
    }
}
