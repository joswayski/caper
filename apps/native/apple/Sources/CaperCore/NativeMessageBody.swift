import Foundation

#if os(macOS)
import AppKit

/// Read-only rich text used by native forward previews, not a second editor.
/// It shares the message parser, hidden-spoiler policy and link/quote renderer.
@MainActor final class AppKitMessageBody: NSTextView {
    private var original = NSAttributedString(string: "")
    private var revealed: Set<Int> = []
    private var width: CGFloat = 0
    var maximumLines = 0 {
        didSet {
            textContainer?.maximumNumberOfLines = maximumLines
            textContainer?.lineBreakMode = maximumLines == 0 ? .byWordWrapping : .byTruncatingTail
            invalidateIntrinsicContentSize()
        }
    }
    init() {
        let storage = NSTextStorage(), layout = DraftLinkLayoutManager(), container = NSTextContainer(size: NSSize(width: 1, height: CGFloat.greatestFiniteMagnitude))
        storage.addLayoutManager(layout); layout.addTextContainer(container); container.lineFragmentPadding = 0
        super.init(frame: .zero, textContainer: container)
        isEditable = false; isSelectable = true; drawsBackground = false
        textContainerInset = .zero; isHorizontallyResizable = false; isVerticallyResizable = true
        translatesAutoresizingMaskIntoConstraints = false
        // As in the timeline: Caper's link colour without AppKit's blue underline.
        linkTextAttributes = [.foregroundColor: CaperThemeStore.linkColor, .cursor: NSCursor.pointingHand]
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var acceptableDragTypes: [NSPasteboard.PasteboardType] { [] }
    func update(_ text: NSAttributedString) {
        guard !original.isEqual(to: text) else { return }
        original = text; revealed = []; render()
    }
    private func render() {
        let text = NSMutableAttributedString(attributedString: original)
        NativePresentation.concealSpoilers(text, revealed: revealed); textStorage?.setAttributedString(text)
        let hidden = NativePresentation.hiddenSpoilers(in: text)
        setAccessibilityCustomActions(hidden.isEmpty ? [] : [NSAccessibilityCustomAction(name: "Spoiler, reveal") { [weak self] in
            guard let self else { return false }; self.revealed.formUnion(hidden); self.render(); return true
        }])
        invalidateIntrinsicContentSize()
    }
    override func accessibilityValue() -> String? { NativePresentation.spokenText(attributedString()) }
    override var intrinsicContentSize: NSSize {
        guard let layoutManager, let textContainer else { return .zero }
        textContainer.containerSize.width = max(1, bounds.width)
        layoutManager.ensureLayout(for: textContainer)
        return NSSize(width: NSView.noIntrinsicMetric, height: ceil(layoutManager.usedRect(for: textContainer).height))
    }
    override func layout() {
        super.layout()
        if width != bounds.width { width = bounds.width; textContainer?.containerSize.width = max(1, width); invalidateIntrinsicContentSize() }
    }
    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 1, let layoutManager, let textContainer, let storage = textStorage, storage.length > 0 {
            let point = convert(event.locationInWindow, from: nil), origin = textContainerOrigin
            let local = NSPoint(x: point.x - origin.x, y: point.y - origin.y)
            let glyph = layoutManager.glyphIndex(for: local, in: textContainer)
            if glyph < layoutManager.numberOfGlyphs,
               layoutManager.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: textContainer).insetBy(dx: -2, dy: -1).contains(local),
               let group = storage.attribute(NativePresentation.hiddenSpoilerAttribute, at: layoutManager.characterIndexForGlyph(at: glyph), effectiveRange: nil) as? Int {
                revealed.insert(group); render(); return
            }
        }
        super.mouseDown(with: event)
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitMessageBody: UITextView, UIGestureRecognizerDelegate {
    private var original = NSAttributedString(string: "")
    private var revealed: Set<Int> = []
    init() {
        let storage = NSTextStorage(), layout = DraftLinkLayoutManager(), container = NSTextContainer(size: .zero)
        storage.addLayoutManager(layout); layout.addTextContainer(container); container.lineFragmentPadding = 0
        super.init(frame: .zero, textContainer: container)
        isEditable = false; isScrollEnabled = false; backgroundColor = .clear; textContainerInset = .zero
        let tap = UITapGestureRecognizer(target: self, action: #selector(reveal(_:))); tap.delegate = self; addGestureRecognizer(tap)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ text: NSAttributedString) {
        guard !original.isEqual(to: text) else { return }; original = text; revealed = []; render()
    }
    private func render() {
        let text = NSMutableAttributedString(attributedString: original); NativePresentation.concealSpoilers(text, revealed: revealed)
        attributedText = text; accessibilityValue = NativePresentation.spokenText(text)
        let hidden = NativePresentation.hiddenSpoilers(in: text)
        accessibilityCustomActions = hidden.isEmpty ? [] : [UIAccessibilityCustomAction(name: "Spoiler, reveal", actionHandler: { [weak self] _ in
            guard let self else { return false }; self.revealed.formUnion(hidden); self.render(); return true
        })]
        invalidateIntrinsicContentSize()
    }
    private func group(at point: CGPoint) -> Int? {
        guard textStorage.length > 0 else { return nil }
        let glyph = layoutManager.glyphIndex(for: point, in: textContainer)
        guard glyph < layoutManager.numberOfGlyphs,
              layoutManager.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: textContainer).insetBy(dx: -2, dy: -1).contains(point) else { return nil }
        return textStorage.attribute(NativePresentation.hiddenSpoilerAttribute, at: layoutManager.characterIndexForGlyph(at: glyph), effectiveRange: nil) as? Int
    }
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool { group(at: touch.location(in: self)) != nil }
    @objc private func reveal(_ tap: UITapGestureRecognizer) {
        guard !SwipeTapGuard.swiping, let group = group(at: tap.location(in: self)) else { return }; revealed.insert(group); render()
    }
}
#endif
