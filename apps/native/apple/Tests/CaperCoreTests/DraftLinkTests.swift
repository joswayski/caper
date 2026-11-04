import XCTest
#if os(macOS)
import AppKit
#elseif os(iOS)
import UIKit
#endif
@testable import CaperCore

#if os(iOS) || os(macOS)
final class DraftLinkTests: XCTestCase {
    @MainActor func testDecorationKeepsPlainTextAndUTF16Ranges() {
        let text = "👩‍💻 **https://example.com/é?q=1**, then www.example.com."
        let storage = NSTextStorage(string: text)
        highlightDraft(storage, source: .empty)
        XCTAssertEqual(storage.string, text)
        var links: [NSRange] = []
        var kerns: [NSRange] = []
        let full = NSRange(location: 0, length: storage.length)
        storage.enumerateAttribute(.draftLink, in: full) { value, range, _ in
            if value != nil { links.append(range) }
        }
        storage.enumerateAttribute(.kern, in: full) { value, range, _ in
            if value != nil { kerns.append(range) }
        }
        XCTAssertEqual(links, [(text as NSString).range(of: "https://example.com/é?q=1"),
                               (text as NSString).range(of: "www.example.com")])
        XCTAssertEqual(kerns, links.map { NSRange(location: $0.location, length: 1) })
        for index in 0..<storage.length {
            XCTAssertNil(storage.attribute(.attachment, at: index, effectiveRange: nil), "no object replacement character or attachment")
            XCTAssertNil(storage.attribute(.link, at: index, effectiveRange: nil), "draft links stay editable, not tappable")
        }
        highlightDraft(storage, source: .empty)
        XCTAssertEqual(storage.string, text, "restyling must be idempotent and not add an icon to the draft")
    }

    @MainActor func testEditingAwayALinkRemovesItsDecoration() {
        let storage = NSTextStorage(string: "https://example.com")
        highlightDraft(storage, source: .empty)
        storage.replaceCharacters(in: NSRange(location: 0, length: 8), with: "")
        highlightDraft(storage, source: .empty)
        XCTAssertEqual(storage.string, "example.com")
        for index in 0..<storage.length {
            XCTAssertNil(storage.attribute(.draftLink, at: index, effectiveRange: nil))
            XCTAssertNil(storage.attribute(.kern, at: index, effectiveRange: nil))
        }
    }

    @MainActor func testMentionsAndFormattingKeepTheirOwnText() {
        let text = "@maya _www.example.com_ @feedback"
        let storage = NSTextStorage(string: text)
        let source = MentionSource(members: [.member(id: "maya", username: "maya", displayName: "Maya")], specials: false)
        highlightDraft(storage, source: source)
        XCTAssertEqual(storage.string, text)
        XCTAssertNotNil(storage.attribute(.backgroundColor, at: 0, effectiveRange: nil))
        XCTAssertNil(storage.attribute(.draftLink, at: 0, effectiveRange: nil))
        XCTAssertNil(storage.attribute(.kern, at: 0, effectiveRange: nil))
        let url = (text as NSString).range(of: "www.example.com")
        XCTAssertNotNil(storage.attribute(.draftLink, at: url.location, effectiveRange: nil))
        XCTAssertNil(storage.attribute(.backgroundColor, at: url.location, effectiveRange: nil), "rounded backgrounds are painted, not square attributed spans")
        XCTAssertNil(storage.attribute(.draftLink, at: NSMaxRange(url), effectiveRange: nil), "formatting delimiters are not part of the URL")
    }

    @MainActor func testLayoutReservesIconWidthWithoutAddingACaretStop() {
        let text = "https://example.com"
        let storage = NSTextStorage(string: text)
        let layout = DraftLinkLayoutManager()
        let glyphs = ComposerGlyphs()
        defer { withExtendedLifetime(glyphs) {} }
        layout.delegate = glyphs
        let container = NSTextContainer(size: CGSize(width: 600, height: 300))
        storage.addLayoutManager(layout); layout.addTextContainer(container)
        // Use the same composer font on both sides: formatting now owns fonts,
        // so measuring an unrelated system font first would compare two changes.
        highlightDraft(storage, source: .empty)
        storage.removeAttribute(.kern, range: NSRange(location: 0, length: storage.length))
        layout.ensureLayout(for: container)
        let plainWidth = layout.usedRect(for: container).width
        highlightDraft(storage, source: .empty)
        layout.ensureLayout(for: container)
        XCTAssertEqual(layout.usedRect(for: container).width - plainWidth, DraftLinkLayoutManager.iconAdvance, accuracy: 0.5)
        XCTAssertEqual(layout.numberOfGlyphs, text.utf16.count)
        XCTAssertEqual(layout.characterRange(forGlyphRange: NSRange(location: 0, length: layout.numberOfGlyphs), actualGlyphRange: nil),
                       NSRange(location: 0, length: text.utf16.count))
        XCTAssertEqual(storage.string, text)
    }

    @MainActor func testRestylingKeepsNativeSelectionAcrossWrappedAndRepeatedLinks() {
        let text = "👩‍💻 https://example.com/long/path and https://example.com/long/path"
        let storage = NSTextStorage(string: text)
        let layout = DraftLinkLayoutManager()
        let glyphs = ComposerGlyphs()
        defer { withExtendedLifetime(glyphs) {} }
        layout.delegate = glyphs
        let container = NSTextContainer(size: CGSize(width: 120, height: 600))
        storage.addLayoutManager(layout); layout.addTextContainer(container)
        let view = ComposerTextView(frame: CGRect(x: 0, y: 0, width: 120, height: 600), textContainer: container)
        let selection = (text as NSString).range(of: "example.com/long")
        #if os(macOS)
        view.font = NSFont.systemFont(ofSize: 14)
        view.setSelectedRange(selection)
        #else
        view.font = UIFont.systemFont(ofSize: 14)
        view.selectedRange = selection
        #endif
        highlightDraft(storage, source: .empty)
        layout.ensureLayout(for: container)
        #if os(macOS)
        XCTAssertEqual(view.selectedRange(), selection)
        XCTAssertEqual(view.string, text)
        #else
        XCTAssertEqual(view.selectedRange, selection)
        XCTAssertEqual(view.text, text)
        #endif
        var occurrences = 0
        storage.enumerateAttribute(.draftLink, in: NSRange(location: 0, length: storage.length)) { value, _, _ in
            if value != nil { occurrences += 1 }
        }
        XCTAssertEqual(occurrences, 2, "repeated destinations have independent decorations")
        XCTAssertEqual(storage.attributedSubstring(from: selection).string, "example.com/long", "selection copies only the source text")
    }

    @MainActor func testFormattedLinksComposeWithHiddenGlyphsAndPlainClipboard() {
        let text = "**_https://example.com/a_b_** 👩‍💻 **www.example.com**"
        let visible = "https://example.com/a_b 👩‍💻 www.example.com"
        let storage = NSTextStorage(string: text)
        let layout = DraftLinkLayoutManager()
        let glyphs = ComposerGlyphs()
        defer { withExtendedLifetime(glyphs) {} }
        layout.delegate = glyphs
        let container = NSTextContainer(size: CGSize(width: 160, height: 600))
        storage.addLayoutManager(layout); layout.addTextContainer(container)
        let view = ComposerTextView(frame: CGRect(x: 0, y: 0, width: 160, height: 600), textContainer: container)
        highlightDraft(storage, source: .empty)
        layout.ensureLayout(for: container)
        XCTAssertEqual(storage.string, text, "Markdown and URLs keep their source UTF-16 coordinates")
        XCTAssertEqual(MessageFormatting.projection(text).text, visible)
        let visibleIndices = Set(MessageFormatting.projection(text).sourceIndices)
        for index in 0..<storage.length where !visibleIndices.contains(index) {
            XCTAssertEqual(storage.attribute(.composerDelimiter, at: index, effectiveRange: nil) as? Bool, true)
            XCTAssertNil(storage.attribute(.draftLink, at: index, effectiveRange: nil))
            XCTAssertNil(storage.attribute(.kern, at: index, effectiveRange: nil))
            XCTAssertTrue(layout.propertyForGlyph(at: layout.glyphIndexForCharacter(at: index)).contains(.null))
        }
        for url in ["https://example.com/a_b", "www.example.com"] {
            let range = (text as NSString).range(of: url)
            XCTAssertEqual(storage.attribute(.draftLink, at: range.location, effectiveRange: nil) as? Int, range.location)
            XCTAssertEqual((storage.attribute(.kern, at: range.location, effectiveRange: nil) as? NSNumber)?.doubleValue,
                           Double(DraftLinkLayoutManager.iconAdvance))
            XCTAssertFalse(layout.propertyForGlyph(at: layout.glyphIndexForCharacter(at: range.location)).contains(.null))
            XCTAssertEqual(MessageFormatting.copiedText(text, selection: range), url)
        }
        XCTAssertEqual((storage.attribute(.obliqueness, at: 3, effectiveRange: nil) as? NSNumber)?.doubleValue, 0.2)
        let emoji = (text as NSString).range(of: "👩‍💻")
        XCTAssertEqual(MessageFormatting.deletionRange(text, selection: NSRange(location: NSMaxRange(emoji), length: 0), backward: true), emoji)
        let selection = NSRange(location: 0, length: storage.length)
        #if os(macOS)
        view.setSelectedRange(selection)
        view.copy(nil)
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), visible)
        #else
        view.selectedRange = selection
        // Hostless iOS tests can hang on the system pasteboard. Check the
        // native source selection and the exact copy projection used by the
        // editor instead; actual iOS clipboard delivery needs a hosted UI test.
        XCTAssertEqual(view.selectedRange, selection)
        XCTAssertEqual(view.text, text)
        XCTAssertEqual(MessageFormatting.copiedText(view.text, selection: view.selectedRange), visible)
        #endif
        XCTAssertEqual(storage.string, text, "copy strips hidden delimiters but never changes the wire draft")
    }

    /// Exercise the actual native drawing APIs, not just the attributes. The
    /// second link used to paint its icon and a zero-width pill at the previous
    /// line's trailing pen position while its italic text drew at the left.
    @MainActor func testWrappedBoldThenItalicLinksPaintAtTheirOwnGlyphPositions() throws {
        let text = "Review **https://example.com/a-long-link-to-check-wrapping?mode=Case** and _www.example.org_."
        let storage = NSTextStorage(string: text)
        let layout = DraftLinkLayoutManager()
        let glyphs = ComposerGlyphs()
        defer { withExtendedLifetime(glyphs) {} }
        layout.delegate = glyphs
        let container = NSTextContainer(size: CGSize(width: 300, height: 160))
        storage.addLayoutManager(layout); layout.addTextContainer(container)
        let view = ComposerTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 160), textContainer: container)
        highlightDraft(storage, source: .empty)
        layout.ensureLayout(for: container)
        #if os(macOS)
        view.backgroundColor = .black
        let origin = view.textContainerOrigin
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: bitmap)
        let image = try XCTUnwrap(bitmap.cgImage)
        #else
        view.backgroundColor = .black
        view.layoutIfNeeded()
        let origin = CGPoint(x: view.textContainerInset.left, y: view.textContainerInset.top)
        let rendered = UIGraphicsImageRenderer(size: view.bounds.size).image { view.layer.render(in: $0.cgContext) }
        let image = try XCTUnwrap(rendered.cgImage)
        #endif
        let pixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(image))
        let scale = CGFloat(pixels.width) / view.bounds.width
        func blueFraction(in rect: CGRect, minimumContrast: Int) -> Double {
            let area = CGRect(x: rect.minX * scale, y: rect.minY * scale,
                              width: rect.width * scale, height: rect.height * scale).integral
                .intersection(CGRect(x: 0, y: 0, width: pixels.width, height: pixels.height))
            guard !area.isNull, area.width > 0, area.height > 0 else { return 0 }
            var blue = 0
            for y in Int(area.minY)..<Int(area.maxY) {
                for x in Int(area.minX)..<Int(area.maxX) {
                    let index = (y * pixels.width + x) * 4
                    if Int(pixels.straight[index + 2]) - Int(pixels.straight[index]) >= minimumContrast { blue += 1 }
                }
            }
            return Double(blue) / Double(area.width * area.height)
        }

        let urls = MessageLinks.segments(in: text).filter { $0.href != nil }.map(\.text)
        XCTAssertEqual(urls.count, 2)
        var starts: [CGPoint] = []
        for (index, url) in urls.enumerated() {
            let range = layout.glyphRange(forCharacterRange: (text as NSString).range(of: url), actualCharacterRange: nil)
            let firstLine = layout.lineFragmentRect(forGlyphAt: range.location, effectiveRange: nil)
            let firstUsed = layout.lineFragmentUsedRect(forGlyphAt: range.location, effectiveRange: nil)
            let position = layout.location(forGlyphAt: range.location)
            let start = CGPoint(x: origin.x + firstLine.minX + position.x, y: origin.y + firstUsed.minY)
            starts.append(start)
            let icon = CGRect(x: start.x + 1, y: origin.y + firstUsed.midY - 5, width: 10, height: 10)
            XCTAssertGreaterThan(blueFraction(in: icon, minimumContrast: 40), 0.03,
                                 "Link \(index): chain ink must occupy its leading reserved space, not the previous line's trailing edge")
            var fragments = 0
            layout.enumerateLineFragments(forGlyphRange: range) { line, used, _, lineGlyphs, _ in
                fragments += 1
                let fragment = NSIntersectionRange(range, lineGlyphs)
                let left = line.minX + layout.location(forGlyphAt: fragment.location).x
                // Get the next visible glyph's caret edge (or the used line
                // end), independently of the enclosing-rect API under test.
                let next = (NSMaxRange(fragment)..<NSMaxRange(lineGlyphs)).first {
                    !layout.propertyForGlyph(at: $0).contains(.null)
                }
                let right = next.map { line.minX + layout.location(forGlyphAt: $0).x } ?? used.maxX
                let band = CGRect(x: origin.x + left + 4, y: origin.y + used.minY + 1,
                                  width: max(0, right - left - 8), height: 1)
                XCTAssertGreaterThan(band.width, 8)
                XCTAssertGreaterThan(blueFraction(in: band, minimumContrast: 8), 0.75,
                                     "Link \(index), fragment \(fragments): pill must span the URL, not collapse beside it")
            }
            if index == 0 { XCTAssertGreaterThan(fragments, 1, "exercise a wrapped first URL") }
        }
        if starts.count == 2 {
            XCTAssertGreaterThan(starts[1].y, starts[0].y)
            XCTAssertLessThan(starts[1].x, starts[0].x, "the italic link starts at the left after a hidden delimiter")
        }
        XCTAssertEqual(storage.string, text)
    }
}
#endif
