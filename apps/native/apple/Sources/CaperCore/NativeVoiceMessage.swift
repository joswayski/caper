import Observation
import QuartzCore
import SwiftUI

/// The native player retains the shared playback owner: changing rows, paging
/// and profile presentation must not create another audio session.
#if os(macOS)
import AppKit

@MainActor final class AppKitVoiceMessage: NSView {
    var heightChanged: (() -> Void)?
    private var file: ChatAttachment
    private let chat: ChatModel?
    private let available: (() -> Bool)?
    private let observation = AppKitObservation()
    private let waveform = AppKitVoicePosition()
    private let time = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let transcript = AppKitKit.label("", size: 13, color: AppKitKit.muted)
    private var play: AppKitKitButton!, expand: AppKitKitButton!
    private let panel = CALayer(), rule = CALayer()
    private var expanded = false
    override var isFlipped: Bool { true }
    init(file: ChatAttachment, chat: ChatModel?, available: (() -> Bool)?) {
        self.file = file; self.chat = chat; self.available = available
        super.init(frame: .zero); wantsLayer = true; Self.rows.add(self)
        panel.cornerRadius = 8; panel.backgroundColor = AppKitKit.surface.cgColor
        panel.borderWidth = 1; panel.borderColor = AppKitKit.border.cgColor; layer?.addSublayer(panel)
        rule.backgroundColor = AppKitKit.border.cgColor; layer?.addSublayer(rule)
        play = AppKitKitButton("", kind: .primary) { [weak self] in
            guard let self, available?() != false else { return }; VoiceMessagePlayer.shared.toggle(self.file, chat: self.chat)
        }
        expand = AppKitKitButton("", kind: .plain) { [weak self] in
            guard let self else { return }; expanded.toggle(); render(); heightChanged?()
        }
        waveform.seek = { [weak self] fraction in
            guard let self, available?() != false else { return }; VoiceMessagePlayer.shared.seek(self.file, chat: self.chat, fraction: fraction)
        }
        transcript.isSelectable = true
        for view in [play!, waveform, time, transcript, expand!] { addSubview(view); view.translatesAutoresizingMaskIntoConstraints = true }
        setAccessibilityElement(true); setAccessibilityRole(.group); setAccessibilityIdentifier("voice-message-\(file.id)")
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(motionChanged), name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)
        transcript.setAccessibilityIdentifier("voice-transcript-text"); render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { NSWorkspace.shared.notificationCenter.removeObserver(self) }
    @objc private func motionChanged() { nativeTranscriptMotion(transcript.layer, pending: file.transcript?.display == .pending && window != nil) }
    private var transcriptLimit: CGFloat { ceil(NSLayoutManager().defaultLineHeight(for: AppKitKit.font(13)) * CGFloat(VoiceTranscript.clampedLines)) }
    func update(_ file: ChatAttachment) { self.file = file; render() }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) }
        else { observation.stop(); transcript.layer?.removeAllAnimations(); stopWhenGone() }
    }
    /// Leaving the conversation stops this note, as the SwiftUI row did on
    /// disappear. Rows are rebuilt when playback state changes, so the
    /// deferred check stops only when no row for the note is still shown.
    private static let rows = NSHashTable<AppKitVoiceMessage>.weakObjects()
    private func stopWhenGone() {
        let id = file.id
        DispatchQueue.main.async {
            guard VoiceMessagePlayer.shared.attachmentID == id,
                  !Self.rows.allObjects.contains(where: { $0.window != nil && $0.file.id == id }) else { return }
            VoiceMessagePlayer.shared.stop()
        }
    }
    private func render() {
        let player = VoiceMessagePlayer.shared, elapsed = player.elapsed(for: file)
        // A template symbol: tint it blackout on the eggshell fill, or it draws
        // in the light text colour and disappears into the button.
        play.image = NSImage(systemSymbolName: player.isPlaying(file) ? "pause.fill" : "play.fill", accessibilityDescription: nil)?
            .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 13, weight: .bold))
        play.imagePosition = .imageOnly
        play.contentTintColor = AppKitKit.onAction
        play.setAccessibilityLabel(player.isPlaying(file) ? "Pause voice message" : "Play voice message")
        time.stringValue = VoiceDuration.playerLabel(durationMs: file.durationMs, elapsedMs: elapsed)
        time.setAccessibilityLabel(elapsed == nil ? "Duration \(time.stringValue)" : "\(time.stringValue) remaining")
        waveform.file = file; waveform.needsLayout = true
        switch file.transcript?.display ?? .hidden {
        case .hidden: transcript.stringValue = ""
        case .pending: transcript.stringValue = VoiceTranscript.pendingLabel
        case .text(let text): transcript.stringValue = text
        }
        transcript.maximumNumberOfLines = expanded ? 0 : VoiceTranscript.clampedLines
        expand.title = expanded ? VoiceTranscript.collapseLabel : VoiceTranscript.expandLabel
        expand.setAccessibilityValue(expanded ? "Expanded" : "Collapsed")
        needsLayout = true
    }
    func height(for width: CGFloat) -> CGFloat {
        guard !transcript.stringValue.isEmpty else { return 44 }
        let full = NativePresentation.textHeight(NSAttributedString(string: transcript.stringValue, attributes: [.font: AppKitKit.font(13)]), width: max(1, width - 14))
        let limit = transcriptLimit
        return 54 + (expanded ? full : min(limit, full)) + (full > limit || expanded ? 28 : 0)
    }
    override func layout() {
        super.layout(); let width = bounds.width
        CATransaction.begin(); CATransaction.setDisableActions(true)
        panel.frame = CGRect(x: 0, y: 0, width: width, height: 44)
        play.frame = CGRect(x: 8, y: 6, width: 32, height: 32); play.layer?.cornerRadius = 16
        let timeWidth = max(30, time.fittingSize.width)
        time.frame = CGRect(x: width - timeWidth - 12, y: 13, width: timeWidth, height: 18)
        waveform.frame = CGRect(x: 50, y: 8, width: max(1, width - timeWidth - 72), height: 28)
        let full = NativePresentation.textHeight(NSAttributedString(string: transcript.stringValue, attributes: [.font: AppKitKit.font(13)]), width: max(1, width - 14))
        let textHeight = expanded ? full : min(transcriptLimit, full)
        transcript.isHidden = transcript.stringValue.isEmpty; rule.isHidden = transcript.isHidden
        transcript.preferredMaxLayoutWidth = max(1, width - 14)
        transcript.frame = CGRect(x: 14, y: 52, width: max(1, width - 14), height: textHeight)
        expand.isHidden = transcript.isHidden || (!expanded && full <= transcriptLimit)
        expand.frame = CGRect(x: 14, y: 54 + textHeight, width: max(1, width - 14), height: 26)
        rule.frame = CGRect(x: 4, y: 50, width: 2, height: max(0, height(for: width) - 50))
        CATransaction.commit()
        transcript.wantsLayer = true
        nativeTranscriptMotion(transcript.layer, pending: file.transcript?.display == .pending && window != nil)
    }
}

@MainActor private final class AppKitVoicePosition: NSControl {
    var file: ChatAttachment?
    var seek: ((Double) -> Void)?
    private let waveform = NativeVoiceWaveform(playedColor: AppKitKit.text.cgColor, unplayedColor: AppKitKit.muted.withAlphaComponent(0.45).cgColor)
    override init(frame: NSRect) { super.init(frame: frame); wantsLayer = true; layer?.addSublayer(waveform.layer) }
    convenience init() { self.init(frame: .zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func layout() { super.layout(); waveform.update(file, frame: bounds) }
    override func mouseDown(with event: NSEvent) { move(event) }
    override func mouseDragged(with event: NSEvent) { move(event) }
    private func move(_ event: NSEvent) { seek?(VoicePlaybackPosition.fraction(x: convert(event.locationInWindow, from: nil).x, width: bounds.width)) }
    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .slider }
    override func accessibilityLabel() -> String? { "Voice message position" }
    override func accessibilityValue() -> Any? {
        guard let file else { return nil }
        return "\(VoiceDuration.format(ms: VoiceMessagePlayer.shared.elapsed(for: file) ?? 0)) of \(VoiceDuration.format(ms: file.durationMs ?? 0))"
    }
    override func accessibilityPerformIncrement() -> Bool { adjust(0.1) }
    override func accessibilityPerformDecrement() -> Bool { adjust(-0.1) }
    private func adjust(_ delta: Double) -> Bool {
        guard let file else { return false }; seek?(VoicePlaybackPosition.progress(elapsedMs: VoiceMessagePlayer.shared.elapsed(for: file) ?? 0, durationMs: file.durationMs) + delta); return true
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitVoiceMessage: UIView {
    var heightChanged: (() -> Void)?
    private var file: ChatAttachment
    private let chat: ChatModel?
    private let available: (() -> Bool)?
    private let waveform = UIKitVoicePosition()
    private let play = UIButton(type: .system), expand = UIButton(type: .system)
    private let time = UIKitMessageDialog.label("", size: 12)
    private let transcript = UITextView()
    private let panel = CALayer(), rule = CALayer()
    private var expanded = false, generation = 0
    init(file: ChatAttachment, chat: ChatModel?, available: (() -> Bool)?) {
        self.file = file; self.chat = chat; self.available = available; super.init(frame: .zero); Self.rows.add(self)
        panel.cornerRadius = 8; panel.backgroundColor = UIColor(CaperTheme.surface).cgColor
        panel.borderWidth = 1; panel.borderColor = UIColor(CaperTheme.border).cgColor; layer.addSublayer(panel)
        rule.backgroundColor = UIColor(CaperTheme.border).cgColor; layer.addSublayer(rule)
        play.tintColor = UIColor(CaperTheme.onAction); play.backgroundColor = UIColor(CaperTheme.action); play.layer.cornerRadius = 16
        play.addAction(UIAction { [weak self] _ in
            guard let self, !SwipeTapGuard.swiping, available?() != false else { return }; VoiceMessagePlayer.shared.toggle(self.file, chat: self.chat)
        }, for: .touchUpInside)
        expand.titleLabel?.font = UIFont(name: "Figtree-Bold", size: 12); expand.contentHorizontalAlignment = .leading
        expand.tintColor = UIColor(CaperTheme.text)
        expand.addAction(UIAction { [weak self] _ in
            guard let self else { return }; expanded.toggle(); render(); heightChanged?()
        }, for: .touchUpInside)
        waveform.seek = { [weak self] fraction in
            guard let self, available?() != false else { return }; VoiceMessagePlayer.shared.seek(self.file, chat: self.chat, fraction: fraction)
        }
        transcript.isEditable = false; transcript.isScrollEnabled = false; transcript.backgroundColor = .clear
        transcript.textContainerInset = .zero; transcript.textContainer.lineFragmentPadding = 0
        transcript.font = UIFont(name: "Figtree-Regular", size: 13) ?? .systemFont(ofSize: 13); transcript.textColor = UIColor(CaperTheme.muted)
        transcript.accessibilityIdentifier = "voice-transcript-text"
        for view in [play, waveform, time, transcript, expand] { addSubview(view) }
        NotificationCenter.default.addObserver(self, selector: #selector(motionChanged), name: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil)
        accessibilityIdentifier = "voice-message-\(file.id)"; render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { NotificationCenter.default.removeObserver(self) }
    @objc private func motionChanged() { nativeTranscriptMotion(transcript.layer, pending: file.transcript?.display == .pending && window != nil) }
    func update(_ file: ChatAttachment) { self.file = file; render() }
    override func didMoveToWindow() {
        super.didMoveToWindow(); generation += 1
        if window != nil { observe(generation) } else { transcript.layer.removeAllAnimations(); stopWhenGone() }
    }
    /// Leaving the conversation stops this note, as the SwiftUI row did on
    /// disappear. Rows are rebuilt when playback state changes, so the
    /// deferred check stops only when no row for the note is still shown.
    private static let rows = NSHashTable<UIKitVoiceMessage>.weakObjects()
    private func stopWhenGone() {
        let id = file.id
        DispatchQueue.main.async {
            guard VoiceMessagePlayer.shared.attachmentID == id,
                  !Self.rows.allObjects.contains(where: { $0.window != nil && $0.file.id == id }) else { return }
            VoiceMessagePlayer.shared.stop()
        }
    }
    private func observe(_ token: Int) {
        guard window != nil, token == generation else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
    }
    private func render() {
        let player = VoiceMessagePlayer.shared, elapsed = player.elapsed(for: file)
        play.setImage(UIImage(systemName: player.isPlaying(file) ? "pause.fill" : "play.fill"), for: .normal)
        play.accessibilityLabel = player.isPlaying(file) ? "Pause voice message" : "Play voice message"
        time.text = VoiceDuration.playerLabel(durationMs: file.durationMs, elapsedMs: elapsed)
        time.accessibilityLabel = elapsed == nil ? "Duration \(time.text ?? "")" : "\(time.text ?? "") remaining"
        waveform.file = file; waveform.setNeedsLayout()
        let text: String
        switch file.transcript?.display ?? .hidden {
        case .hidden: text = ""
        case .pending: text = VoiceTranscript.pendingLabel
        case .text(let value): text = value
        }
        if transcript.text != text { transcript.text = text }
        transcript.textContainer.maximumNumberOfLines = expanded ? 0 : VoiceTranscript.clampedLines
        transcript.textContainer.lineBreakMode = .byTruncatingTail
        expand.setTitle(expanded ? VoiceTranscript.collapseLabel : VoiceTranscript.expandLabel, for: .normal)
        expand.accessibilityValue = expanded ? "Expanded" : "Collapsed"; setNeedsLayout()
    }
    private func fullHeight(_ width: CGFloat) -> CGFloat {
        ceil((transcript.text as NSString).boundingRect(with: CGSize(width: max(1, width - 14), height: .greatestFiniteMagnitude), options: [.usesLineFragmentOrigin, .usesFontLeading], attributes: [.font: transcript.font!], context: nil).height)
    }
    func height(for width: CGFloat) -> CGFloat {
        guard !transcript.text.isEmpty else { return 56 }
        let full = fullHeight(width), limit = ceil(transcript.font!.lineHeight * CGFloat(VoiceTranscript.clampedLines))
        return 66 + (expanded ? full : min(limit, full)) + (full > limit || expanded ? 32 : 0)
    }
    override func layoutSubviews() {
        super.layoutSubviews(); let width = bounds.width
        CATransaction.begin(); CATransaction.setDisableActions(true)
        panel.frame = CGRect(x: 0, y: 0, width: width, height: 56)
        play.frame = CGRect(x: 8, y: 6, width: 44, height: 44)
        let timeWidth = max(30, time.sizeThatFits(bounds.size).width)
        time.frame = CGRect(x: width - timeWidth - 12, y: 19, width: timeWidth, height: 18)
        waveform.frame = CGRect(x: 62, y: 6, width: max(1, width - timeWidth - 84), height: 44)
        let full = fullHeight(width), limit = ceil(transcript.font!.lineHeight * CGFloat(VoiceTranscript.clampedLines)), textHeight = expanded ? full : min(limit, full)
        transcript.isHidden = transcript.text.isEmpty; rule.isHidden = transcript.isHidden
        transcript.frame = CGRect(x: 14, y: 64, width: max(1, width - 14), height: textHeight)
        expand.isHidden = transcript.isHidden || (!expanded && full <= limit)
        expand.frame = CGRect(x: 14, y: 66 + textHeight, width: max(1, width - 14), height: 32)
        rule.frame = CGRect(x: 4, y: 62, width: 2, height: max(0, height(for: width) - 62)); CATransaction.commit()
        nativeTranscriptMotion(transcript.layer, pending: file.transcript?.display == .pending && window != nil)
    }
}

@MainActor private final class UIKitVoicePosition: UIView, UIGestureRecognizerDelegate {
    var file: ChatAttachment?
    var seek: ((Double) -> Void)?
    private let waveform = NativeVoiceWaveform(playedColor: UIColor(CaperTheme.text).cgColor, unplayedColor: UIColor(CaperTheme.muted).withAlphaComponent(0.45).cgColor)
    override init(frame: CGRect) {
        super.init(frame: frame); layer.addSublayer(waveform.layer)
        let pan = UIPanGestureRecognizer(target: self, action: #selector(scrub(_:))); pan.delegate = self; addGestureRecognizer(pan)
        addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(scrub(_:))))
        isAccessibilityElement = true; accessibilityTraits = .adjustable; accessibilityLabel = "Voice message position"
    }
    convenience init() { self.init(frame: .zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard let pan = gestureRecognizer as? UIPanGestureRecognizer else { return true }
        let v = pan.velocity(in: self); return abs(v.x) > abs(v.y)
    }
    @objc private func scrub(_ gesture: UIGestureRecognizer) { seek?(VoicePlaybackPosition.fraction(x: gesture.location(in: self).x, width: bounds.width)) }
    override func layoutSubviews() {
        super.layoutSubviews(); waveform.update(file, frame: bounds.insetBy(dx: 0, dy: 8))
        if let file { accessibilityValue = "\(VoiceDuration.format(ms: VoiceMessagePlayer.shared.elapsed(for: file) ?? 0)) of \(VoiceDuration.format(ms: file.durationMs ?? 0))" }
    }
    override func accessibilityIncrement() { adjust(0.1) }
    override func accessibilityDecrement() { adjust(-0.1) }
    private func adjust(_ delta: Double) {
        guard let file else { return }; seek?(VoicePlaybackPosition.progress(elapsedMs: VoiceMessagePlayer.shared.elapsed(for: file) ?? 0, durationMs: file.durationMs) + delta)
    }
}
#endif

@MainActor private func nativeTranscriptMotion(_ layer: CALayer?, pending: Bool) {
    if pending && !CaperAccessibility.reduceMotion {
        guard layer?.animation(forKey: "pending") == nil else { return }
        let fade = CABasicAnimation(keyPath: "opacity"); fade.fromValue = 1; fade.toValue = VoiceTranscript.pendingBreathOpacity
        fade.duration = VoiceTranscript.pendingBreathSeconds / 2; fade.autoreverses = true; fade.repeatCount = .infinity
        fade.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut); layer?.add(fade, forKey: "pending")
    } else { layer?.removeAnimation(forKey: "pending") }
}
