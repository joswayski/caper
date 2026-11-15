import Foundation
import Observation
import QuartzCore

@MainActor @Observable final class NativeAudioTest {
    let voice: VoiceClient
    #if os(macOS)
    let microphone = MacMicrophoneTest()
    #else
    let microphone = IOSMicrophoneTest()
    #endif
    let speaker = SpeakerTest()
    private var starting: Task<Void, Never>?
    private(set) var preparing = false
    private var phase: VoiceClient.Phase
    var routeError: String?
    let recordedPreview = CaperRuntime.isAudioPreview("audio-recorded")
    init(voice: VoiceClient) { self.voice = voice; phase = voice.phase }
    var canRecord: Bool { !recordedPreview && !preparing && (microphone.recording || [.idle, .failed, .connected].contains(voice.phase)) }
    func sync() {
        if phase != voice.phase {
            phase = voice.phase
            if phase != .idle && phase != .failed { microphone.close() }
        }
        speaker.setGain(voice.outputGain)
    }
    func record() {
        guard canRecord else { return }
        if microphone.recording { microphone.stopRecording(); return }
        preparing = true
        starting = Task { [weak self, microphone, voice] in
            await microphone.start(voice: voice)
            guard !Task.isCancelled else { microphone.close(); return }
            self?.preparing = false; self?.starting = nil
        }
    }
    func close() { starting?.cancel(); starting = nil; preparing = false; microphone.close(); speaker.stop() }
    #if os(macOS)
    func select(_ id: String, input: Bool) {
        if input { routeError = voice.selectInput(id) ? nil : "Could not switch microphone. The previous route is still selected." }
        else { speaker.stop(); routeError = voice.selectOutput(id) ? nil : "Could not switch output. The previous route is still selected." }
    }
    #endif
}

@MainActor private final class AudioTestMeter {
    let shape = CAShapeLayer()
    private var levels = Array(repeating: Float(0), count: 40)
    func tick(_ state: NativeAudioTest, bounds: CGRect, active: CGColor, idle: CGColor) {
        levels = state.microphone.recording ? Array(levels.dropFirst()) + [min(1, sqrt(state.microphone.level) * 2.5)] : Array(repeating: 0, count: 40)
        let path = CGMutablePath(), step = min(5, bounds.width / 40)
        for (index, level) in levels.enumerated() {
            let height = 3 + CGFloat((level * 8).rounded()) * 4
            path.addRoundedRect(in: CGRect(x: CGFloat(index) * step, y: (bounds.height - height) / 2, width: max(1, step - 2), height: height), cornerWidth: 1.5, cornerHeight: 1.5)
        }
        CATransaction.begin(); CATransaction.setDisableActions(true); shape.frame = bounds
        shape.path = path; shape.fillColor = state.microphone.recording ? active : idle; CATransaction.commit()
    }
}

#if os(macOS)
import AppKit

@MainActor final class AppKitAudioTest: AppKitSettingsDialog, NSPopoverDelegate {
    private let state: NativeAudioTest
    private lazy var input = AppKitKitButton("") { [weak self] in self?.choose(input: true) }
    private lazy var output = AppKitKitButton("") { [weak self] in self?.choose(input: false) }
    private let inputGain = NSSlider(value: 100, minValue: 0, maxValue: 200, target: nil, action: nil)
    private let outputGain = NSSlider(value: 100, minValue: 0, maxValue: 200, target: nil, action: nil)
    private let strength = NSSlider(value: 25, minValue: 0, maxValue: 100, target: nil, action: nil)
    private let inputLabel = AppKitKit.label("", size: 12), outputLabel = AppKitKit.label("", size: 12), strengthLabel = AppKitKit.label("", size: 12)
    private let status = AppKitKit.label("", size: 12, color: AppKitKit.error), elapsed = AppKitKit.label("", size: 12)
    private var record: AppKitKitButton!, speaker: AppKitKitButton!, natural: AppKitKitButton!, enhanced: AppKitKitButton!, stopPlayback: AppKitKitButton!
    private let samples = AppKitKit.stack([], spacing: 8), meterView = NSView(), meter = AudioTestMeter()
    private let diagnostics = AppKitKit.label("", size: 11)
    private var showDiagnostics = false
    private var timer: Timer?
    private var picker: NSPopover?
    override var dismissDisabled: Bool { picker?.isShown == true }
    init(voice: VoiceClient, debugEnabled: Bool, close: @escaping () -> Void) {
        state = NativeAudioTest(voice: voice)
        super.init(title: "Audio test", detail: "Only you can hear these tests.", closeLabel: "Close audio settings", identifier: "audio-preferences-sheet", close: close)
        header.close.setAccessibilityIdentifier("close-audio-preferences")
        subviews.compactMap { $0 as? NSScrollView }.first?.setAccessibilityIdentifier("audio-preferences-controls")
        input.setAccessibilityIdentifier("audio-input-device"); output.setAccessibilityIdentifier("audio-output-device")
        for (slider, label, tag) in [(inputGain, "Test microphone volume", 0), (outputGain, "Test speaker volume", 1), (strength, "Voice processing", 2)] {
            slider.tag = tag; slider.target = self; slider.action = #selector(gainChanged(_:)); slider.isContinuous = true; slider.setAccessibilityLabel(label)
        }
        speaker = AppKitKitButton("Test speakers") { [state] in state.speaker.toggle(voice: voice) }; speaker.setAccessibilityIdentifier("speaker-test")
        record = AppKitKitButton("Test microphone", kind: .primary) { [state] in state.record() }; record.setAccessibilityIdentifier("local-mic-test")
        natural = AppKitKitButton("Play natural") { [state] in state.microphone.play(enhanced: false) }
        enhanced = AppKitKitButton("Play enhanced") { [state] in state.microphone.play(enhanced: true) }
        stopPlayback = AppKitKitButton("Stop playback") { [state] in state.microphone.stopPlayback() }
        for view in [natural!, enhanced!, stopPlayback!] { samples.addArrangedSubview(view); AppKitKit.fillWidth(view, of: samples) }
        samples.setAccessibilityElement(true); samples.setAccessibilityRole(.group); samples.setAccessibilityLabel("Recorded samples")
        meterView.wantsLayer = true; meterView.layer?.addSublayer(meter.shape); meterView.heightAnchor.constraint(equalToConstant: 36).isActive = true
        meterView.setAccessibilityElement(true); meterView.setAccessibilityRole(.levelIndicator)
        let mic = Self.group([input, inputLabel, inputGain], label: "Microphone", spacing: 10)
        let out = Self.group([output, outputLabel, outputGain, speaker], label: "Speaker", spacing: 10)
        let columns = NSStackView(views: [mic, out]); columns.orientation = .horizontal; columns.alignment = .top; columns.spacing = 18; columns.distribution = .fillEqually
        let fixture = Self.note("TEST FIXTURE — completed local recording layout only; no microphone or playback."); fixture.isHidden = !state.recordedPreview
        var rows: [NSView] = [columns, Self.heading("Try your microphone"), Self.note("Less noise. Clearer voice."), strengthLabel, strength,
            Self.note("Natural                                                        Enhanced"), record, elapsed, meterView, fixture, status, samples]
        if debugEnabled {
            diagnostics.font = .monospacedSystemFont(ofSize: 11, weight: .regular); diagnostics.isSelectable = true
            rows += [AppKitKitButton("Audio diagnostics") { [weak self] in self?.showDiagnostics.toggle(); self?.render() }, diagnostics,
                AppKitKitButton("Copy diagnostics") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(NativeAudioReport.processing(voice), forType: .string) }]
        }
        setRows(rows); preferHeight(650)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { timer?.invalidate(); state.close() }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow(); timer?.invalidate(); timer = nil
        if window != nil {
            observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
            Task { [state] in await state.voice.refreshAudioDevices() }
            timer = Timer.scheduledTimer(withTimeInterval: 0.08, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.tick() } }
        } else { observation.stop(); state.close(); picker?.close(); picker = nil }
    }
    private func render() {
        state.sync(); let voice = state.voice, mic = state.microphone
        for (button, devices, id, label) in [(input, voice.availableInputs, voice.selectedInputID, "Microphone"), (output, voice.availableOutputs, voice.selectedOutputID, "Speaker")] {
            button.title = (id ?? "").isEmpty ? "System default" : devices.first { $0.id == id }?.name ?? "Device unavailable"
            button.setAccessibilityLabel(label); button.setAccessibilityValue(button.title); button.setAccessibilityHelp("Opens device choices")
        }
        input.isEnabled = !mic.recording && !state.preparing
        inputGain.integerValue = voice.inputGain; outputGain.integerValue = voice.outputGain; strength.integerValue = voice.voiceProcessingStrength
        inputLabel.stringValue = "Microphone volume · \(voice.inputGain)%"; outputLabel.stringValue = "Speaker volume · \(voice.outputGain)%"
        strengthLabel.stringValue = "Voice enhancement · \(voice.voiceProcessingStrength)%"; strength.isEnabled = !mic.recording
        speaker.title = state.speaker.playing ? "Stop speaker test" : "Test speakers"; speaker.setAccessibilityValue(state.speaker.playing ? "Playing" : "")
        record.title = mic.recording ? "Stop recording" : state.preparing ? "Preparing…" : "Test microphone"; record.isEnabled = state.canRecord
        samples.isHidden = !mic.hasRecording && !state.recordedPreview
        natural.isEnabled = !state.recordedPreview; enhanced.isEnabled = !state.recordedPreview
        natural.title = mic.playing == false ? "Playing…" : "Play natural"; natural.setAccessibilityLabel("Play natural")
        enhanced.title = mic.playing == true ? "Playing…" : "Play enhanced"; enhanced.setAccessibilityLabel("Play enhanced")
        stopPlayback.isEnabled = !state.recordedPreview && mic.playing != nil
        status.stringValue = [state.routeError, mic.error, state.speaker.error, mic.silent ? "No audible signal detected. Check your mic and try again." : nil].compactMap { $0 }.joined(separator: "\n")
        status.isHidden = status.stringValue.isEmpty; diagnostics.isHidden = !showDiagnostics; tick()
    }
    private func tick() {
        elapsed.stringValue = state.microphone.startedAt.map { String(format: "Recording… %.1fs", min(30, Date().timeIntervalSince($0))) } ?? ""
        elapsed.isHidden = elapsed.stringValue.isEmpty
        meter.tick(state, bounds: meterView.bounds, active: NSColor(CaperTheme.green).cgColor, idle: AppKitKit.border.cgColor)
        meterView.setAccessibilityLabel(state.microphone.recording ? "Received microphone level" : "Microphone level inactive")
        if showDiagnostics { diagnostics.stringValue = "Local diagnostics only. Nothing is uploaded.\n\(state.voice.noiseSuppressionStatus)\n\(NativeAudioReport.processing(state.voice))" }
    }
    @objc private func gainChanged(_ slider: NSSlider) {
        let value = Int(slider.doubleValue.rounded())
        if slider.tag == 0 { state.voice.setInputGain(value) } else if slider.tag == 1 { state.voice.setOutputGain(value) } else { state.voice.setVoiceProcessingStrength(value) }
        CaperEffects.shared.slider(Double(value) / slider.maxValue)
    }
    private func choose(input: Bool) {
        guard picker == nil else { return }
        let devices = input ? state.voice.availableInputs : state.voice.availableOutputs, selection = input ? state.voice.selectedInputID : state.voice.selectedOutputID
        let popover = NSPopover(); picker = popover; popover.behavior = .transient; popover.delegate = self
        let controller = NSViewController(), body = AppKitKit.stack([], spacing: 4)
        let close = AppKitKitButton("Close device picker") { [weak popover] in popover?.performClose(nil) }; close.keyEquivalent = "\u{1b}"
        body.addArrangedSubview(close)
        for (id, name) in [("", "System default")] + devices.map({ ($0.id, $0.name) }) {
            let button = AppKitKitButton(name) { [weak self, weak popover] in self?.state.select(id, input: input); popover?.performClose(nil) }
            button.minimumHeight = 44; button.alignment = .left
            button.setAccessibilityValue(id == (selection ?? "") ? "Selected" : "Not selected")
            body.addArrangedSubview(button); AppKitKit.fillWidth(button, of: body)
        }
        controller.view = appKitKitScroll(body, insets: NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16))
        popover.contentViewController = controller; popover.contentSize = NSSize(width: 320, height: min(380, 32 + 48 * CGFloat(devices.count + 2)))
        let anchor = input ? self.input : output
        popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .maxY)
        controller.view.window?.makeFirstResponder(close)
    }
    func popoverDidClose(_ notification: Notification) { picker = nil }
}
#elseif os(iOS)
import UIKit
import MediaPlayer

@MainActor final class UIKitAudioTest: UIKitMessageDialog {
    private let state: NativeAudioTest
    private let debugEnabled: Bool
    private let inputGain = UISlider(), outputGain = UISlider(), strength = UISlider()
    private let inputLabel = UIKitMessageDialog.label(""), outputLabel = UIKitMessageDialog.label(""), strengthLabel = UIKitMessageDialog.label("")
    private let inputRoute = UIKitMessageDialog.label(""), outputRoute = UIKitMessageDialog.label("")
    private let status = UIKitMessageDialog.label("", error: true), elapsed = UIKitMessageDialog.label(""), diagnostics = UIKitMessageDialog.label("")
    private let meterView = UIView(), meter = AudioTestMeter(), samples = UIStackView()
    private var record: UIButton!, speaker: UIButton!, natural: UIButton!, enhanced: UIButton!, stopPlayback: UIButton!
    private var timer: Timer?
    private var showDiagnostics = false
    init(voice: VoiceClient, debugEnabled: Bool, close: @escaping () -> Void) {
        state = NativeAudioTest(voice: voice); self.debugEnabled = debugEnabled
        super.init(title: "Audio test", closeLabel: "Close audio settings", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { timer?.invalidate(); state.close() }
    override func viewDidLoad() {
        super.viewDidLoad(); view.accessibilityIdentifier = "audio-preferences-sheet"
        body.superview?.accessibilityIdentifier = "audio-preferences-controls"
        for (slider, label, tag) in [(inputGain, "Test microphone volume", 0), (outputGain, "Test speaker volume", 1), (strength, "Voice processing", 2)] {
            slider.minimumValue = 0; slider.maximumValue = tag == 2 ? 100 : 200; slider.tag = tag; slider.tintColor = UIColor(CaperTheme.terracottaBright)
            slider.accessibilityLabel = label; slider.addTarget(self, action: #selector(gainChanged(_:)), for: .valueChanged)
            slider.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        }
        let route = MPVolumeView(); route.showsVolumeSlider = false; route.tintColor = UIColor(CaperTheme.text)
        route.accessibilityLabel = "Choose system audio route"; route.accessibilityIdentifier = "system-audio-route-picker"
        route.widthAnchor.constraint(equalToConstant: 44).isActive = true; route.heightAnchor.constraint(equalToConstant: 44).isActive = true
        let output = UIStackView(arrangedSubviews: [outputRoute, route]); output.spacing = 8
        record = Self.button("Test microphone", primary: true) { [state] in state.record() }; record.accessibilityIdentifier = "local-mic-test"
        speaker = Self.button("Test speakers") { [state] in state.speaker.toggle(voice: state.voice) }; speaker.accessibilityIdentifier = "speaker-test"
        natural = Self.button("Play natural") { [state] in state.microphone.play(enhanced: false) }
        enhanced = Self.button("Play enhanced") { [state] in state.microphone.play(enhanced: true) }
        stopPlayback = Self.button("Stop playback") { [state] in state.microphone.stopPlayback() }
        samples.axis = .vertical; samples.spacing = 8; [natural!, enhanced!, stopPlayback!].forEach(samples.addArrangedSubview); samples.accessibilityLabel = "Recorded samples"
        meterView.layer.addSublayer(meter.shape); meterView.heightAnchor.constraint(equalToConstant: 36).isActive = true; meterView.isAccessibilityElement = true
        let fixture = Self.label("TEST FIXTURE — completed local recording layout only; no microphone or playback."); fixture.isHidden = !state.recordedPreview
        [Self.label("Only you can hear these tests."), inputRoute, Self.label("Microphone follows the system audio route."), inputLabel, inputGain,
         output, outputLabel, outputGain, speaker!, Self.label("Try your microphone", size: 14, bold: true), Self.label("Less noise. Clearer voice."), strengthLabel, strength,
         Self.label("Natural                                  Enhanced"), record!, elapsed, meterView, fixture, status, samples].forEach(body.addArrangedSubview)
        if debugEnabled {
            body.addArrangedSubview(Self.button("Audio diagnostics") { [weak self] in self?.showDiagnostics.toggle(); self?.render() })
            body.addArrangedSubview(diagnostics)
            body.addArrangedSubview(Self.button("Copy diagnostics") { [state] in UIPasteboard.general.string = NativeAudioReport.processing(state.voice) })
        }
        observe(); Task { [state] in await state.voice.refreshAudioDevices() }
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated); timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.08, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.tick() } }
    }
    override func stop() { super.stop(); timer?.invalidate(); timer = nil; state.close() }
    override func render() {
        state.sync(); let voice = state.voice, mic = state.microphone
        inputRoute.text = voice.availableInputs.first { $0.id == voice.selectedInputID }?.name ?? "System default"
        outputRoute.text = voice.availableOutputs.first { $0.id == voice.selectedOutputID }?.name ?? "System default"
        inputGain.value = Float(voice.inputGain); outputGain.value = Float(voice.outputGain); strength.value = Float(voice.voiceProcessingStrength)
        for slider in [inputGain, outputGain, strength] { slider.accessibilityValue = "\(Int(slider.value))%" }
        inputLabel.text = "Microphone volume · \(voice.inputGain)%"; outputLabel.text = "Speaker volume · \(voice.outputGain)%"; strengthLabel.text = "Voice enhancement · \(voice.voiceProcessingStrength)%"
        strength.isEnabled = !mic.recording
        speaker.configuration?.title = state.speaker.playing ? "Stop speaker test" : "Test speakers"; speaker.accessibilityValue = state.speaker.playing ? "Playing" : ""
        record.configuration?.title = mic.recording ? "Stop recording" : state.preparing ? "Preparing…" : "Test microphone"; record.isEnabled = state.canRecord
        samples.isHidden = !mic.hasRecording && !state.recordedPreview
        natural.isEnabled = !state.recordedPreview; enhanced.isEnabled = !state.recordedPreview
        natural.configuration?.title = mic.playing == false ? "Playing…" : "Play natural"; natural.accessibilityLabel = "Play natural"
        enhanced.configuration?.title = mic.playing == true ? "Playing…" : "Play enhanced"; enhanced.accessibilityLabel = "Play enhanced"
        stopPlayback.isEnabled = !state.recordedPreview && mic.playing != nil
        status.text = [state.routeError, mic.error, state.speaker.error, mic.silent ? "No audible signal detected. Check your mic and try again." : nil].compactMap { $0 }.joined(separator: "\n")
        status.isHidden = status.text?.isEmpty != false; diagnostics.isHidden = !showDiagnostics; tick(); parent?.view.setNeedsLayout()
    }
    private func tick() {
        elapsed.text = state.microphone.startedAt.map { String(format: "Recording… %.1fs", min(30, Date().timeIntervalSince($0))) }
        elapsed.isHidden = elapsed.text == nil
        meter.tick(state, bounds: meterView.bounds, active: UIColor(CaperTheme.green).cgColor, idle: UIColor(CaperTheme.border).cgColor)
        meterView.accessibilityLabel = state.microphone.recording ? "Received microphone level" : "Microphone level inactive"
        if showDiagnostics { diagnostics.text = "Local diagnostics only. Nothing is uploaded.\n\(state.voice.noiseSuppressionStatus)\n\(NativeAudioReport.processing(state.voice))" }
    }
    @objc private func gainChanged(_ slider: UISlider) {
        let value = Int(slider.value.rounded())
        if slider.tag == 0 { state.voice.setInputGain(value) } else if slider.tag == 1 { state.voice.setOutputGain(value) } else { state.voice.setVoiceProcessingStrength(value) }
        CaperEffects.shared.slider(Double(value) / Double(slider.maximumValue))
    }
}
#endif
