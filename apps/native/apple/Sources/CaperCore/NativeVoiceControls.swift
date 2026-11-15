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
        record.imagePosition = .imageOnly; record.widthAnchor.constraint(equalToConstant: 36).isActive = true
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
import UIKit

@MainActor final class UIKitVoiceControls: UIStackView {
    enum Kind { case microphone, bar, notice }
    private let session: NativeVoiceSession
    private let kind: Kind
    private let timer = UIKitMessageDialog.label("")
    private let waveform = UIKitVoiceMeter()
    private var record: UIKitVoiceButton!
    private var discard: UIButton!
    private var note: UIButton!
    private var active = false, generation = 0
    private var noticeKey: String?
    init(session: NativeVoiceSession, kind: Kind) {
        self.session = session; self.kind = kind; super.init(frame: .zero)
        axis = kind == .notice ? .vertical : .horizontal; alignment = kind == .notice ? .fill : .center; spacing = 8
        record = UIKitVoiceButton(session: session)
        record.widthAnchor.constraint(equalToConstant: 36).isActive = true; record.heightAnchor.constraint(equalToConstant: 42).isActive = true
        discard = UIKitMessageDialog.button("Discard") { VoiceMessageRecorder.shared.cancel() }
        note = UIKitMessageDialog.button(VoiceMessage.asVoiceNoteLabel) { [session] in session.finish(asVoice: true) }
        if kind == .microphone { addArrangedSubview(record) }
        if kind == .bar {
            [discard!, timer, waveform, note!].forEach(addArrangedSubview)
            waveform.heightAnchor.constraint(equalToConstant: 22).isActive = true
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
        if kind == .notice {
            let key = "\(target.owner)|\(recorder.holding)|\(recorder.notice(for: target.owner) ?? "")|\(recorder.dictation?.recording.fileURL.absoluteString ?? "")|\(recorder.dictation?.error ?? "")"
            guard key != noticeKey else { return }; noticeKey = key
            arrangedSubviews.forEach { removeArrangedSubview($0); $0.removeFromSuperview() }
            if recorder.isRecording(target.owner), recorder.holding {
                addArrangedSubview(UIKitMessageDialog.label(VoiceMessage.releaseHint(sendsOnStop: VoiceMessageSettings.shared.sendOnStop)))
            }
            if let text = recorder.notice(for: target.owner) {
                let label = UIKitMessageDialog.label(text, error: true); label.accessibilityIdentifier = "\(target.identifier)-notice"; addArrangedSubview(label)
            }
            if let dictation = recorder.dictation, dictation.target.owner == target.owner {
                let label = UIKitMessageDialog.label(dictation.error.map(VoiceMessage.dictationFailed) ?? VoiceMessage.transcribing, error: dictation.error != nil)
                label.accessibilityIdentifier = "\(target.identifier)-\(dictation.error == nil ? "transcribing" : "dictation-failed")"; addArrangedSubview(label)
                let actions = UIStackView(); actions.spacing = 8; actions.axis = .vertical
                if dictation.error != nil {
                    let retry = UIKitMessageDialog.button("Retry") { VoiceTarget.transcribe(dictation.recording, for: dictation.target) }
                    retry.accessibilityIdentifier = "\(target.identifier)-retry"; actions.addArrangedSubview(retry)
                    if dictation.target.enabled {
                        let keep = UIKitMessageDialog.button(VoiceMessageSettings.shared.sendOnStop ? "Send as voice note" : "Keep as voice note") {
                            recorder.dictation = nil; Task { await VoiceTarget.keep(dictation.recording, for: dictation.target) }
                        }
                        keep.accessibilityIdentifier = "\(target.identifier)-keep"; actions.addArrangedSubview(keep)
                    }
                }
                let cancel = UIKitMessageDialog.button(dictation.error == nil ? "Cancel" : "Discard") { VoiceTarget.dropDictation() }
                cancel.accessibilityIdentifier = "\(target.identifier)-\(dictation.error == nil ? "cancel-dictation" : "discard-dictation")"
                actions.addArrangedSubview(cancel); addArrangedSubview(actions)
            }
            isHidden = arrangedSubviews.isEmpty; return
        }
        record.render()
        timer.text = VoiceDuration.format(ms: recorder.elapsedMs); waveform.update()
        discard.isHidden = recorder.holding; discard.accessibilityIdentifier = "\(target.identifier)-discard"
        discard.accessibilityLabel = recorder.dictating ? VoiceMessage.discardDictationLabel : VoiceMessage.discardLabel
        note.isHidden = !recorder.dictating; note.accessibilityIdentifier = "\(target.identifier)-as-note"
        if kind == .bar { accessibilityIdentifier = "\(target.identifier)-recording"; accessibilityLabel = recorder.dictating ? "Dictating" : "Recording voice message" }
    }
}

@MainActor private final class UIKitVoiceButton: UIButton {
    private let session: NativeVoiceSession
    private var start: CGPoint?
    private var finishing = false
    private let lock = UILabel()
    init(session: NativeVoiceSession) {
        self.session = session; super.init(frame: .zero)
        lock.text = "↑\n🔒"; lock.numberOfLines = 2; lock.textAlignment = .center
        lock.backgroundColor = UIColor(CaperTheme.raised); lock.layer.cornerRadius = 8; lock.clipsToBounds = true
        lock.isUserInteractionEnabled = false; lock.isAccessibilityElement = false
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func didMoveToWindow() { super.didMoveToWindow(); if window == nil { lock.removeFromSuperview(); session.stop() } }
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
        setImage(UIImage(systemName: stopping ? "stop.fill" : "mic"), for: .normal)
        tintColor = UIColor(recording ? CaperTheme.text : CaperTheme.muted)
        backgroundColor = recording ? UIColor(CaperTheme.terracotta) : .clear; layer.cornerRadius = 8
        accessibilityLabel = stopping ? (recorder.dictating ? VoiceMessage.doneLabel : VoiceMessage.stopLabel) : target.dictates ? VoiceMessage.dictateLabel : VoiceMessage.recordLabel
        accessibilityIdentifier = "\(target.identifier)-\(stopping ? "stop" : "record")"
        accessibilityHint = target.enabled ? VoiceMessage.dictateHint : target.unavailableReason
        if recording, recorder.holding, let window {
            if lock.superview !== window { window.addSubview(lock) }
            let rect = convert(bounds, to: window); lock.frame = CGRect(x: rect.midX - 16, y: rect.minY - 70, width: 32, height: 60)
        } else { lock.removeFromSuperview() }
    }
}

@MainActor private final class UIKitVoiceMeter: UIView {
    private var timer: Timer?
    override init(frame: CGRect) { super.init(frame: frame); isOpaque = false }
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
            UIBezierPath(roundedRect: CGRect(x: bar.x, y: bar.y, width: bar.width, height: bar.height), cornerRadius: 1.5).fill()
        }
    }
}
#endif
