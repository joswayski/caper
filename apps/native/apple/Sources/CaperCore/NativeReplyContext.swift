import Observation
import QuartzCore
import SwiftUI

/// The reply's spring is seeded by the reply, not by the quoted message. Only
/// scroll/geometry changes excite it; Reduce Motion and detachment stop it.
@MainActor private final class NativeReplyWire {
    let layer = CALayer()
    private let wire = CAShapeLayer(), upper = CALayer(), lower = CALayer()
    private let seed: UInt32
    private var y: CGFloat?, displacement: CGFloat = 0, velocity: CGFloat = 0, height: CGFloat = 0
    private var motion: Task<Void, Never>?
    init(key: String) {
        seed = key.utf8.reduce(UInt32(2166136261)) { ($0 ^ UInt32($1)) &* 16777619 }
        #if os(macOS)
        let border = AppKitKit.border.cgColor, surface = AppKitKit.surface.cgColor, accent = AppKitKit.terracotta.cgColor
        wire.strokeColor = AppKitKit.muted.withAlphaComponent(0.75).cgColor
        #else
        let border = UIColor(CaperTheme.border).cgColor, surface = UIColor(CaperTheme.surface).cgColor, accent = UIColor(CaperTheme.terracotta).cgColor
        wire.strokeColor = UIColor(CaperTheme.muted).withAlphaComponent(0.75).cgColor
        #endif
        wire.fillColor = nil; wire.lineWidth = 1.1; wire.lineCap = .round; layer.addSublayer(wire)
        for socket in [upper, lower] {
            let cup = CGMutablePath(); cup.move(to: CGPoint(x: 1, y: 2.1)); cup.addQuadCurve(to: CGPoint(x: 1, y: 6.9), control: CGPoint(x: 0, y: 4.5))
            cup.addLine(to: CGPoint(x: 12, y: 9)); cup.addLine(to: CGPoint(x: 12, y: 0)); cup.closeSubpath()
            let metal = CAGradientLayer(); metal.frame = CGRect(x: -13, y: -4.5, width: 13, height: 9)
            metal.colors = [CGColor(gray: 0.40, alpha: 1), CGColor(gray: 0.66, alpha: 1), CGColor(gray: 0.47, alpha: 1), CGColor(gray: 0.21, alpha: 1)]
            metal.locations = [0, 0.32, 0.55, 1]; let mask = CAShapeLayer(); mask.path = cup; metal.mask = mask; socket.addSublayer(metal)
            let outline = CAShapeLayer(); outline.frame = metal.frame; outline.path = cup; outline.fillColor = nil; outline.strokeColor = border; outline.lineWidth = 0.6; socket.addSublayer(outline)
            let rim = CAShapeLayer(); rim.path = CGPath(ellipseIn: CGRect(x: -2.6, y: -4.5, width: 3.2, height: 9), transform: nil)
            rim.fillColor = surface; rim.strokeColor = accent; rim.lineWidth = 0.8; socket.addSublayer(rim); layer.addSublayer(socket)
        }
    }
    isolated deinit { motion?.cancel() }
    func update(y: CGFloat, height: CGFloat, hidden: Bool) {
        self.height = height; layer.isHidden = hidden
        let delta = (self.y ?? y) - y; self.y = y
        if hidden || CaperAccessibility.reduceMotion { stop(); self.y = y; draw(); return }
        if delta != 0 {
            let limit = 5 + CGFloat(seed >> 24) / 255 * 2
            velocity = max(-limit * 16, min(limit * 16, velocity + delta * (1 + CGFloat((seed >> 16) & 255) / 255 * 0.4)))
            if motion == nil {
                motion = Task { [weak self] in
                    var previous = Date()
                    while !Task.isCancelled {
                        do { try await Task.sleep(for: .milliseconds(16)) } catch { return }
                        guard let self else { return }
                        if CaperAccessibility.reduceMotion { stop(); draw(); return }
                        let now = Date(), dt = CGFloat(min(now.timeIntervalSince(previous), 0.032)); previous = now
                        velocity -= ((560 + CGFloat(seed & 255) / 255 * 400) * displacement + (28 + CGFloat((seed >> 8) & 255) / 255 * 14) * velocity) * dt
                        displacement = max(-limit, min(limit, displacement + velocity * dt)); draw()
                        if abs(displacement) < 0.01 && abs(velocity) < 0.05 { stop(); draw(); return }
                    }
                }
            }
        }
        draw()
    }
    func stop() { motion?.cancel(); motion = nil; displacement = 0; velocity = 0; y = nil }
    private func draw() {
        CATransaction.begin(); CATransaction.setDisableActions(true); defer { CATransaction.commit() }
        let angle = displacement * 0.025, dx = 12 * cos(angle), dy = 12 * sin(angle)
        let bottom = height + 23, start = CGPoint(x: -dx, y: bottom + dy), end = CGPoint(x: 56 - dx, y: 20 - dy), span = start.y - end.y
        let path = CGMutablePath(); path.move(to: start)
        path.addCurve(to: end, control1: CGPoint(x: start.x + 8 + displacement, y: start.y - span * 0.3), control2: CGPoint(x: end.x - 30 + displacement, y: end.y + span * 0.3))
        wire.path = path; upper.position = CGPoint(x: 56, y: 20); lower.position = CGPoint(x: 0, y: bottom)
        upper.setAffineTransform(CGAffineTransform(rotationAngle: angle)); lower.setAffineTransform(CGAffineTransform(rotationAngle: -angle))
    }
}

@MainActor private func replyExcerpt(_ reference: MessageReply) -> String {
    if reference.deleted == true { return MessageDeletion.quote }
    let text = reference.fullText ?? reference.text
    return text.isEmpty ? VoiceMessage.excerptFallback(hasAttachments: reference.hasAttachments, hasVoice: reference.hasVoice, attachments: reference.attachments ?? []) : text
}

#if os(macOS)
import AppKit

@MainActor final class AppKitReplyContext: NSView {
    var heightChanged: (() -> Void)?
    private let reference: MessageReply, chat: ChatModel, wire: NativeReplyWire
    private let panel = CALayer(), avatar = AppKitAvatar(), name = AppKitKit.label("", size: 12, weight: .bold)
    private let excerpt = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let hint = AppKitKit.label("View original", size: 11, color: AppKitKit.muted)
    private let open = AppKitMediaHitTarget()
    private var expand: AppKitKitButton!
    private let thumbnails = NSStackView()
    private var loads: [Task<Void, Never>] = []
    private let observation = AppKitObservation()
    private var scrolling: NSKeyValueObservation?
    private var expanded = false, blocked = false, lastHeight: CGFloat = 0
    override var isFlipped: Bool { true }
    init(reference: MessageReply, motionKey: String, chat: ChatModel) {
        self.reference = reference; self.chat = chat; wire = NativeReplyWire(key: motionKey)
        super.init(frame: .zero); wantsLayer = true; layer?.addSublayer(panel); layer?.addSublayer(wire.layer)
        panel.cornerRadius = 8; panel.backgroundColor = AppKitKit.surface.cgColor; panel.borderWidth = 1; panel.borderColor = AppKitKit.border.withAlphaComponent(0.7).cgColor
        avatar.update(name: reference.author.name, avatarId: reference.author.avatarId, picture: reference.author.avatarPicture)
        open.handler = { Task { if reference.deleted != true { _ = await chat.goToReply(reference) } } }
        expand = AppKitKitButton("Show more", kind: .plain) { [weak self] in self?.expanded.toggle(); self?.render(); self?.heightChanged?() }
        thumbnails.orientation = .horizontal; thumbnails.alignment = .centerY; thumbnails.spacing = 4
        for view in [avatar, name, hint, excerpt, thumbnails, open, expand!] { addSubview(view); view.translatesAutoresizingMaskIntoConstraints = true }
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(motionChanged), name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { loads.forEach { $0.cancel() }; NSWorkspace.shared.notificationCenter.removeObserver(self) }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow(); scrolling = nil
        if window != nil {
            observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
            makeThumbnails()
            scrolling = enclosingScrollView?.contentView.observe(\.bounds, options: []) { [weak self] _, _ in MainActor.assumeIsolated { self?.moveWire() } }
        } else { observation.stop(); wire.stop(); loads.forEach { $0.cancel() } }
    }
    @objc private func motionChanged() { wire.stop(); moveWire() }
    private func render() {
        let wasHidden = blocked; blocked = chat.blockedAuthorIDs.contains(reference.author.id) && reference.author.id != chat.viewerID
        name.stringValue = blocked ? "Blocked message" : reference.author.name + (reference.hasAttachments && reference.deleted != true ? (reference.hasVoice ? "  ◉" : "  📎") : "")
        excerpt.stringValue = blocked ? "" : replyExcerpt(reference); excerpt.maximumNumberOfLines = expanded ? 0 : 3
        avatar.isHidden = blocked; hint.isHidden = reference.deleted == true; open.isEnabled = reference.deleted != true
        open.setAccessibilityLabel(blocked ? "View original blocked message" : "View original message from \(reference.author.name)")
        open.setAccessibilityHelp(blocked ? nil : excerpt.stringValue)
        expand.title = expanded ? "Show less" : "Show more"; expand.setAccessibilityValue(expanded ? "Expanded" : "Collapsed")
        if thumbnails.arrangedSubviews.isEmpty || wasHidden != blocked { makeThumbnails() }
        needsLayout = true; invalidateIntrinsicContentSize()
    }
    private func makeThumbnails() {
        loads.forEach { $0.cancel() }; loads = []; thumbnails.arrangedSubviews.forEach { $0.removeFromSuperview() }
        guard !blocked, reference.deleted != true else { return }
        let attachments = reference.attachments ?? []
        for file in attachments.prefix(3) {
            let image = NSImageView(); image.imageScaling = .scaleProportionallyUpOrDown; image.wantsLayer = true
            image.layer?.cornerRadius = 6; image.layer?.masksToBounds = true; image.layer?.backgroundColor = AppKitKit.raised.cgColor
            image.image = NSImage(systemSymbolName: file.spoiler ? "eye.slash" : file.voice ? "mic" : "paperclip", accessibilityDescription: nil)
            NSLayoutConstraint.activate([image.widthAnchor.constraint(equalToConstant: 44), image.heightAnchor.constraint(equalToConstant: 44)])
            image.setAccessibilityElement(false); thumbnails.addArrangedSubview(image)
            if !file.spoiler && !file.unavailable && file.status == .ready && (file.kind == .image || file.kind == .video),
               file.previewUrl != nil || (file.kind == .image && !file.animated && file.contentType != "image/gif") {
                loads.append(Task { [weak image, chat] in
                    let value = await loadAttachmentImage(file, preview: true, chat: chat, maxPixelSize: 132).image
                    guard !Task.isCancelled, let value else { return }; image?.image = NSImage(cgImage: value, size: NSSize(width: 44, height: 44))
                })
            }
        }
        if attachments.count > 3 { thumbnails.addArrangedSubview(AppKitKit.label("+\(attachments.count - 3)", size: 12, color: AppKitKit.muted)) }
    }
    private func metrics(_ width: CGFloat) -> (text: CGFloat, expand: Bool, height: CGFloat) {
        let full = blocked ? 0 : NativePresentation.textHeight(NSAttributedString(string: excerpt.stringValue, attributes: [.font: AppKitKit.font(12)]), width: max(1, width - 68))
        let more = !blocked && reference.deleted != true && (expanded || full > 48)
        let text = expanded ? full : min(48, full)
        return (text, more, 20 + 20 + text + (thumbnails.arrangedSubviews.isEmpty ? 0 : 47) + (more ? 32 : 0) + 13)
    }
    override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: metrics(bounds.width).height) }
    override func layout() {
        super.layout(); let width = max(1, bounds.width - 68), m = metrics(bounds.width)
        CATransaction.begin(); CATransaction.setDisableActions(true); panel.frame = CGRect(x: 44, y: 0, width: max(1, bounds.width - 44), height: m.height - 13); CATransaction.commit()
        avatar.frame = CGRect(x: 56, y: 12, width: 16, height: 16)
        let nameX: CGFloat = blocked ? 56 : 78
        name.frame = CGRect(x: nameX, y: 10, width: max(1, bounds.width - nameX - (hint.isHidden ? 12 : 86)), height: 20)
        hint.frame = CGRect(x: max(56, bounds.width - 86), y: 10, width: 74, height: 20)
        excerpt.preferredMaxLayoutWidth = width; excerpt.frame = CGRect(x: 56, y: 33, width: width, height: m.text)
        thumbnails.frame = CGRect(x: 56, y: 36 + m.text, width: width, height: 44)
        let bottom = m.height - 13 - (m.expand ? 32 : 0)
        open.frame = CGRect(x: 44, y: 0, width: max(1, bounds.width - 44), height: bottom)
        expand.isHidden = !m.expand; expand.frame = CGRect(x: 56, y: bottom - 4, width: width, height: 32)
        moveWire(); if lastHeight != m.height { lastHeight = m.height; invalidateIntrinsicContentSize(); heightChanged?() }
    }
    private func moveWire() { wire.update(y: convert(NSPoint.zero, to: nil).y, height: intrinsicContentSize.height, hidden: blocked) }
}
#elseif os(iOS)
import UIKit

@MainActor public final class UIKitNativeReplyContext: UIView {
    private var reference: MessageReply
    private let chat: ChatModel
    private var motionKey: String, wire: NativeReplyWire
    private let panel = CALayer(), avatar = UIKitAvatar(side: 16)
    private let name = UIKitMessageDialog.label("", size: 12, bold: true), excerpt = UIKitMessageDialog.label("", size: 12), hint = UIKitMessageDialog.label("View original", size: 11)
    private let open = UIButton(type: .custom), expand = UIButton(type: .system), thumbnails = UIStackView()
    private var loads: [Task<Void, Never>] = [], scrolling: NSKeyValueObservation?
    private var expanded = false, blocked = false, generation = 0, lastHeight: CGFloat = 0
    public init(reference: MessageReply, motionKey: String, chat: ChatModel) {
        self.reference = reference; self.motionKey = motionKey; self.chat = chat; wire = NativeReplyWire(key: motionKey)
        super.init(frame: .zero); layer.addSublayer(panel); layer.addSublayer(wire.layer)
        panel.cornerRadius = 8; panel.backgroundColor = UIColor(CaperTheme.surface).cgColor; panel.borderWidth = 1; panel.borderColor = UIColor(CaperTheme.border).withAlphaComponent(0.7).cgColor
        open.addAction(UIAction { [weak self] _ in
            guard let self, !SwipeTapGuard.swiping, self.reference.deleted != true else { return }; Task { _ = await chat.goToReply(self.reference) }
        }, for: .touchUpInside)
        expand.addAction(UIAction { [weak self] _ in self?.expanded.toggle(); self?.render(); self?.invalidateIntrinsicContentSize() }, for: .touchUpInside)
        expand.titleLabel?.font = UIFont(name: "Satoshi-Regular", size: 11); expand.tintColor = UIColor(CaperTheme.link); expand.contentHorizontalAlignment = .leading
        thumbnails.axis = .horizontal; thumbnails.alignment = .center; thumbnails.spacing = 4
        name.numberOfLines = 1; [avatar, name, hint, excerpt, thumbnails, open, expand].forEach(addSubview)
        avatar.translatesAutoresizingMaskIntoConstraints = true
        NotificationCenter.default.addObserver(self, selector: #selector(motionChanged), name: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil)
        render(); makeThumbnails()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { loads.forEach { $0.cancel() }; NotificationCenter.default.removeObserver(self) }
    public func update(reference: MessageReply, motionKey: String) {
        if self.motionKey != motionKey { expanded = false; wire.stop(); wire.layer.removeFromSuperlayer(); wire = NativeReplyWire(key: motionKey); layer.addSublayer(wire.layer) }
        let changed = self.reference != reference; self.reference = reference; self.motionKey = motionKey
        render(); if changed { makeThumbnails() }
    }
    public override func didMoveToWindow() {
        super.didMoveToWindow(); generation += 1; scrolling = nil
        if window != nil {
            observe(generation); makeThumbnails(); var ancestor = superview
            while let current = ancestor {
                if let scroll = current as? UIScrollView {
                    scrolling = scroll.observe(\.contentOffset, options: []) { [weak self] _, _ in MainActor.assumeIsolated { self?.moveWire() } }; break
                }; ancestor = current.superview
            }
        } else { wire.stop(); loads.forEach { $0.cancel() } }
    }
    private func observe(_ token: Int) { guard window != nil, token == generation else { return }; withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } } }
    @objc private func motionChanged() { wire.stop(); moveWire() }
    private func render() {
        let wasHidden = blocked; blocked = chat.blockedAuthorIDs.contains(reference.author.id) && reference.author.id != chat.viewerID
        name.text = blocked ? "Blocked message" : reference.author.name + (reference.hasAttachments && reference.deleted != true ? (reference.hasVoice ? "  ◉" : "  📎") : "")
        excerpt.text = blocked ? "" : replyExcerpt(reference); excerpt.numberOfLines = expanded ? 0 : 3
        avatar.show(name: reference.author.name, avatarID: reference.author.avatarId, picture: reference.author.avatarPicture); avatar.isHidden = blocked
        hint.isHidden = reference.deleted == true; open.isEnabled = reference.deleted != true
        open.accessibilityLabel = blocked ? "View original blocked message" : "View original message from \(reference.author.name)"; open.accessibilityHint = blocked ? nil : excerpt.text
        expand.setTitle(expanded ? "Show less" : "Show more", for: .normal); expand.accessibilityValue = expanded ? "Expanded" : "Collapsed"
        if wasHidden != blocked { makeThumbnails() }; setNeedsLayout(); invalidateIntrinsicContentSize()
    }
    private func makeThumbnails() {
        loads.forEach { $0.cancel() }; loads = []; thumbnails.arrangedSubviews.forEach { $0.removeFromSuperview() }
        guard !blocked, reference.deleted != true else { return }
        let attachments = reference.attachments ?? []
        for file in attachments.prefix(3) {
            let image = UIImageView(); image.contentMode = .scaleAspectFill; image.clipsToBounds = true; image.layer.cornerRadius = 6; image.backgroundColor = UIColor(CaperTheme.raised)
            image.image = UIImage(systemName: file.spoiler ? "eye.slash" : file.voice ? "mic" : "paperclip"); image.tintColor = UIColor(CaperTheme.muted)
            NSLayoutConstraint.activate([image.widthAnchor.constraint(equalToConstant: 44), image.heightAnchor.constraint(equalToConstant: 44)]); thumbnails.addArrangedSubview(image)
            if !file.spoiler && !file.unavailable && file.status == .ready && (file.kind == .image || file.kind == .video),
               file.previewUrl != nil || (file.kind == .image && !file.animated && file.contentType != "image/gif") {
                loads.append(Task { [weak image, chat] in
                    let value = await loadAttachmentImage(file, preview: true, chat: chat, maxPixelSize: 132).image
                    guard !Task.isCancelled, let value else { return }; image?.image = UIImage(cgImage: value)
                })
            }
        }
        if attachments.count > 3 { thumbnails.addArrangedSubview(UIKitMessageDialog.label("+\(attachments.count - 3)", size: 12)) }
    }
    private func metrics(_ width: CGFloat) -> (text: CGFloat, expand: Bool, height: CGFloat) {
        let full = blocked ? 0 : NativePresentation.textHeight(NSAttributedString(string: excerpt.text ?? "", attributes: [.font: excerpt.font!]), width: max(1, width - 68))
        let limit = ceil(excerpt.font.lineHeight * 3), more = !blocked && reference.deleted != true && (expanded || full > limit), text = expanded ? full : min(limit, full)
        return (text, more, max(44, 40 + text + (thumbnails.arrangedSubviews.isEmpty ? 0 : 47)) + (more ? 44 : 0) + 13)
    }
    public override var intrinsicContentSize: CGSize { CGSize(width: UIView.noIntrinsicMetric, height: metrics(bounds.width).height) }
    public override func layoutSubviews() {
        super.layoutSubviews(); let width = max(1, bounds.width - 68), m = metrics(bounds.width)
        CATransaction.begin(); CATransaction.setDisableActions(true); panel.frame = CGRect(x: 44, y: 0, width: max(1, bounds.width - 44), height: m.height - 13); CATransaction.commit()
        avatar.frame = CGRect(x: 56, y: 12, width: 16, height: 16)
        let nameX: CGFloat = blocked ? 56 : 78
        name.frame = CGRect(x: nameX, y: 10, width: max(1, bounds.width - nameX - (hint.isHidden ? 12 : 86)), height: 20)
        hint.frame = CGRect(x: max(56, bounds.width - 86), y: 10, width: 74, height: 20)
        excerpt.frame = CGRect(x: 56, y: 33, width: width, height: m.text); thumbnails.frame = CGRect(x: 56, y: 36 + m.text, width: width, height: 44)
        let bottom = m.height - 13 - (m.expand ? 44 : 0)
        open.frame = CGRect(x: 44, y: 0, width: max(1, bounds.width - 44), height: bottom)
        expand.isHidden = !m.expand; expand.frame = CGRect(x: 56, y: bottom - 4, width: width, height: 44)
        moveWire(); if lastHeight != m.height { lastHeight = m.height; invalidateIntrinsicContentSize() }
    }
    private func moveWire() { wire.update(y: convert(CGPoint.zero, to: nil).y, height: intrinsicContentSize.height, hidden: blocked) }
}
#endif
