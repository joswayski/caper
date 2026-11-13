import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

#if os(iOS) || os(macOS)
extension NSAttributedString.Key {
    public static let draftLink = NSAttributedString.Key("chat.caper.draftLink")
    /// Spoiler text's rounded fill, a platform colour: #34383B in drafts and
    /// over a hidden spoiler, faint white once revealed.
    public static let spoilerFill = NSAttributedString.Key("chat.caper.spoilerFill")
    /// Quoted text: its lines get a 3pt #34383B bar on the left.
    public static let quoteBar = NSAttributedString.Key("chat.caper.quoteBar")
}

/// TextKit 1 decoration, not an attachment or a replacement string. Reserve
/// space after the first ASCII URL character with kerning, then draw that
/// character after the icon instead. All UTF-16 offsets, native editing,
/// pasteboard text, accessibility and undo remain owned by the text view.
/// The icon belongs to the first character's selection/hit area, not a new
/// caret stop. A wrapped URL gets one leading icon and a pill on each line.
/// Spoilers get the same kind of rounded fill, and quoted lines a 3pt bar;
/// the AppKit timeline draws sent messages' spoilers and quotes with it too.
public final class DraftLinkLayoutManager: NSLayoutManager {
    static let iconAdvance: CGFloat = 14

    public override func drawBackground(forGlyphRange glyphsToShow: NSRange, at origin: CGPoint) {
        guard let storage = textStorage else {
            super.drawBackground(forGlyphRange: glyphsToShow, at: origin)
            return
        }
        #if os(iOS)
        let context = UIGraphicsGetCurrentContext()
        let color = UIColor(CaperTheme.link)
        let border = UIColor(CaperTheme.border)
        #else
        let context = NSGraphicsContext.current?.cgContext
        let color = NSColor(CaperTheme.link)
        let border = NSColor(CaperTheme.border)
        #endif
        if let context {
            context.saveGState()
            context.setFillColor(border.cgColor)
            storage.enumerateAttribute(.quoteBar, in: NSRange(location: 0, length: storage.length)) { value, characters, _ in
                guard value != nil else { return }
                let glyphs = NSIntersectionRange(self.glyphRange(forCharacterRange: characters, actualCharacterRange: nil), glyphsToShow)
                guard glyphs.length > 0 else { return }
                // Whole line fragments, so consecutive quoted lines join into one bar.
                self.enumerateLineFragments(forGlyphRange: glyphs) { line, _, container, _, _ in
                    context.fill(CGRect(x: origin.x + line.minX + container.lineFragmentPadding, y: origin.y + line.minY,
                                        width: 3, height: line.height))
                }
            }
            self.fillRoundedRuns(.spoilerFill, in: storage, glyphsToShow: glyphsToShow, at: origin, context: context) { value in
                #if os(iOS)
                return (value as? UIColor)?.cgColor
                #else
                return (value as? NSColor)?.cgColor
                #endif
            }
            let linkFill = color.withAlphaComponent(0.10).cgColor
            self.fillRoundedRuns(.draftLink, in: storage, glyphsToShow: glyphsToShow, at: origin, context: context) { _ in linkFill }
            context.restoreGState()
        }
        // Native selection/marked-text and mention backgrounds stay on top.
        super.drawBackground(forGlyphRange: glyphsToShow, at: origin)
    }

    /// A rounded fill behind each line's part of every run carrying `key`,
    /// in the colour `fill` gives for the attribute's value.
    private func fillRoundedRuns(_ key: NSAttributedString.Key, in storage: NSTextStorage, glyphsToShow: NSRange,
                                 at origin: CGPoint, context: CGContext, fill: (Any) -> CGColor?) {
        storage.enumerateAttribute(key, in: NSRange(location: 0, length: storage.length)) { value, characters, _ in
            guard let value, let color = fill(value) else { return }
            context.setFillColor(color)
            let glyphs = self.glyphRange(forCharacterRange: characters, actualCharacterRange: nil)
            let visible = NSIntersectionRange(glyphs, glyphsToShow)
            guard visible.length > 0 else { return }
            self.enumerateLineFragments(forGlyphRange: visible) { line, used, _, lineGlyphs, _ in
                let fragment = NSIntersectionRange(glyphs, lineGlyphs)
                // Both ink bounds and enclosing rectangles can retain the
                // previous line's pen x after a hidden delimiter at a wrap.
                // Use the actual glyph positions, skipping null delimiters
                // at either edge, and the used line end for a wrapped tail.
                guard let first = (fragment.location..<NSMaxRange(fragment)).first(where: {
                    !self.propertyForGlyph(at: $0).contains(.null)
                }) else { return }
                let left = line.minX + self.location(forGlyphAt: first).x
                let next = (NSMaxRange(fragment)..<NSMaxRange(lineGlyphs)).first {
                    !self.propertyForGlyph(at: $0).contains(.null)
                }
                let right = next.map { line.minX + self.location(forGlyphAt: $0).x } ?? used.maxX
                guard right > left else { return }
                let rect = CGRect(x: origin.x + left, y: origin.y + used.minY,
                                  width: right - left, height: used.height).insetBy(dx: -2, dy: -1)
                context.addPath(CGPath(roundedRect: rect, cornerWidth: 4, cornerHeight: 4, transform: nil))
                context.fillPath()
            }
        }
    }

    public override func drawGlyphs(forGlyphRange glyphsToShow: NSRange, at origin: CGPoint) {
        guard let storage = textStorage else {
            super.drawGlyphs(forGlyphRange: glyphsToShow, at: origin)
            return
        }
        var starts: [Int] = []
        storage.enumerateAttribute(.draftLink, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            // Marked-text input can split a decorated run while highlighting
            // is deliberately paused. Only the character with reserved width
            // owns an icon; never shift a suffix's first glyph over its neighbour.
            guard value != nil,
                  (storage.attribute(.kern, at: range.location, effectiveRange: nil) as? NSNumber)?.doubleValue == Double(Self.iconAdvance) else { return }
            let glyph = self.glyphIndexForCharacter(at: range.location)
            if NSLocationInRange(glyph, glyphsToShow) { starts.append(glyph) }
        }
        var cursor = glyphsToShow.location
        for glyph in starts {
            if glyph > cursor {
                super.drawGlyphs(forGlyphRange: NSRange(location: cursor, length: glyph - cursor), at: origin)
            }
            super.drawGlyphs(forGlyphRange: NSRange(location: glyph, length: 1),
                             at: CGPoint(x: origin.x + Self.iconAdvance, y: origin.y))
            // The glyph's position is relative to its line fragment, including
            // after zero-advance Markdown glyphs. Ink bounds are not an anchor.
            let line = lineFragmentRect(forGlyphAt: glyph, effectiveRange: nil)
            let used = lineFragmentUsedRect(forGlyphAt: glyph, effectiveRange: nil)
            let position = location(forGlyphAt: glyph)
            drawChain(in: CGRect(x: origin.x + line.minX + position.x + 1,
                                 y: origin.y + used.midY - 5, width: 10, height: 10))
            cursor = glyph + 1
        }
        if cursor < NSMaxRange(glyphsToShow) {
            super.drawGlyphs(forGlyphRange: NSRange(location: cursor, length: NSMaxRange(glyphsToShow) - cursor), at: origin)
        }
        // A hidden spoiler's fill goes over its glyphs too: colour emoji
        // ignore the clear text colour.
        #if os(iOS)
        let context = UIGraphicsGetCurrentContext()
        let cover = UIColor(CaperTheme.border).cgColor
        #else
        let context = NSGraphicsContext.current?.cgContext
        let cover = NSColor(CaperTheme.border).cgColor
        #endif
        if let context {
            context.saveGState()
            fillRoundedRuns(NativePresentation.hiddenSpoilerAttribute, in: storage, glyphsToShow: glyphsToShow, at: origin, context: context) { _ in cover }
            context.restoreGState()
        }
    }

    private func drawChain(in rect: CGRect) {
        #if os(iOS)
        UIImage(systemName: "link")?.withTintColor(UIColor(CaperTheme.link), renderingMode: .alwaysOriginal).draw(in: rect)
        #else
        let image = NSImage(systemSymbolName: "link", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(paletteColors: [NSColor(CaperTheme.link)]))
        image?.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
        #endif
    }
}
#endif
