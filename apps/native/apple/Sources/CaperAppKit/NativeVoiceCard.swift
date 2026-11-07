import AppKit
import CaperCore

/// A stable channel row grows into a call card. Speaking and the one-second
/// clock update this region only; controls survive roster and navigation refreshes.
@MainActor final class NativeVoiceCard: NativeView {
    let heading: NSView
    var changed: (() -> Void)?
    private let model: AppModel
    private let channelID: String
    private let panels: NativeVoicePanels
    private let openChat: () -> Void
    private let observation = NativeObservation()
    private let body = NativeView()
    private let state = NativeStyle.label("", size: 11, bold: true)
    private let clock = NativeStyle.label("", size: 11)
    private var timer: Timer?
    private var availability: Task<Void, Never>?
    private var startedAt: Double?
    private var own = false
    private var live = false
    private var people: [CallPerson] = []
    private var occupants: [VoiceSpectator] = []
    private var faces: [String: NativeVoiceFace] = [:]
    private var faceOrder: [NativeVoiceFace] = []
    private var previousRows = 0
    private var hovering = false
    private var tracking: NSTrackingArea?
    private lazy var mute = NativeButton.icon("Mute microphone", symbol: "mic") { [weak self] in
        guard let self else { return }; VoicePresentation.toggleMute(self.model.voice)
    }
    private lazy var deafen = NativeButton.icon("Deafen audio", symbol: "headphones") { [weak self] in
        guard let self else { return }; VoicePresentation.toggleDeafen(self.model.voice)
    }
    private lazy var leave = NativeButton("Leave") { [weak self] in
        guard let self else { return }; VoicePresentation.leave(self.model)
    }
    lazy var join = NativeButton("Join") { [weak self] in
        guard let self, let channel = self.channel else { return }
        ChannelVoiceAction(model: self.model, channel: channel).join()
    }
    private var channel: Channel? {
        model.detail?.channels.first { $0.id == channelID } ?? (model.visitChannel?.id == channelID ? model.visitChannel : nil)
    }

    init(model: AppModel, channelID: String, heading: NSView, panels: NativeVoicePanels, openChat: @escaping () -> Void) {
        self.model = model; self.channelID = channelID; self.heading = heading
        self.panels = panels; self.openChat = openChat
        super.init()
        addSubview(heading); addSubview(body); addSubview(join)
        [state, clock, mute, deafen, leave].forEach { body.addSubview($0) }
        layer?.cornerRadius = 10
        setAccessibilityElement(true); setAccessibilityRole(.group)
        clock.textColor = NSColor(CaperTheme.voiceSessionGreen)
        clock.alignment = .right
        clock.setAccessibilityLabel("Voice session duration")
        state.setAccessibilityIdentifier("voice-call-status")
        mute.setAccessibilityIdentifier("voice-call-mute")
        deafen.setAccessibilityIdentifier("voice-call-deafen")
        leave.setAccessibilityIdentifier("voice-call-leave")
        leave.font = NativeStyle.font(12, bold: true)
        leave.contentTintColor = NSColor(VoicePresentation.disconnect)
        leave.layer?.backgroundColor = NSColor(VoicePresentation.disconnect).withAlphaComponent(0.12).cgColor
        join.font = NativeStyle.font(12, bold: true)
        join.focusChanged = { [weak self] focused in
            self?.updateJoinVisibility(focused: focused)
            if focused { self?.prepareJoin() }
        }
        observation.start { [weak self] in self?.render() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { timer?.invalidate(); availability?.cancel() }

    private func render() {
        let wasLive = live
        own = model.voice.isActive(channelID: channelID)
        people = own ? VoicePresentation.people(model) : []
        occupants = own ? [] : VoicePresentation.occupants(model, channelID: channelID)
        live = own || !occupants.isEmpty
        startedAt = own ? VoicePresentation.startedAt(model)
            : model.voicePresence.sessionStartedAt(for: channelID) ?? model.visitVoicePresence.sessionStartedAt(for: channelID)
        body.isHidden = !live
        layer?.backgroundColor = live ? NSColor(own ? VoicePresentation.callFill : VoicePresentation.roomFill).cgColor : NSColor.clear.cgColor
        layer?.borderWidth = live ? 1 : 0
        layer?.borderColor = NSColor(own ? VoicePresentation.callBorder : VoicePresentation.roomBorder).cgColor
        setAccessibilityIdentifier(own ? "voice-call-card" : "live-room-\(channelID)")
        state.stringValue = own ? VoicePresentation.pending(model.voice) ?? "You’re in · \(people.count)"
            : LiveNow.names(occupants.map(\.name))
        state.toolTip = state.stringValue
        state.textColor = own && VoicePresentation.pending(model.voice) != nil ? NSColor(VoicePresentation.connecting) : NativeStyle.muted
        mute.isHidden = !own; deafen.isHidden = !own; leave.isHidden = !own
        updateToggle(mute, on: model.voice.muted, onLabel: "Unmute microphone", offLabel: "Mute microphone", onSymbol: "mic.slash", offSymbol: "mic", value: "Muted")
        updateToggle(deafen, on: model.voice.deafened, onLabel: "Undeafen audio", offLabel: "Deafen audio", onSymbol: "speaker.slash", offSymbol: "headphones", value: "Deafened")
        leave.title = model.voice.phase == .joining ? "Cancel" : "Leave"
        leave.setAccessibilityLabel(model.voice.phase == .joining ? "Cancel joining voice" : "Leave voice")
        if let channel {
            let action = ChannelVoiceAction(model: model, channel: channel)
            join.isHidden = own
            join.title = live ? action.title : ""
            join.image = live ? nil : NSImage(systemSymbolName: "headphones", accessibilityDescription: nil)
            join.imagePosition = live ? .noImage : .imageOnly
            join.isEnabled = action.enabled
            join.setAccessibilityLabel(action.description); join.toolTip = action.help
            join.setAccessibilityIdentifier("\(live ? "live-join" : "channel-voice")-\(channelID)")
            join.contentTintColor = live && !action.switches ? NativeStyle.blackout : NativeStyle.text
            join.layer?.backgroundColor = live ? (action.switches ? NativeStyle.composer : NSColor(CaperTheme.action)).cgColor : NSColor.clear.cgColor
            if action.busy { join.alphaValue = 1 }
        } else { join.isHidden = true }
        updateJoinVisibility()
        updateClock()
        updateTimer()
        needsLayout = true
        if wasLive != live {
            changed?()
            if live && window != nil && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
                let fade = CABasicAnimation(keyPath: "opacity")
                fade.fromValue = 0; fade.toValue = 1; fade.duration = 0.14
                fade.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
                body.layer?.add(fade, forKey: "entrance")
            }
        }
    }

    private func updateToggle(_ button: NativeButton, on: Bool, onLabel: String, offLabel: String, onSymbol: String, offSymbol: String, value: String) {
        button.image = NSImage(systemSymbolName: on ? onSymbol : offSymbol, accessibilityDescription: nil)
        button.setAccessibilityLabel(on ? onLabel : offLabel)
        button.setAccessibilityValue(on ? value : "On"); button.toolTip = on ? onLabel : offLabel
        button.contentTintColor = on ? NSColor(CaperTheme.voiceOff) : NativeStyle.text
        button.layer?.backgroundColor = (on ? NSColor(CaperTheme.voiceOff).withAlphaComponent(0.16) : NSColor(CaperTheme.raised)).cgColor
    }

    func height(width: CGFloat) -> CGFloat {
        guard live else { return 32 }
        return own ? 32 + 28 + CGFloat(faceRows(width: width)) * 44 + 54 : 112
    }
    private func faceRows(width: CGFloat) -> Int {
        let columns = LiveNow.columns(width: Double(max(0, width - 24)), pitch: 44)
        let count = min(people.count, LiveNow.faceSlots(columns: columns))
        return (count + columns - 1) / columns
    }

    override func layout() {
        super.layout()
        heading.frame = NSRect(x: live ? 3 : 0, y: live ? 3 : 0, width: max(0, bounds.width - (live ? 6 : 32)), height: 32)
        body.frame = NSRect(x: 12, y: 36, width: max(0, bounds.width - 24), height: max(0, bounds.height - 48))
        let width = body.bounds.width
        clock.frame = NSRect(x: max(0, width - 58), y: 0, width: 58, height: 18)
        state.frame = NSRect(x: 0, y: 0, width: max(0, width - (clock.isHidden ? 0 : 64)), height: 18)
        updateFaces()
        mute.frame = NSRect(x: 0, y: body.bounds.height - 32, width: 32, height: 32)
        deafen.frame = NSRect(x: 38, y: body.bounds.height - 32, width: 32, height: 32)
        leave.frame = NSRect(x: max(76, width - 72), y: body.bounds.height - 32, width: 72, height: 32)
        join.frame = live ? NSRect(x: bounds.width - 84, y: bounds.height - 44, width: 72, height: 32)
            : NSRect(x: bounds.width - 30, y: 0, width: 28, height: 32)
        let count = faceRows(width: bounds.width)
        if count != previousRows { previousRows = count; changed?() }
    }

    private func updateFaces() {
        var next: [String: NativeVoiceFace] = [:]
        faceOrder = []
        let columns = LiveNow.columns(width: Double(body.bounds.width), pitch: 44)
        if own {
            let split = LiveNow.fold(people, slots: LiveNow.faceSlots(columns: columns))
            for person in split.shown {
                let face = faces[person.id] ?? makeFace(id: person.id)
                face.update(person)
                next[person.id] = face; faceOrder.append(face)
            }
            if !split.folded.isEmpty {
                let face = faces["+more"] ?? makeFace(id: nil)
                face.updateMore(split.folded.count, speaking: split.folded.contains(where: \.speaking))
                next["+more"] = face; faceOrder.append(face)
            }
        } else if live {
            for person in occupants.prefix(3) {
                let face = faces[person.id] ?? makeFace(id: person.id)
                face.updateRoom(person)
                next[person.id] = face; faceOrder.append(face)
            }
        }
        for face in faces.values where !next.values.contains(where: { $0 === face }) { face.removeFromSuperview() }
        faces = next
        for (index, face) in faceOrder.enumerated() {
            if face.superview == nil { body.addSubview(face) }
            face.frame = own ? NSRect(x: CGFloat(index % columns) * 44, y: 24 + CGFloat(index / columns) * 44, width: 44, height: 44)
                : NSRect(x: CGFloat(index) * 26, y: 28, width: 32, height: 32)
        }
    }

    private func makeFace(id: String?) -> NativeVoiceFace {
        let face = NativeVoiceFace(open: openChat)
        face.showList = { [weak self, weak face] in
            guard let self, let face, self.own else { return }
            self.panels.participants(from: face, expanding: id)
        }
        return face
    }
    override func mouseDown(with event: NSEvent) { if live { openChat() } else { super.mouseDown(with: event) } }
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let hit = super.hitTest(point) else { return nil }
        return live && !(hit is NSButton) ? self : hit
    }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    override func mouseEntered(with event: NSEvent) { hovering = true; updateJoinVisibility(); prepareJoin() }
    override func mouseExited(with event: NSEvent) { hovering = false; updateJoinVisibility() }
    private func prepareJoin() { if let channel { model.prepareVoiceJoin(channel: channel) } }
    private func updateJoinVisibility(focused: Bool? = nil) {
        // Layer opacity preserves the native button's keyboard/AX identity.
        join.layer?.opacity = live || hovering || (focused ?? (window?.firstResponder === join)) ? 1 : 0
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow(); updateTimer()
        availability?.cancel(); availability = nil
        if window != nil, let channel {
            availability = Task { [weak model] in
                guard let model else { return }; await model.refreshVoiceAvailability(channel: channel)
            }
        }
    }
    private func updateTimer() {
        guard window != nil, live, startedAt != nil else { timer?.invalidate(); timer = nil; return }
        guard timer == nil else { return }
        let timer = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.updateClock() }
        }
        RunLoop.main.add(timer, forMode: .common); self.timer = timer
    }
    private func updateClock() {
        clock.isHidden = startedAt == nil
        clock.stringValue = startedAt.map { VoicePresentation.duration(startedAt: $0, now: .now) } ?? ""
        clock.setAccessibilityValue(clock.stringValue)
    }
}

/// One native button owns the face's hit testing, keyboard and AX actions;
/// decorative avatar/badge subviews never steal the click.
@MainActor private final class NativeVoiceFace: NSButton {
    private let avatar = NativeAvatar()
    private let badge = NSImageView()
    private let ring = CALayer()
    private let open: () -> Void
    var showList: (() -> Void)?
    private var rounded = false
    init(open: @escaping () -> Void) {
        self.open = open
        super.init(frame: .zero)
        title = ""; isBordered = false; wantsLayer = true
        font = NativeStyle.font(12, bold: true); contentTintColor = NativeStyle.text
        target = self; action = #selector(opened)
        addSubview(avatar); addSubview(badge); layer?.addSublayer(ring)
        ring.borderWidth = 2; ring.borderColor = NSColor(CaperTheme.voiceSessionGreen).cgColor
        badge.contentTintColor = NSColor(CaperTheme.error)
        badge.wantsLayer = true; badge.layer?.backgroundColor = NativeStyle.blackout.cgColor
        badge.layer?.cornerRadius = 8; badge.setAccessibilityElement(false)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func opened() { open() }
    func update(_ person: CallPerson) {
        title = ""; avatar.isHidden = false; rounded = person.picture != nil
        avatar.update(name: person.name, avatarId: person.avatarID, picture: person.picture)
        badge.isHidden = !person.muted && !person.deafened
        badge.image = NSImage(systemSymbolName: person.deafened ? "speaker.slash.fill" : "mic.slash.fill", accessibilityDescription: nil)
        ring.isHidden = !person.speaking
        setAccessibilityLabel(person.name + (person.isSelf ? " (you)" : ""))
        setAccessibilityValue(VoicePresentation.state(person)); setAccessibilityIdentifier("voice-face-\(person.id)")
        toolTip = person.isSelf ? "\(person.name) (you): right-click for everyone" : "\(person.name): right-click for volume"
        setAccessibilityCustomActions([NSAccessibilityCustomAction(name: person.isSelf ? "Show everyone" : "Volume", handler: { [weak self] in self?.showList?(); return true })])
    }
    func updateMore(_ count: Int, speaking: Bool) {
        title = "+\(count)"; avatar.isHidden = true; badge.isHidden = true; rounded = false; ring.isHidden = !speaking
        setAccessibilityLabel("\(count) more in the call"); setAccessibilityIdentifier("voice-faces-more")
        toolTip = "Right-click for everyone in the call"
        setAccessibilityCustomActions([NSAccessibilityCustomAction(name: "Show everyone", handler: { [weak self] in self?.showList?(); return true })])
    }
    func updateRoom(_ person: VoiceSpectator) {
        title = ""; avatar.isHidden = false; badge.isHidden = true; ring.isHidden = true
        avatar.update(name: person.name, avatarId: person.avatarId, picture: person.avatarPicture)
        setAccessibilityLabel(person.name); setAccessibilityValue(""); setAccessibilityCustomActions([])
        setAccessibilityIdentifier("room-face-\(person.id)"); toolTip = nil
    }
    override func layout() {
        super.layout()
        avatar.frame = bounds.insetBy(dx: 4, dy: 4)
        badge.frame = NSRect(x: bounds.width - 17, y: 0, width: 16, height: 16)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        ring.frame = bounds; ring.cornerRadius = rounded ? ProfilePicturePolicy.cornerRadius(side: bounds.width) : bounds.width / 2
        CATransaction.commit()
    }
    override func hitTest(_ point: NSPoint) -> NSView? { frame.contains(point) ? self : nil }
    override func rightMouseDown(with event: NSEvent) { showList?() }
}
