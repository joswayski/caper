import Foundation
import Observation
import QuartzCore

/// The microphone and its bar share the target captured before permission or
/// transcription suspends. Navigation must never retarget a finished recording.
@MainActor final class NativeVoiceSession {
    let target: () -> VoiceTarget
    private var captured: VoiceTarget?
    private var owner: VoiceComposerID?
    private var pressed: Date?
    private var locked = false
    private var startTask: Task<Void, Never>?
    private var startID: UUID?
    private var releasedHandsFree = false
    init(target: @escaping () -> VoiceTarget) { self.target = target }
    var current: VoiceTarget { captured ?? target() }
    var enabled: Bool { target().enabled && VoiceMessageRecorder.shared.dictation?.target.owner != target().owner }
    func synchronize() {
        let next = target().owner
        if let owner, next != owner { stop() }
        owner = next
        if let captured, captured.isCurrent?() == false { stop() }
        if let captured, startTask == nil, !VoiceMessageRecorder.shared.isActive(captured.owner) { self.captured = nil }
    }
    func begin(handsFree: Bool) {
        guard enabled else { return }
        let target = target(); captured = target; owner = target.owner
        startTask?.cancel()
        let attempt = UUID(); startID = attempt; releasedHandsFree = handsFree
        startTask = Task { [weak self] in
            guard !Task.isCancelled, self?.startID == attempt, target.isCurrent?() != false else { return }
            let started = await VoiceMessageRecorder.shared.start(for: target.owner,
                handsFree: self?.releasedHandsFree == true, dictating: target.dictates) {
                Task { @MainActor in await VoiceTarget.finish(target) }
            }
            guard !Task.isCancelled, let self, self.startID == attempt else { return }
            self.startTask = nil
            guard target.isCurrent?() != false else { self.stop(); return }
            if started, self.releasedHandsFree { self.handsFree() }
        }
    }
    func finish(asVoice: Bool = false) { let target = current; Task { await VoiceTarget.finish(target, asVoice: asVoice) } }
    func stop() {
        startTask?.cancel(); startTask = nil; startID = nil; releasedHandsFree = false
        pressed = nil; locked = false
        if let owner, VoiceMessageRecorder.shared.isActive(owner) { VoiceMessageRecorder.shared.cancel() }
        captured = nil
    }
    func touchDown() {
        guard enabled else { return }
        let start = Date(); pressed = start; locked = false; begin(handsFree: false)
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(VoiceMessage.tapThreshold))
            guard let self, self.pressed == start, !self.locked else { return }
            let recorder = VoiceMessageRecorder.shared
            if recorder.isActive(self.current.owner), !recorder.handsFree {
                #if os(iOS)
                VoiceMessageRecorder.haptic()
                #endif
                recorder.holding = true; recorder.dictating = false
            }
        }
    }
    func drag(_ dy: Double) {
        let recorder = VoiceMessageRecorder.shared
        guard pressed != nil, !locked, recorder.isActive(current.owner), !recorder.handsFree else { return }
        recorder.lift = min(0, dy)
        if VoiceMessage.locksBySliding(translation: dy) {
            locked = true; recorder.dictating = false; handsFree()
            #if os(iOS)
            VoiceMessageRecorder.haptic()
            #endif
        }
    }
    func release(interrupted: Bool = false) {
        guard let start = pressed else { return }
        pressed = nil; let wasLocked = locked; locked = false
        let recorder = VoiceMessageRecorder.shared; recorder.lift = 0
        guard startTask != nil || recorder.isActive(current.owner) else { return }
        if interrupted { handsFree(); return }
        switch VoiceMessage.release(pressDuration: Date().timeIntervalSince(start), recordedMs: recorder.isRecording(current.owner) ? recorder.elapsedMs : 0, locked: wasLocked) {
        case .discard: stop()
        case .handsFree: handsFree()
        case .stop: finish(asVoice: true)
        }
    }
    private func handsFree() {
        releasedHandsFree = true
        let recorder = VoiceMessageRecorder.shared
        recorder.handsFree = true; recorder.holding = false; recorder.lift = 0
        #if os(iOS)
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        #endif
    }
}

#if os(macOS)
import AppKit

@MainActor final class AppKitVoiceControls: NSStackView {
    enum Kind { case microphone, bar, notice }
    private let session: NativeVoiceSession
    private let kind: Kind
    private let observation = AppKitObservation()
    private var noticeKey: String?
    private let timer = AppKitKit.label("", size: 12)
    private let waveform = AppKitVoiceMeter()
    private var record: AppKitKitButton!
    private var discard: AppKitKitButton!
    private var note: AppKitKitButton!
    private var stopButton: AppKitKitButton!
    init(session: NativeVoiceSession, kind: Kind) {
        self.session = session; self.kind = kind; super.init(frame: .zero)
        orientation = kind == .notice ? .vertical : .horizontal; alignment = kind == .notice ? .leading : .centerY; spacing = 8
        record = AppKitKitButton("", kind: .plain) { [session] in
            if VoiceMessageRecorder.shared.isActive(session.current.owner) { session.finish() } else { session.begin(handsFree: true) }
        }
        record.image = NSImage(systemSymbolName: "mic", accessibilityDescription: VoiceMessage.dictateLabel)
        record.imagePosition = .imageOnly; record.minimumHeight = 32
        record.widthAnchor.constraint(equalToConstant: 32).isActive = true
        record.heightAnchor.constraint(equalToConstant: 32).isActive = true
        discard = AppKitKitButton("Discard", kind: .plain) { VoiceMessageRecorder.shared.cancel() }
        discard.keyEquivalent = "\u{1b}"
        note = AppKitKitButton(VoiceMessage.asVoiceNoteLabel) { [session] in session.finish(asVoice: true) }
        stopButton = AppKitKitButton("Done", kind: .primary) { [session] in session.finish() }; stopButton.keyEquivalent = "\r"
        if kind == .microphone { addArrangedSubview(record) }
        if kind == .bar {
            [discard!, timer, waveform, note!, stopButton!].forEach(addArrangedSubview)
            waveform.widthAnchor.constraint(greaterThanOrEqualToConstant: 24).isActive = true
            waveform.heightAnchor.constraint(equalToConstant: 22).isActive = true
        }
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) }
        else { observation.stop(); if kind == .microphone { session.stop() } }
    }
    override func layout() {
        if kind == .bar { orientation = bounds.width < 300 ? .vertical : .horizontal }
        super.layout()
    }
    override var acceptsFirstResponder: Bool { kind == .bar }
    override func keyDown(with event: NSEvent) {
        if kind == .bar, event.modifierFlags.intersection([.command, .control, .option, .shift]).isEmpty {
            if event.keyCode == 53 { VoiceMessageRecorder.shared.cancel(); return }
            if event.keyCode == 36 || event.keyCode == 76 { session.finish(); return }
        }
        super.keyDown(with: event)
    }
    func render() {
        session.synchronize()
        let target = session.current, recorder = VoiceMessageRecorder.shared
        if kind == .notice {
            let key = "\(target.owner)|\(recorder.notice(for: target.owner) ?? "")|\(recorder.dictation?.recording.fileURL.absoluteString ?? "")|\(recorder.dictation?.error ?? "")"
            guard key != noticeKey else { return }; noticeKey = key
            arrangedSubviews.forEach { $0.removeFromSuperview() }
            if let text = recorder.notice(for: target.owner) {
                let label = AppKitKit.label(text, size: 11, color: AppKitKit.error); label.setAccessibilityIdentifier("\(target.identifier)-notice"); addArrangedSubview(label)
            }
            if let dictation = recorder.dictation, dictation.target.owner == target.owner {
                let text = dictation.error.map(VoiceMessage.dictationFailed) ?? VoiceMessage.transcribing
                let label = AppKitKit.label(text, size: 11, color: dictation.error == nil ? AppKitKit.muted : AppKitKit.error)
                label.setAccessibilityIdentifier("\(target.identifier)-\(dictation.error == nil ? "transcribing" : "dictation-failed")")
                addArrangedSubview(label)
                let actions = NSStackView(); actions.spacing = 8
                if dictation.error != nil {
                    let retry = AppKitKitButton("Retry") { VoiceTarget.transcribe(dictation.recording, for: dictation.target) }
                    retry.setAccessibilityIdentifier("\(target.identifier)-retry"); actions.addArrangedSubview(retry)
                    if dictation.target.enabled {
                        let keep = AppKitKitButton(VoiceMessageSettings.shared.sendOnStop ? "Send as voice note" : "Keep as voice note") {
                            recorder.dictation = nil; Task { await VoiceTarget.keep(dictation.recording, for: dictation.target) }
                        }
                        keep.setAccessibilityIdentifier("\(target.identifier)-keep"); actions.addArrangedSubview(keep)
                    }
                }
                let cancel = AppKitKitButton(dictation.error == nil ? "Cancel" : "Discard") { VoiceTarget.dropDictation() }
                cancel.setAccessibilityIdentifier("\(target.identifier)-\(dictation.error == nil ? "cancel-dictation" : "discard-dictation")")
                actions.addArrangedSubview(cancel); addArrangedSubview(actions)
            }
            isHidden = arrangedSubviews.isEmpty
            return
        }
        let label = target.dictates ? VoiceMessage.dictateLabel : VoiceMessage.recordLabel
        record.isEnabled = session.enabled; record.setAccessibilityLabel(label); record.toolTip = target.enabled ? label : target.unavailableReason
        record.setAccessibilityIdentifier("\(target.identifier)-record")
        timer.stringValue = VoiceDuration.format(ms: recorder.elapsedMs)
        timer.setAccessibilityLabel(timer.stringValue)
        waveform.update(); note.isHidden = !recorder.dictating
        discard.setAccessibilityIdentifier("\(target.identifier)-discard")
        discard.setAccessibilityLabel(recorder.dictating ? VoiceMessage.discardDictationLabel : VoiceMessage.discardLabel)
        note.setAccessibilityIdentifier("\(target.identifier)-as-note")
        stopButton.title = recorder.dictating ? "Done" : "Stop"
        stopButton.setAccessibilityLabel(recorder.dictating ? VoiceMessage.doneLabel : VoiceMessage.stopLabel)
        stopButton.setAccessibilityIdentifier("\(target.identifier)-stop")
        if kind == .bar { setAccessibilityIdentifier("\(target.identifier)-recording"); setAccessibilityLabel(recorder.dictating ? "Dictating" : "Recording voice message") }
    }
}

@MainActor private final class AppKitVoiceMeter: NSView {
    private var timer: Timer?
    isolated deinit { timer?.invalidate() }
    override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); update() }
    func update() {
        let animating = window != nil && !CaperAccessibility.reduceMotion && VoiceMessageRecorder.shared.phase == .recording
        if animating, timer == nil {
            timer = Timer.scheduledTimer(withTimeInterval: 1 / 30, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.needsDisplay = true }
            }
        } else if !animating { timer?.invalidate(); timer = nil }
        needsDisplay = true
    }
    override func draw(_ dirtyRect: NSRect) {
        let recorder = VoiceMessageRecorder.shared
        let scroll = CaperAccessibility.reduceMotion ? 0 : VoiceWaveform.scrollFraction(sinceLastBar: Date().timeIntervalSince(recorder.lastBarAt))
        for bar in VoiceLiveMeter.layout(recorder.levels, pending: recorder.pendingLevel, width: bounds.width, height: bounds.height, scroll: scroll) {
            let color = VoiceLiveMeter.color(brightness: bar.brightness)
            NSColor(srgbRed: color.red, green: color.green, blue: color.blue, alpha: bar.alpha).setFill()
            NSBezierPath(roundedRect: NSRect(x: bar.x, y: bar.y, width: bar.width, height: bar.height), xRadius: 1.5, yRadius: 1.5).fill()
        }
    }
}
#elseif os(iOS)
import CoreText
import UIKit

/// The phone recorder. While recording, the bar covers the field:
/// `[trash]  ● 0:07  ⋯live⋯  [≋ Voice note]` with the microphone beside it
/// turned into a chalk Done (✓, dictating) or Stop (■) once the finger lifts,
/// pulsing once a second. While held, the microphone is live green under the
/// finger, Discard steps out and a lock rises above it. Above the composer,
/// a dictation that is transcribing or failed is one compact card.
@MainActor final class UIKitVoiceControls: UIStackView {
    enum Kind { case microphone, bar, notice }
    private let session: NativeVoiceSession
    private let kind: Kind
    private let dot = UIView()
    private let timer = UILabel()
    private let waveform = UIKitVoiceMeter()
    private var record: UIKitVoiceButton!
    private var discard: UIButton!
    private var note: UIButton!
    private var active = false, generation = 0
    private var noticeKey: String?
    init(session: NativeVoiceSession, kind: Kind) {
        self.session = session; self.kind = kind; super.init(frame: .zero)
        axis = kind == .notice ? .vertical : .horizontal; alignment = kind == .notice ? .fill : .center
        spacing = kind == .notice ? 8 : 10
        record = UIKitVoiceButton(session: session)
        record.widthAnchor.constraint(equalToConstant: 36).isActive = true; record.heightAnchor.constraint(equalToConstant: 42).isActive = true
        discard = Self.iconButton("trash-2", size: 18, tint: UIColor(CaperTheme.muted)) { VoiceMessageRecorder.shared.cancel() }
        note = Self.pill(VoiceMessage.asVoiceNoteLabel, icon: "audio-lines", height: 30, scaled: false) { [session] in session.finish(asVoice: true) }
        if kind == .microphone { addArrangedSubview(record) }
        if kind == .bar {
            dot.backgroundColor = UIColor(CaperTheme.terracottaBright); dot.layer.cornerRadius = 4
            dot.widthAnchor.constraint(equalToConstant: 8).isActive = true; dot.heightAnchor.constraint(equalToConstant: 8).isActive = true
            dot.isAccessibilityElement = false
            let base = UIFont(name: "Figtree-Medium", size: 15) ?? .systemFont(ofSize: 15, weight: .medium)
            timer.font = UIFont(descriptor: base.fontDescriptor.addingAttributes([.featureSettings: [[
                UIFontDescriptor.FeatureKey.type: kNumberSpacingType, UIFontDescriptor.FeatureKey.selector: kMonospacedNumbersSelector,
            ]]]), size: 15)
            timer.textColor = UIColor(CaperTheme.text)
            timer.setContentHuggingPriority(.required, for: .horizontal)
            timer.setContentCompressionResistancePriority(.required, for: .horizontal)
            note.setContentHuggingPriority(.required, for: .horizontal)
            note.setContentCompressionResistancePriority(.required, for: .horizontal)
            [discard!, dot, timer, waveform, note!].forEach(addArrangedSubview)
            setCustomSpacing(2, after: discard); setCustomSpacing(7, after: dot)
            isLayoutMarginsRelativeArrangement = true
            directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 4)
            waveform.heightAnchor.constraint(equalToConstant: 24).isActive = true
            waveform.widthAnchor.constraint(greaterThanOrEqualToConstant: 20).isActive = true
        }
        render()
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func didMoveToWindow() {
        super.didMoveToWindow(); active = window != nil; generation += 1
        if active { observe(generation) } else if kind == .microphone { session.stop() }
    }
    private func observe(_ token: Int) {
        guard active, token == generation else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor [weak self] in self?.observe(token) } }
    }
    func render() {
        session.synchronize()
        let target = session.current, recorder = VoiceMessageRecorder.shared
        if kind == .notice { renderNotice(target: target, recorder: recorder); return }
        record.render()
        timer.text = VoiceDuration.format(ms: recorder.elapsedMs); waveform.update()
        timer.accessibilityLabel = "Recording, \(timer.text ?? "")"
        // Breathes while the microphone is live; steady under Reduce Motion.
        let live = recorder.phase == .recording && !CaperAccessibility.reduceMotion
        if live, dot.layer.animation(forKey: "breathe") == nil {
            let breathe = CABasicAnimation(keyPath: "opacity")
            breathe.fromValue = 1; breathe.toValue = 0.35; breathe.duration = 0.9
            breathe.autoreverses = true; breathe.repeatCount = .infinity
            breathe.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            dot.layer.add(breathe, forKey: "breathe")
        } else if !live { dot.layer.removeAnimation(forKey: "breathe") }
        // While held the finger is the control: releasing stops, so Discard
        // keeps its place but steps out.
        discard.alpha = recorder.holding ? 0 : 1; discard.isUserInteractionEnabled = !recorder.holding
        discard.accessibilityElementsHidden = recorder.holding
        discard.accessibilityIdentifier = "\(target.identifier)-discard"
        discard.accessibilityLabel = recorder.dictating ? VoiceMessage.discardDictationLabel : VoiceMessage.discardLabel
        if note.isHidden == recorder.dictating { note.isHidden = !recorder.dictating }
        note.accessibilityIdentifier = "\(target.identifier)-as-note"
        note.accessibilityHint = VoiceMessageSettings.shared.sendOnStop ? "Stops and sends as a voice message" : "Stops and keeps it as a voice message"
        if kind == .bar { accessibilityIdentifier = "\(target.identifier)-recording"; accessibilityLabel = recorder.dictating ? "Dictating" : "Recording voice message" }
    }

    private func renderNotice(target: VoiceTarget, recorder: VoiceMessageRecorder) {
        let holding = recorder.isRecording(target.owner) && recorder.holding
        let key = "\(target.owner)|\(holding)|\(recorder.notice(for: target.owner) ?? "")|\(recorder.dictation?.recording.fileURL.absoluteString ?? "")|\(recorder.dictation?.error ?? "")"
        guard key != noticeKey else { return }; noticeKey = key
        arrangedSubviews.forEach { removeArrangedSubview($0); $0.removeFromSuperview() }
        if holding {
            let hint = Self.text(VoiceMessage.releaseHint(sendsOnStop: VoiceMessageSettings.shared.sendOnStop), size: 12, weight: "Figtree-Bold", color: UIColor(CaperTheme.muted))
            hint.textAlignment = .center; hint.isAccessibilityElement = false
            addArrangedSubview(hint)
        }
        if let text = recorder.notice(for: target.owner) {
            let label = Self.text(text, size: 12, color: UIColor(CaperTheme.error)); label.accessibilityIdentifier = "\(target.identifier)-notice"; addArrangedSubview(label)
        }
        if let dictation = recorder.dictation, dictation.target.owner == target.owner {
            addArrangedSubview(dictation.error == nil ? transcribingCard(dictation, identifier: target.identifier)
                : failedCard(dictation, identifier: target.identifier, recorder: recorder))
        }
        isHidden = arrangedSubviews.isEmpty
    }

    /// `(spinner) Transcribing…   [Cancel]`
    private func transcribingCard(_ dictation: VoiceDictation, identifier: String) -> UIView {
        let spinner = UIActivityIndicatorView(style: .medium); spinner.color = UIColor(CaperTheme.muted); spinner.startAnimating()
        let label = Self.text(VoiceMessage.transcribing, size: 13, color: UIColor(CaperTheme.muted))
        label.accessibilityIdentifier = "\(identifier)-transcribing"
        let cancel = Self.pill("Cancel", height: 32) { VoiceTarget.dropDictation() }
        cancel.accessibilityIdentifier = "\(identifier)-cancel-dictation"
        cancel.setContentHuggingPriority(.required, for: .horizontal)
        let row = UIStackView(arrangedSubviews: [spinner, label, cancel]); row.spacing = 10; row.alignment = .center
        return Self.card(row)
    }

    /// `(!) Couldn’t insert your words / No words heard.` over
    /// `[≋ Keep as voice note] [↻ Retry]  [trash]`. Keeping the recording is
    /// the chalk primary; Retry and Discard stay neutral.
    private func failedCard(_ dictation: VoiceDictation, identifier: String, recorder: VoiceMessageRecorder) -> UIView {
        let badge = UIView(); badge.backgroundColor = UIColor(CaperTheme.error).withAlphaComponent(0.14); badge.layer.cornerRadius = 15
        let mark = UIImageView(image: UIImage(systemName: "exclamationmark", withConfiguration: UIImage.SymbolConfiguration(pointSize: 13, weight: .bold)))
        mark.tintColor = UIColor(CaperTheme.error); mark.translatesAutoresizingMaskIntoConstraints = false
        badge.addSubview(mark); badge.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            badge.widthAnchor.constraint(equalToConstant: 30), badge.heightAnchor.constraint(equalToConstant: 30),
            mark.centerXAnchor.constraint(equalTo: badge.centerXAnchor), mark.centerYAnchor.constraint(equalTo: badge.centerYAnchor),
        ])
        let title = Self.text("Couldn’t insert your words", size: 13, weight: "Figtree-Bold", color: UIColor(CaperTheme.text))
        let reason = Self.text(dictation.error ?? "", size: 12, color: UIColor(CaperTheme.muted))
        let words = UIStackView(arrangedSubviews: [title, reason]); words.axis = .vertical; words.spacing = 2
        words.isAccessibilityElement = true; words.accessibilityLabel = VoiceMessage.dictationFailed(dictation.error ?? "")
        words.accessibilityIdentifier = "\(identifier)-dictation-failed"
        let header = UIStackView(arrangedSubviews: [badge, words]); header.spacing = 10; header.alignment = .center

        var buttons: [UIView] = []
        if dictation.target.enabled {
            let keep = Self.pill(VoiceMessageSettings.shared.sendOnStop ? "Send as voice note" : "Keep as voice note", icon: "audio-lines", primary: true) {
                recorder.dictation = nil; Task { await VoiceTarget.keep(dictation.recording, for: dictation.target) }
            }
            keep.accessibilityIdentifier = "\(identifier)-keep"; buttons.append(keep)
        }
        let retry = Self.pill("Retry", icon: "rotate-ccw") { VoiceTarget.transcribe(dictation.recording, for: dictation.target) }
        retry.accessibilityIdentifier = "\(identifier)-retry"; buttons.append(retry)
        let discard = Self.iconButton("trash-2", size: 16, tint: UIColor(CaperTheme.muted), side: 36, bordered: true) { VoiceTarget.dropDictation() }
        discard.accessibilityLabel = "Discard"; discard.accessibilityIdentifier = "\(identifier)-discard-dictation"
        let spacer = UIView(); spacer.setContentHuggingPriority(.fittingSizeLevel, for: .horizontal)
        let actions = UIStackView(arrangedSubviews: buttons + [spacer, discard]); actions.spacing = 8; actions.alignment = .center
        // At accessibility text sizes the actions stack instead of clipping.
        if traitCollection.preferredContentSizeCategory.isAccessibilityCategory { actions.axis = .vertical; actions.alignment = .leading; spacer.isHidden = true }

        let body = UIStackView(arrangedSubviews: [header, actions]); body.axis = .vertical; body.spacing = 12
        return Self.card(body)
    }

    /// Raised, 16pt concentric corners and a hairline, as the composer's other cards.
    private static func card(_ content: UIView) -> UIView {
        let card = UIView()
        card.backgroundColor = UIColor(CaperTheme.raised); card.layer.cornerRadius = 16; card.layer.cornerCurve = .continuous
        card.layer.borderWidth = 1; card.layer.borderColor = UIColor(CaperTheme.borderSubtle).cgColor
        content.translatesAutoresizingMaskIntoConstraints = false; card.addSubview(content)
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: card.topAnchor, constant: 12),
            content.bottomAnchor.constraint(equalTo: card.bottomAnchor, constant: -12),
            content.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 12),
            content.trailingAnchor.constraint(equalTo: card.trailingAnchor, constant: -12),
        ])
        return card
    }

    nonisolated static func icon(_ name: String, size: CGFloat) -> UIImage? {
        guard let image = UIImage(named: "caper-\(name)", in: caperResourceBundle, compatibleWith: nil) else { return nil }
        return UIGraphicsImageRenderer(size: CGSize(width: size, height: size)).image { _ in
            image.draw(in: CGRect(x: 0, y: 0, width: size, height: size))
        }.withRenderingMode(.alwaysTemplate)
    }

    private static func text(_ text: String, size: CGFloat, weight: String = "Figtree-Regular", color: UIColor) -> UILabel {
        let label = UILabel(); label.text = text; label.numberOfLines = 0
        label.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: weight, size: size) ?? .systemFont(ofSize: size))
        label.adjustsFontForContentSizeCategory = true
        label.textColor = color
        return label
    }

    /// A capsule: chalk with obsidian ink when primary, otherwise a hairline
    /// over the raised surface. The recording bar's own pill keeps a fixed
    /// size so the 42pt bar never grows.
    private static func pill(_ title: String, icon: String? = nil, primary: Bool = false, height: CGFloat = 36, scaled: Bool = true,
                             action: @escaping () -> Void) -> UIButton {
        var config = UIButton.Configuration.plain()
        config.title = title; config.cornerStyle = .capsule
        if let icon { config.image = Self.icon(icon, size: 14); config.imagePadding = 6 }
        config.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: icon == nil ? 14 : 12, bottom: 0, trailing: 14)
        config.titleLineBreakMode = .byTruncatingTail
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
            var attributes = incoming
            let font = UIFont(name: "Figtree-Bold", size: 13) ?? .systemFont(ofSize: 13, weight: .bold)
            attributes.font = scaled ? UIFontMetrics(forTextStyle: .body).scaledFont(for: font) : font
            return attributes
        }
        let button = UIButton(configuration: config, primaryAction: UIAction { _ in action() })
        button.configurationUpdateHandler = { button in
            guard var config = button.configuration else { return }
            let pressed = button.isHighlighted
            if primary {
                config.background.backgroundColor = UIColor(pressed ? CaperTheme.actionPress : CaperTheme.action)
                config.baseForegroundColor = UIColor(CaperTheme.onAction)
                config.background.strokeWidth = 0
            } else {
                config.background.backgroundColor = pressed ? UIColor.white.withAlphaComponent(0.08) : UIColor(CaperTheme.raised)
                config.baseForegroundColor = UIColor(button.isEnabled ? CaperTheme.text : CaperTheme.muted)
                config.background.strokeColor = UIColor(CaperTheme.border); config.background.strokeWidth = 1
            }
            button.configuration = config
        }
        button.heightAnchor.constraint(greaterThanOrEqualToConstant: height).isActive = true
        if !scaled { button.heightAnchor.constraint(equalToConstant: height).isActive = true }
        return button
    }

    private static func iconButton(_ icon: String, size: CGFloat, tint: UIColor, side: CGFloat = 36, bordered: Bool = false,
                                   action: @escaping () -> Void) -> UIButton {
        var config = UIButton.Configuration.plain()
        config.image = Self.icon(icon, size: size); config.cornerStyle = .capsule
        config.contentInsets = .zero
        let button = UIButton(configuration: config, primaryAction: UIAction { _ in action() })
        button.configurationUpdateHandler = { button in
            guard var config = button.configuration else { return }
            config.baseForegroundColor = button.isHighlighted ? UIColor(CaperTheme.text) : tint
            config.background.backgroundColor = button.isHighlighted ? UIColor.white.withAlphaComponent(0.08) : bordered ? UIColor(CaperTheme.raised) : .clear
            config.background.strokeColor = bordered ? UIColor(CaperTheme.border) : .clear; config.background.strokeWidth = bordered ? 1 : 0
            button.configuration = config
        }
        button.widthAnchor.constraint(equalToConstant: side).isActive = true; button.heightAnchor.constraint(equalToConstant: side).isActive = true
        return button
    }
}

/// The composer's microphone: tap to dictate, hold for a voice message,
/// slide up to lock. Held, it is a live green circle under the finger with
/// the lock above it; hands-free, a chalk circle with Done (✓) or Stop (■)
/// and a ring that grows and fades from it every second.
@MainActor private final class UIKitVoiceButton: UIButton {
    private let session: NativeVoiceSession
    private var start: CGPoint?
    private var finishing = false
    private let face = CALayer()
    private let ring = CAShapeLayer()
    private let lock = UIKitVoiceLock(frame: .zero)
    private static let side: CGFloat = 32
    private static let microphone = UIKitVoiceControls.icon("mic", size: 18)
    private static let check = UIKitVoiceControls.icon("check", size: 17)
    private static let stopSquare = UIGraphicsImageRenderer(size: CGSize(width: 11, height: 11)).image { _ in
        UIBezierPath(roundedRect: CGRect(x: 0, y: 0, width: 11, height: 11), cornerRadius: 2.5).fill()
    }.withRenderingMode(.alwaysTemplate)
    init(session: NativeVoiceSession) {
        self.session = session; super.init(frame: .zero)
        face.cornerRadius = Self.side / 2
        ring.fillColor = UIColor.clear.cgColor; ring.lineWidth = 2; ring.opacity = 0
        ring.strokeColor = UIColor(CaperTheme.terracottaBright).cgColor
        layer.insertSublayer(face, at: 0); layer.insertSublayer(ring, at: 0)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func didMoveToWindow() { super.didMoveToWindow(); if window == nil { lock.removeFromSuperview(); session.stop() } }
    override func layoutSubviews() {
        super.layoutSubviews()
        let rect = CGRect(x: bounds.midX - Self.side / 2, y: bounds.midY - Self.side / 2, width: Self.side, height: Self.side)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        face.frame = rect; ring.frame = rect
        ring.path = UIBezierPath(ovalIn: CGRect(x: 0, y: 0, width: Self.side, height: Self.side)).cgPath
        CATransaction.commit()
    }
    override func beginTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        let recorder = VoiceMessageRecorder.shared
        finishing = recorder.isActive(session.current.owner) && recorder.handsFree
        if !finishing { start = touch.location(in: window); session.touchDown() }
        return true
    }
    override func continueTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        if let start { session.drag(Double(touch.location(in: window).y - start.y)) }
        return true
    }
    override func endTracking(_ touch: UITouch?, with event: UIEvent?) {
        if finishing { session.finish() } else { session.release() }
        start = nil; finishing = false
    }
    override func cancelTracking(with event: UIEvent?) { session.release(interrupted: true); start = nil; finishing = false }
    override func accessibilityActivate() -> Bool {
        if VoiceMessageRecorder.shared.isActive(session.current.owner) { session.finish() } else { session.begin(handsFree: true) }
        return true
    }
    func render() {
        let recorder = VoiceMessageRecorder.shared, target = session.current
        let recording = recorder.isActive(target.owner), stopping = recording && recorder.handsFree
        isEnabled = recording || session.enabled
        alpha = isEnabled ? 1 : 0.45
        setImage(stopping ? (recorder.dictating ? Self.check : Self.stopSquare) : Self.microphone, for: .normal)
        tintColor = UIColor(stopping ? CaperTheme.onAction : recording ? CaperTheme.text : CaperTheme.muted)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        face.backgroundColor = (stopping ? UIColor(CaperTheme.action) : recording ? UIColor(CaperTheme.terracotta) : .clear).cgColor
        CATransaction.commit()
        let pulsing = stopping && recorder.phase == .recording && !CaperAccessibility.reduceMotion
        if pulsing, ring.animation(forKey: "pulse") == nil {
            let grow = CABasicAnimation(keyPath: "transform.scale"); grow.fromValue = 1; grow.toValue = 1.45
            let fade = CABasicAnimation(keyPath: "opacity"); fade.fromValue = 0.55; fade.toValue = 0
            let pulse = CAAnimationGroup(); pulse.animations = [grow, fade]; pulse.duration = 1; pulse.repeatCount = .infinity
            pulse.timingFunction = CAMediaTimingFunction(controlPoints: 0.2, 0.8, 0.2, 1)
            ring.add(pulse, forKey: "pulse")
        } else if !pulsing { ring.removeAnimation(forKey: "pulse") }
        accessibilityLabel = stopping ? (recorder.dictating ? VoiceMessage.doneLabel : VoiceMessage.stopLabel) : target.dictates ? VoiceMessage.dictateLabel : VoiceMessage.recordLabel
        accessibilityIdentifier = "\(target.identifier)-\(stopping ? "stop" : "record")"
        accessibilityHint = target.enabled ? VoiceMessage.dictateHint : target.unavailableReason
        if recording, recorder.holding, let window {
            if lock.superview !== window { window.addSubview(lock) }
            let rect = convert(bounds, to: window)
            lock.frame = CGRect(x: rect.midX - UIKitVoiceLock.size.width / 2, y: rect.minY - 8 - UIKitVoiceLock.size.height,
                                width: UIKitVoiceLock.size.width, height: UIKitVoiceLock.size.height)
            lock.show(progress: VoiceMessage.lockProgress(translation: recorder.lift))
        } else { lock.removeFromSuperview() }
    }
}

/// Above a held microphone: a lock over an arrow in a raised capsule.
/// Sliding up lifts the lock and fades the arrow; reaching it locks.
@MainActor private final class UIKitVoiceLock: UIView {
    static let size = CGSize(width: 34, height: 64)
    private let padlock = UIImageView(image: UIKitVoiceControls.icon("lock", size: 15))
    private let arrow = UIImageView(image: UIKitVoiceControls.icon("chevron-up", size: 14))
    override init(frame: CGRect) {
        super.init(frame: CGRect(origin: .zero, size: Self.size))
        backgroundColor = UIColor(CaperTheme.raised); layer.cornerRadius = Self.size.width / 2; layer.cornerCurve = .continuous
        layer.borderWidth = 1; layer.borderColor = UIColor(CaperTheme.border).cgColor
        padlock.tintColor = UIColor(CaperTheme.text); arrow.tintColor = UIColor(CaperTheme.muted)
        addSubview(padlock); addSubview(arrow)
        isUserInteractionEnabled = false; isAccessibilityElement = false; accessibilityElementsHidden = true
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func layoutSubviews() {
        super.layoutSubviews()
        padlock.bounds = CGRect(x: 0, y: 0, width: 15, height: 15); padlock.center = CGPoint(x: bounds.midX, y: 20)
        arrow.bounds = CGRect(x: 0, y: 0, width: 14, height: 14); arrow.center = CGPoint(x: bounds.midX, y: 44)
    }
    func show(progress: Double) {
        padlock.transform = CGAffineTransform(translationX: 0, y: -6 * progress)
        arrow.alpha = 1 - progress
    }
}

@MainActor private final class UIKitVoiceMeter: UIView {
    private var timer: Timer?
    override init(frame: CGRect) { super.init(frame: frame); isOpaque = false; isAccessibilityElement = false }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { timer?.invalidate() }
    override func didMoveToWindow() { super.didMoveToWindow(); update() }
    func update() {
        let animating = window != nil && !CaperAccessibility.reduceMotion && VoiceMessageRecorder.shared.phase == .recording
        if animating, timer == nil {
            timer = Timer.scheduledTimer(withTimeInterval: 1 / 30, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.setNeedsDisplay() }
            }
        } else if !animating { timer?.invalidate(); timer = nil }
        setNeedsDisplay()
    }
    override func draw(_ rect: CGRect) {
        let recorder = VoiceMessageRecorder.shared
        let scroll = CaperAccessibility.reduceMotion ? 0 : VoiceWaveform.scrollFraction(sinceLastBar: Date().timeIntervalSince(recorder.lastBarAt))
        for bar in VoiceLiveMeter.layout(recorder.levels, pending: recorder.pendingLevel, width: bounds.width, height: bounds.height, scroll: scroll) {
            let color = VoiceLiveMeter.color(brightness: bar.brightness)
            UIColor(red: color.red, green: color.green, blue: color.blue, alpha: bar.alpha).setFill()
            let rect = CGRect(x: bar.x, y: bar.y, width: bar.width, height: bar.height)
            UIBezierPath(roundedRect: rect, cornerRadius: min(rect.width, rect.height) / 2).fill()
        }
    }
}
#endif
