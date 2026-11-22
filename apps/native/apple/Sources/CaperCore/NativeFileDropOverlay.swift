import QuartzCore
import SwiftUI
#if os(macOS)
import AppKit
typealias FileDropPlatformView = NSView
private typealias DropColor = NSColor
#else
import UIKit
typealias FileDropPlatformView = UIView
private typealias DropColor = UIColor
#endif

/// The native clients share the same layer artwork and seeded motion as the
/// SwiftUI reference, without hosting a SwiftUI view or intercepting the drag.
@MainActor final class NativeFileDropOverlay: FileDropPlatformView {
    private let scrim = CALayer()
    private let panel = CALayer()
    private let outline = CAShapeLayer()
    private let stage = CALayer()
    private let cards = (0..<3).map { _ in CALayer() }
    private let character = CALayer()
    // TextKit draws the text into plain layers: CATextLayer will not break an
    // overlong word such as a long channel name, so it clipped instead of wrapping.
    private let titleLayer = CALayer()
    private let detailLayer = CALayer()
    private var titleString = NSAttributedString()
    private var detailString = NSAttributedString()
    private let titleText: String
    private let detailText: String
    private let plan: FileDropMotion.Plan
    private let started = CACurrentMediaTime()
    private var timer: Timer?

    init(title: String, detail: String) {
        titleText = title; detailText = detail
        let fixture = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
        plan = FileDropMotion.Plan(seed: fixture ? 0 : UInt32.random(in: .min ... .max))
        let avatar = fixture ? 0 : Int.random(in: 0..<CaperDailyIcon.count)
        super.init(frame: .zero)
        #if os(macOS)
        wantsLayer = true
        let backing = layer!
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
        setAccessibilityLabel(title + "\n" + detail)
        setAccessibilityIdentifier("attachment-drop-overlay")
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(refreshMotion),
            name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)
        #else
        let backing = layer
        isUserInteractionEnabled = false
        isAccessibilityElement = true
        accessibilityTraits = .staticText
        accessibilityLabel = title + "\n" + detail
        accessibilityIdentifier = "attachment-drop-overlay"
        NotificationCenter.default.addObserver(self, selector: #selector(refreshMotion),
            name: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(textSizeChanged),
            name: UIContentSizeCategory.didChangeNotification, object: nil)
        #endif
        backing.addSublayer(scrim)
        backing.addSublayer(panel)
        scrim.backgroundColor = DropColor(CaperTheme.blackout).cgColor
        panel.backgroundColor = DropColor(CaperTheme.surface).cgColor
        panel.cornerRadius = 8
        panel.borderWidth = 1
        panel.borderColor = DropColor(CaperTheme.border).cgColor
        panel.shadowColor = DropColor.black.cgColor
        panel.shadowOpacity = 0.45
        panel.shadowRadius = 24
        panel.shadowOffset = CGSize(width: 0, height: 18)
        outline.fillColor = nil
        outline.strokeColor = DropColor(CaperTheme.terracotta).cgColor
        outline.lineWidth = 1.5
        outline.lineDashPattern = [5, 4]
        panel.addSublayer(outline)
        panel.addSublayer(stage)
        for index in [1, 0, 2] {
            let card = cards[index]
            card.bounds = CGRect(x: 0, y: 0, width: 54, height: 66)
            card.anchorPoint = CGPoint(x: 0.5, y: 1)
            card.cornerRadius = 8
            card.backgroundColor = DropColor(CaperTheme.raised).cgColor
            card.borderColor = DropColor(CaperTheme.border).cgColor
            card.borderWidth = 1
            let icon = CALayer()
            icon.frame = CGRect(x: 15, y: 21, width: 24, height: 24)
            icon.contentsGravity = .resizeAspect
            icon.contents = Self.symbol(["photo", "doc.text", "film"][index])
            card.addSublayer(icon)
            stage.addSublayer(card)
        }
        character.bounds = CGRect(x: 0, y: 0, width: 76, height: 76)
        character.anchorPoint = CGPoint(x: 0.5, y: 1)
        character.contentsGravity = .resizeAspect
        #if os(macOS)
        character.contents = caperResourceBundle.image(forResource: "caper-branding-\(avatar)")?
            .cgImage(forProposedRect: nil, context: nil, hints: nil)
        #else
        character.contents = UIImage(named: "caper-branding-\(avatar)", in: caperResourceBundle, compatibleWith: nil)?.cgImage
        #endif
        stage.addSublayer(character)
        for text in [titleLayer, detailLayer] { panel.addSublayer(text) }
        updateText()
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    deinit {
        timer?.invalidate()
        NotificationCenter.default.removeObserver(self)
        #if os(macOS)
        NSWorkspace.shared.notificationCenter.removeObserver(self)
        #endif
    }

    #if os(macOS)
    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func layout() { super.layout(); layoutLayers() }
    override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); refreshMotion() }
    override func viewDidChangeBackingProperties() { super.viewDidChangeBackingProperties(); layoutLayers() }
    override func accessibilityFrame() -> NSRect { NSAccessibility.screenRect(fromView: self, rect: bounds) }
    private var scale: CGFloat { window?.backingScaleFactor ?? 2 }
    #else
    override func layoutSubviews() { super.layoutSubviews(); layoutLayers() }
    override func didMoveToWindow() { super.didMoveToWindow(); refreshMotion() }
    private var scale: CGFloat { traitCollection.displayScale }
    @objc private func textSizeChanged() { updateText(); setNeedsLayout() }
    #endif
    private var reduceMotion: Bool { CaperAccessibility.reduceMotion }

    private func updateText() {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        paragraph.lineBreakMode = .byWordWrapping
        for (title, value, size, bold) in [(true, titleText, CGFloat(18), true), (false, detailText, CGFloat(13), false)] {
            #if os(macOS)
            let font = AppKitKit.font(size, weight: bold ? .bold : .regular)
            #else
            let base = UIFont(name: bold ? "Inrond40-Bold" : "Inrond40-Regular", size: size)
                ?? UIFont.systemFont(ofSize: size, weight: bold ? .bold : .regular)
            let font = UIFontMetrics.default.scaledFont(for: base, compatibleWith: traitCollection)
            #endif
            let string = NSAttributedString(string: value, attributes: [
                .font: font, .foregroundColor: DropColor(bold ? CaperTheme.text : CaperTheme.muted),
                .paragraphStyle: paragraph,
            ])
            if title { titleString = string } else { detailString = string }
        }
    }

    private func layoutLayers() {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        scrim.frame = bounds
        let width = min(360, max(0, bounds.width - 32))
        let textWidth = max(1, width - 48)
        func height(_ text: NSAttributedString) -> CGFloat {
            ceil(text.boundingRect(with: CGSize(width: textWidth, height: .greatestFiniteMagnitude),
                options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil).height)
        }
        let titleHeight = height(titleString), detailHeight = height(detailString)
        // Same 20pt top, 128pt artwork, 14pt gap, 6pt text gap and 26pt bottom
        // padding as FileDropOverlay; long destination names wrap, not truncate.
        let panelHeight = 20 + 128 + 14 + titleHeight + 6 + detailHeight + 26
        panel.bounds = CGRect(x: 0, y: 0, width: width, height: panelHeight)
        panel.position = CGPoint(x: bounds.midX, y: bounds.midY)
        panel.shadowPath = CGPath(roundedRect: panel.bounds, cornerWidth: 8, cornerHeight: 8, transform: nil)
        outline.frame = panel.bounds
        outline.path = CGPath(rect: panel.bounds.insetBy(dx: 9.75, dy: 9.75), transform: nil)
        stage.frame = CGRect(x: (width - 176) / 2, y: 20, width: 176, height: 128)
        titleLayer.frame = CGRect(x: 24, y: 162, width: textWidth, height: titleHeight)
        detailLayer.frame = CGRect(x: 24, y: 162 + titleHeight + 6, width: textWidth, height: detailHeight)
        for (layer, text) in [(titleLayer, titleString), (detailLayer, detailString)] {
            layer.contentsScale = scale
            layer.contents = Self.image(text, size: layer.bounds.size, scale: scale)
        }
        render()
        CATransaction.commit()
    }

    @objc private func refreshMotion() {
        timer?.invalidate(); timer = nil
        render()
        guard window != nil, !reduceMotion else { return }
        let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.render() }
        }
        // File drags use the tracking run-loop mode on Mac.
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    private func render() {
        let elapsed = CACurrentMediaTime() - started, still = reduceMotion
        let pose = FileDropMotion.pose(plan, elapsed, still: still)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        scrim.opacity = Float(0.82 * (still ? 1 : FileDropMotion.progress(elapsed, delay: 0, duration: 0.14)))
        panel.opacity = Float(still ? 1 : FileDropMotion.progress(elapsed, delay: 0, duration: 0.12))
        let pop = still ? 1 : FileDropMotion.progress(elapsed, delay: 0, duration: 0.26)
        panel.setAffineTransform(CGAffineTransform(translationX: 0, y: 10 * (1 - FileDropMotion.easeOutCubic(pop))))
        for (layer, card) in zip(cards, pose.cards) {
            layer.position = CGPoint(x: 88 + card.x, y: 98 + card.y)
            layer.setAffineTransform(CGAffineTransform(rotationAngle: card.turn * .pi / 180))
            layer.opacity = Float(card.opacity)
        }
        character.position = CGPoint(x: 88, y: 132 + pose.character.y)
        character.setAffineTransform(CGAffineTransform(rotationAngle: pose.character.turn * .pi / 180)
            .scaledBy(x: pose.character.scaleX, y: pose.character.scaleY))
        character.opacity = Float(pose.character.opacity)
        CATransaction.commit()
    }

    private static func image(_ text: NSAttributedString, size: CGSize, scale: CGFloat) -> CGImage? {
        guard size.width >= 1, size.height >= 1 else { return nil }
        let rect = CGRect(origin: .zero, size: size)
        #if os(macOS)
        guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(ceil(size.width * scale)),
            pixelsHigh: Int(ceil(size.height * scale)), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
            isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { return nil }
        rep.size = size
        guard let context = NSGraphicsContext(bitmapImageRep: rep) else { return nil }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context.cgContext, flipped: true)
        context.cgContext.translateBy(x: 0, y: size.height); context.cgContext.scaleBy(x: 1, y: -1)
        text.draw(with: rect, options: [.usesLineFragmentOrigin, .usesFontLeading])
        NSGraphicsContext.restoreGraphicsState()
        return rep.cgImage
        #else
        let format = UIGraphicsImageRendererFormat(); format.scale = scale; format.opaque = false
        return UIGraphicsImageRenderer(size: size, format: format).image { _ in
            text.draw(with: rect, options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil)
        }.cgImage
        #endif
    }

    private static func symbol(_ name: String) -> CGImage? {
        #if os(macOS)
        let configuration = NSImage.SymbolConfiguration(pointSize: 18, weight: .medium)
            .applying(.init(paletteColors: [DropColor(CaperTheme.muted)]))
        return NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(configuration)?.cgImage(forProposedRect: nil, context: nil, hints: nil)
        #else
        guard let symbol = UIImage(systemName: name, withConfiguration: UIImage.SymbolConfiguration(pointSize: 18, weight: .medium))?
            .withTintColor(DropColor(CaperTheme.muted), renderingMode: .alwaysOriginal) else { return nil }
        // Symbol images can be vector-backed, without a directly usable CGImage.
        return UIGraphicsImageRenderer(size: symbol.size).image { _ in
            symbol.draw(at: .zero)
        }.cgImage
        #endif
    }
}
