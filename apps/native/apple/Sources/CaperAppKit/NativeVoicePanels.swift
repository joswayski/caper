import AppKit
import CaperCore

/// Short-lived native popovers. No participant, device or gain state is owned
/// here: VoiceClient remains the source of truth, including changes elsewhere.
@MainActor final class NativeVoicePanels: NSObject, NSPopoverDelegate {
    private let model: AppModel
    private var popover: NSPopover?
    private weak var previousResponder: NSResponder?
    private weak var parentWindow: NSWindow?
    init(model: AppModel) { self.model = model; super.init() }

    func participants(from view: NSView, expanding id: String?) {
        guard VoicePresentation.active(model.voice), let context = model.voice.context else { return }
        let list = NativeVoiceParticipants(model: model, channelID: context.channelID, expanding: id) { [weak self] in self?.close() }
        show(list, from: view)
    }
    func audio(input: Bool, from view: NSView) {
        show(NativeAudioOptions(voice: model.voice, input: input), from: view)
    }
    private func show(_ content: NSView, from view: NSView) {
        guard let window = view.window else { return }
        close()
        previousResponder = window.firstResponder; parentWindow = window
        let popover = NSPopover()
        self.popover = popover
        popover.behavior = .transient
        popover.animates = !CaperAccessibility.reduceMotion
        popover.delegate = self
        let controller = NSViewController(); controller.view = content
        controller.preferredContentSize = content.frame.size
        popover.contentViewController = controller
        popover.show(relativeTo: view.bounds, of: view, preferredEdge: .maxX)
    }
    func close() { popover?.animates = false; popover?.close() }
    func popoverShouldClose(_ popover: NSPopover) -> Bool { popover.animates = false; return true }
    func popoverDidClose(_ notification: Notification) {
        guard let closed = notification.object as? NSPopover else { return }
        closed.contentViewController = nil
        guard popover === closed else { return }
        popover = nil
        if let previousResponder, let parentWindow,
           (previousResponder as? NSView)?.window === parentWindow {
            parentWindow.makeFirstResponder(previousResponder)
        }
        previousResponder = nil; parentWindow = nil
    }
}

@MainActor private final class NativeVoiceParticipants: NativeView {
    private let model: AppModel
    private let channelID: String
    private let close: () -> Void
    private let observation = NativeObservation()
    private let title = NativeStyle.label("", size: 13, bold: true)
    private lazy var dismiss = NativeButton.icon("Close", symbol: "xmark", action: close)
    private let scroll = NSScrollView()
    private let rows = NativeView()
    private var controls: [String: NativeParticipantRow] = [:]
    private var order: [NativeParticipantRow] = []
    private var expandedID: String?

    init(model: AppModel, channelID: String, expanding id: String?, close: @escaping () -> Void) {
        self.model = model; self.channelID = channelID; self.close = close; expandedID = id
        super.init(NativeStyle.surface)
        frame.size = NSSize(width: 320, height: 420)
        [title, dismiss, scroll].forEach { addSubview($0) }
        title.setAccessibilityIdentifier("voice-list-title")
        dismiss.setAccessibilityIdentifier("voice-list-close")
        scroll.documentView = rows; scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true; scroll.drawsBackground = false
        setAccessibilityIdentifier("voice-participants")
        observation.start { [weak self] in self?.render() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func cancelOperation(_ sender: Any?) { close() }
    private func render() {
        guard VoicePresentation.active(model.voice), model.voice.context?.channelID == channelID else { close(); return }
        let people = VoicePresentation.people(model)
        title.stringValue = "In #\(model.voice.context?.channelName ?? "") · \(LiveNow.people(people.count))"
        var next: [String: NativeParticipantRow] = [:]
        order = []
        for person in people {
            let row = controls[person.id] ?? NativeParticipantRow(voice: model.voice, id: person.id) { [weak self] in
                guard let self else { return }
                self.expandedID = self.expandedID == person.id ? nil : person.id
                self.render()
            }
            row.update(person, expanded: !person.isSelf && expandedID == person.id)
            next[person.id] = row; order.append(row)
            if row.superview == nil { rows.addSubview(row) }
        }
        for row in controls.values where !next.values.contains(where: { $0 === row }) { row.removeFromSuperview() }
        controls = next; rows.setAccessibilityChildren(order)
        needsLayout = true
    }
    override func layout() {
        super.layout()
        title.frame = NSRect(x: 16, y: 12, width: bounds.width - 56, height: 22)
        dismiss.frame = NSRect(x: bounds.width - 36, y: 8, width: 28, height: 28)
        scroll.frame = NSRect(x: 0, y: 44, width: bounds.width, height: bounds.height - 44)
        var y: CGFloat = 8
        for row in order {
            row.frame = NSRect(x: 8, y: y, width: scroll.contentSize.width - 16, height: row.preferredHeight)
            y += row.preferredHeight + 2
        }
        rows.frame.size = NSSize(width: scroll.contentSize.width, height: y + 8)
    }
}

@MainActor private final class NativeParticipantRow: NativeView {
    private let voice: VoiceClient
    private let id: String
    private let avatar = NativeAvatar()
    private let name = NativeStyle.label("", size: 13)
    private let status = NativeStyle.label("", size: 11)
    private let volume = NativeStyle.label("", size: 12, bold: true)
    private let note = NativeStyle.label("Only changes what you hear.", size: 10)
    private let audio: NativeButton
    private let muted = NSButton(checkboxWithTitle: "Mute for me", target: nil, action: nil)
    private lazy var slider = NativeVoiceSlider { [weak self] value in
        guard let self else { return }; self.voice.setParticipantGain(value, participantID: self.id)
    }
    private var expanded = false
    var preferredHeight: CGFloat { expanded ? 156 : 52 }

    init(voice: VoiceClient, id: String, toggle: @escaping () -> Void) {
        self.voice = voice; self.id = id; audio = NativeButton("Audio ⌄", action: toggle)
        super.init()
        [avatar, name, status, audio, volume, slider, muted, note].forEach { addSubview($0) }
        layer?.cornerRadius = 8
        audio.font = NativeStyle.font(11, bold: true)
        audio.setAccessibilityIdentifier("participant-audio-\(id)")
        setAccessibilityIdentifier("voice-participant-\(id)")
        muted.target = self; muted.action = #selector(muteChanged)
        muted.font = NativeStyle.font(12)
        status.textColor = NativeStyle.muted; note.textColor = NativeStyle.muted
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func applyTheme() {
        super.applyTheme()
        layer?.backgroundColor = expanded ? NativeStyle.raised.cgColor : NSColor.clear.cgColor
    }
    @objc private func muteChanged() {
        let value = muted.state == .on
        voice.setParticipantMuted(value, participantID: id)
    }
    func update(_ person: CallPerson, expanded: Bool) {
        self.expanded = expanded
        name.stringValue = person.name + (person.isSelf ? " (you)" : "")
        name.font = NativeStyle.font(13, bold: person.speaking)
        avatar.update(name: person.name, avatarId: person.avatarID, picture: person.picture)
        status.stringValue = person.locallyMuted ? "You muted \(person.name)" : person.speaking ? "Speaking" : person.deafened ? "Deafened" : person.muted ? "Muted" : ""
        status.textColor = person.locallyMuted ? NativeStyle.accent : person.speaking ? NativeStyle.voiceSessionGreen : NativeStyle.muted
        status.setAccessibilityIdentifier(person.locallyMuted ? "participant-local-muted-\(id)" : "participant-status-\(id)")
        audio.isHidden = person.isSelf
        audio.setAccessibilityLabel("Audio controls for \(person.name)"); audio.setAccessibilityValue(expanded ? "Expanded" : "Collapsed")
        [volume, slider, muted, note].forEach { $0.isHidden = !expanded }
        slider.setAccessibilityLabel("\(person.name) volume")
        slider.setValue(voice.participantGains[id] ?? 100)
        volume.stringValue = "User volume · \(slider.integerValue)%"
        muted.state = person.locallyMuted ? .on : .off
        layer?.backgroundColor = expanded ? NativeStyle.raised.cgColor : NSColor.clear.cgColor
        needsLayout = true
    }
    override func layout() {
        super.layout()
        avatar.frame = NSRect(x: 8, y: 10, width: 30, height: 30)
        name.frame = NSRect(x: 48, y: 6, width: bounds.width - 124, height: 20)
        status.frame = NSRect(x: 48, y: 27, width: bounds.width - 56, height: 18)
        audio.frame = NSRect(x: bounds.width - 68, y: 7, width: 60, height: 26)
        volume.frame = NSRect(x: 48, y: 54, width: bounds.width - 56, height: 20)
        slider.frame = NSRect(x: 48, y: 76, width: bounds.width - 56, height: 24)
        // A checkbox only tracks its box and label. Do not advertise the blank
        // remainder of the row as part of its activation frame.
        muted.sizeToFit()
        muted.frame = NSRect(x: 48, y: 106, width: min(bounds.width - 56, muted.frame.width), height: 20)
        note.frame = NSRect(x: 48, y: 132, width: bounds.width - 56, height: 16)
    }
}

@MainActor private final class NativeAudioOptions: NativeView {
    private let voice: VoiceClient
    private let input: Bool
    private let observation = NativeObservation()
    private let title = NativeStyle.label("", size: 13, bold: true)
    private lazy var devices = AppKitCaperSelect { [weak self] index in
        self?.selectDevice(at: index)
    }
    private let volume = NativeStyle.label("", size: 12)
    private let error = NativeStyle.label("", size: 11)
    private var deviceIDs: [String] = []
    private var deviceNames: [String] = []
    private var refresh: Task<Void, Never>?
    private lazy var slider = NativeVoiceSlider { [weak self] value in
        guard let self else { return }
        if self.input { self.voice.setInputGain(value) } else { self.voice.setOutputGain(value) }
    }
    init(voice: VoiceClient, input: Bool) {
        self.voice = voice; self.input = input
        super.init(NativeStyle.surface)
        frame.size = NSSize(width: 288, height: 204)
        [title, devices, volume, slider, error].forEach { addSubview($0) }
        title.stringValue = input ? "Microphone" : "Audio output"
        devices.setAccessibilityLabel(input ? "Microphone device" : "Output device")
        slider.setAccessibilityLabel(input ? "Input volume" : "Output volume")
        error.textColor = NativeStyle.error; error.maximumNumberOfLines = 2
        observation.start { [weak self] in self?.render() }
        refresh = Task { [weak voice] in
            guard let voice else { return }; await voice.refreshAudioDevices()
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { refresh?.cancel() }
    private func render() {
        let routes = input ? voice.availableInputs : voice.availableOutputs
        let ids = [""] + routes.map(\.id), names = ["System default"] + routes.map(\.name)
        if ids != deviceIDs || names != deviceNames {
            deviceIDs = ids; deviceNames = names
            devices.items = names
        }
        let selected = (input ? voice.selectedInputID : voice.selectedOutputID) ?? ""
        devices.selectedIndex = ids.firstIndex(of: selected) ?? 0
        slider.setValue(input ? voice.inputGain : voice.outputGain)
        volume.stringValue = "\(input ? "Input" : "Output") volume · \(slider.integerValue)%"
    }
    private func selectDevice(at index: Int) {
        guard deviceIDs.indices.contains(index) else { return }
        let id = deviceIDs[index]
        let changed = input ? voice.selectInput(id) : voice.selectOutput(id)
        error.stringValue = changed ? "" : "Could not switch devices. Check system audio settings."
        render()
    }
    override func layout() {
        super.layout()
        title.frame = NSRect(x: 16, y: 16, width: bounds.width - 32, height: 22)
        devices.frame = NSRect(x: 16, y: 46, width: bounds.width - 32, height: 34)
        volume.frame = NSRect(x: 16, y: 92, width: bounds.width - 32, height: 20)
        slider.frame = NSRect(x: 16, y: 116, width: bounds.width - 32, height: 28)
        error.frame = NSRect(x: 16, y: 152, width: bounds.width - 32, height: 36)
    }
}

/// Bespoke Caper voice gain slider wrapping AppKitCaperSlider with 100% haptic snap.
@MainActor private final class NativeVoiceSlider: NSView {
    private let control: AppKitCaperSlider
    var isEnabled: Bool {
        get { control.isEnabled }
        set { control.isEnabled = newValue }
    }
    var integerValue: Int { Int(control.value.rounded()) }

    init(changed: @escaping (Int) -> Void) {
        control = AppKitCaperSlider(value: 100, minValue: 0, maxValue: 200, defaultValue: 100) { next in
            let value = Int(next.rounded())
            changed(value)
        }
        super.init(frame: .zero)
        addSubview(control)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func layout() { super.layout(); control.frame = bounds }
    func setValue(_ value: Int) { control.value = Double(value) }
    override func setAccessibilityLabel(_ label: String?) { control.setAccessibilityLabel(label) }
}
