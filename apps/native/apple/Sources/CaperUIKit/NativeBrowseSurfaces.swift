import CaperCore
import UIKit

/// A reusable native list cell, with the same full-width insets as channel rows.
final class NativeBrowseCell: UICollectionViewListCell {
    private var displayed: UIView?
    func show(_ view: UIView, inset: CGFloat = 16) {
        guard displayed !== view || view.superview !== contentView else { return }
        if displayed?.superview === contentView { displayed?.removeFromSuperview() }
        displayed = view
        view.translatesAutoresizingMaskIntoConstraints = false; contentView.addSubview(view)
        NSLayoutConstraint.activate([
            view.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: inset),
            view.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -inset),
            view.topAnchor.constraint(equalTo: contentView.topAnchor), view.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])
    }
    override func prepareForReuse() {
        super.prepareForReuse()
        if displayed?.superview === contentView { displayed?.removeFromSuperview() }
        displayed = nil
    }
}

/// The rail observes badges independently of Browse's structural snapshots.
final class NativeSpaceTile: UIButton {
    private let model: AppModel
    private let space: Space
    private let avatar = AvatarView(side: 44, corner: 12)
    private let unread = UIView()
    private let mentions = UIKitStyle.label(size: 11, weight: .bold)
    private let muted = UIImageView(image: UIKitStyle.icon("bell-off"))
    private let voice = UIImageView(image: UIKitStyle.icon("volume-2"))
    private let observer = ModelObserver()
    init(model: AppModel, space: Space) {
        self.model = model; self.space = space
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        avatar.isUserInteractionEnabled = false; addSubview(avatar)
        NSLayoutConstraint.activate([avatar.centerXAnchor.constraint(equalTo: centerXAnchor), avatar.centerYAnchor.constraint(equalTo: centerYAnchor)])
        [unread, mentions, muted, voice].forEach { $0.isUserInteractionEnabled = false; addSubview($0) }
        mentions.translatesAutoresizingMaskIntoConstraints = true; mentions.textAlignment = .center
        mentions.backgroundColor = UIKitStyle.terracotta; mentions.layer.cornerRadius = 9; mentions.clipsToBounds = true
        unread.backgroundColor = UIKitStyle.text; unread.layer.cornerRadius = 3
        for badge in [muted, voice] { badge.tintColor = UIKitStyle.text; badge.backgroundColor = UIKitStyle.blackout; badge.contentMode = .center; badge.layer.cornerRadius = 9 }
        layer.cornerRadius = 12; layer.shadowColor = UIKitStyle.terracotta.cgColor; layer.shadowRadius = 6; layer.shadowOffset = .zero
        accessibilityIdentifier = "space-\(space.id)"
        addAction(UIAction { [weak self] _ in
            guard let self, !SwipeTapGuard.swiping else { return }; Task { await model.select(space: space) }
        }, for: .primaryActionTriggered)
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func didMoveToWindow() { super.didMoveToWindow(); if window != nil { observer.start { [weak self] in self?.render() } } else { observer.stop() } }
    override func layoutSubviews() {
        super.layoutSubviews()
        unread.frame = CGRect(x: -7, y: 19, width: 6, height: 6)
        let width = max(18, mentions.intrinsicContentSize.width + 8)
        mentions.frame = CGRect(x: bounds.width - width + 3, y: bounds.height - 15, width: width, height: 18)
        muted.frame = CGRect(x: -3, y: -3, width: 18, height: 18)
        voice.frame = CGRect(x: bounds.width - 15, y: -3, width: 18, height: 18)
    }
    private func render() {
        let selected = model.selectedSpaceID == space.id
        let isMuted = space.demo != true && model.notificationsMuted(.space(space.id))
        let activity = model.unreadSpaces.first { $0.id == space.id }
        let count = activity?.channels.reduce(0) { $0 + ($1.mentionCount ?? 0) } ?? 0
        let connected = model.account != nil && model.voice.phase == .connected && model.voice.context?.spaceID == space.id
        let voiceActive = model.account != nil && (connected || model.voiceActivitySpaceIDs.contains(space.id))
        let name = space.demo == true ? "Caper" : space.name
        avatar.show(name: name, avatarID: nil, picture: space.iconPicture); avatar.alpha = isMuted ? 0.55 : 1
        unread.isHidden = activity == nil; mentions.isHidden = count == 0; mentions.text = count > 99 ? "99+" : "\(count)"
        muted.isHidden = !isMuted; voice.isHidden = !voiceActive
        voice.backgroundColor = connected ? UIColor(CaperTheme.voiceSessionGreen) : UIKitStyle.blackout
        layer.borderWidth = selected ? 1 : 0; layer.borderColor = UIKitStyle.terracotta.cgColor; layer.shadowOpacity = selected ? 0.36 : 0
        accessibilityLabel = name
        accessibilityValue = [model.openingSpaceID == space.id ? "Opening" : selected ? "Selected" : "",
            activity != nil ? "Unread messages" : "", count > 0 ? "\(count) \(count == 1 ? "mention" : "mentions")" : "",
            isMuted ? "Muted" : "", connected ? "Connected to voice" : voiceActive ? "Voice active" : ""].filter { !$0.isEmpty }.joined(separator: ", ")
        setNeedsLayout()
    }
}

/// Channel identity and controls survive presence updates. Its body expands
/// into the same call/room card whether the channel lives here or in Live now.
final class NativeChannelView: UIStackView {
    private let model: AppModel
    private let id: String
    private let show: (WorkspaceSheet) -> Void
    private let present: (UIViewController) -> Void
    private let openChat: () -> Void
    private let observer = ModelObserver()
    private let header = UIStackView()
    private let name = UIButton(type: .system)
    private let count = UIKitStyle.label(size: 11, weight: .bold, color: UIColor(CaperTheme.voiceSessionGreen))
    private let countIcon = UIImageView(image: UIKitStyle.icon("volume-2")?.resized(to: 12))
    private let muteIcon = UIImageView(image: UIKitStyle.icon("bell-off")?.resized(to: 12))
    private let options = UIButton(type: .system)
    private let quick = UIButton(type: .system)
    private let card: NativeVoiceCard
    private var availability: Task<Void, Never>?
    private let external: Bool
    private var channel: Channel? {
        model.detail?.channels.first { $0.id == id } ?? (model.visitChannel?.id == id ? model.visitChannel : nil)
    }
    init(model: AppModel, id: String, external: Bool = false, show: @escaping (WorkspaceSheet) -> Void,
         present: @escaping (UIViewController) -> Void, open: @escaping () -> Void) {
        self.model = model; self.id = id; self.external = external; self.show = show; self.present = present; self.openChat = open
        card = NativeVoiceCard(model: model, channelID: id, present: present, open: open)
        super.init(frame: .zero); axis = .vertical; spacing = 0
        header.spacing = 2; header.alignment = .center
        for view in [name, muteIcon, countIcon, count, quick, options] { header.addArrangedSubview(view) }
        addArrangedSubview(header); addArrangedSubview(card)
        layer.cornerRadius = 10
        name.titleLabel?.font = UIKitStyle.font(13, .medium); name.contentHorizontalAlignment = .leading
        name.titleLabel?.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        for icon in [countIcon, muteIcon] {
            icon.contentMode = .center; icon.isAccessibilityElement = false
            icon.widthAnchor.constraint(equalToConstant: 12).isActive = true
        }
        countIcon.tintColor = UIColor(CaperTheme.voiceSessionGreen); muteIcon.tintColor = UIKitStyle.muted
        name.addAction(UIAction { _ in if !SwipeTapGuard.swiping { open() } }, for: .primaryActionTriggered)
        quick.setImage(UIKitStyle.icon("headphones")?.resized(to: 18), for: .normal)
        quick.tintColor = UIKitStyle.muted
        quick.addAction(UIAction { [weak self] _ in
            guard let self, let channel, !SwipeTapGuard.swiping else { return }; ChannelVoiceAction(model: model, channel: channel).join()
        }, for: .primaryActionTriggered)
        quick.addAction(UIAction { [weak self] _ in self?.prepare() }, for: .touchDown)
        options.setImage(UIKitStyle.icon("ellipsis")?.resized(to: 18), for: .normal); options.tintColor = UIKitStyle.muted
        options.showsMenuAsPrimaryAction = true
        options.menu = UIMenu(children: [UIDeferredMenuElement.uncached { [weak self] completion in completion(self?.menu() ?? []) }])
        for button in [quick, options] { button.widthAnchor.constraint(equalToConstant: 44).isActive = true }
        header.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        count.setContentCompressionResistancePriority(.required, for: .horizontal)
        render()
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { availability?.cancel() }
    override func didMoveToWindow() {
        super.didMoveToWindow(); availability?.cancel(); availability = nil
        if window != nil {
            observer.start { [weak self] in self?.render() }
            if let channel { availability = Task { [model] in await model.refreshVoiceAvailability(channel: channel) } }
        } else { observer.stop() }
    }
    private func prepare() { if let channel { model.prepareVoiceJoin(channel: channel) } }
    private func menu() -> [UIMenuElement] {
        guard let channel else { return [] }
        var items: [UIMenuElement] = []
        if model.isOwner {
            if !channel.private, model.detail?.space.demo != true { items.append(UIAction(title: "Invite people…") { [show] _ in show(.invitePeople(spaceID: channel.spaceId, channelID: channel.id)) }) }
            items.append(UIAction(title: "Channel settings") { [show] _ in show(.manageChannel(channel)) })
        }
        if model.account != nil { items.append(UIMenu(options: .displayInline, children: NotificationMenus.channel(model, channel: channel))) }
        if channel.joined { items.append(UIAction(title: "Leave channel") { [model, present] _ in present(UIKitHosting.leaveChannel(model: model, channel: channel)) }) }
        return items
    }
    private func render() {
        let own = model.voice.isActive(channelID: id)
        let people = VoicePresentation.occupants(model, channelID: id)
        let live = own || !people.isEmpty
        let channel = self.channel
        let channelName = channel?.name ?? (model.voice.context?.channelID == id ? model.voice.context?.channelName : nil) ?? ""
        let muted = channel.map { model.notificationsMuted(.channel(spaceID: $0.spaceId, channelID: $0.id)) } ?? false
        let unread = channel.map { model.channelUnread($0.id) } ?? false
        var configuration = UIButton.Configuration.plain()
        var attributes = AttributeContainer(); attributes.font = UIKitStyle.font(13, .medium)
        configuration.attributedTitle = AttributedString(channelName + (unread ? " •" : ""), attributes: attributes)
        configuration.image = UIKitStyle.icon(channel?.private == true ? "lock" : "hash")?.resized(to: 14)
        configuration.imagePadding = 6; configuration.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 8, bottom: 0, trailing: 2)
        configuration.baseForegroundColor = model.selectedChannelID == id ? UIKitStyle.text : UIKitStyle.muted
        name.configuration = configuration; name.alpha = muted ? 0.55 : 1; muteIcon.isHidden = !muted
        name.accessibilityLabel = channelName + (channel?.private == true ? ", private" : "")
        name.accessibilityValue = [model.openingChannelID == id ? "Opening" : model.selectedChannelID == id ? "Selected" : "", unread ? "Unread activity" : "", muted ? "Muted" : ""].filter { !$0.isEmpty }.joined(separator: ", ")
        name.accessibilityIdentifier = external ? (own ? "voice-call-channel" : "live-room-channel-\(id)") : "channel-\(id)"
        count.text = "\(people.count)"; count.accessibilityLabel = "\(people.count) in voice"; count.accessibilityIdentifier = "voice-count-\(id)"
        count.isHidden = people.isEmpty || own; countIcon.isHidden = count.isHidden
        options.isHidden = external || model.detail?.space.demo == true; options.accessibilityLabel = "Channel options for \(channelName)"; options.accessibilityIdentifier = "channel-options-\(id)"
        quick.isHidden = live || channel == nil
        if let channel {
            let action = ChannelVoiceAction(model: model, channel: channel)
            quick.isEnabled = action.enabled; quick.isHidden = live || action.here
            quick.accessibilityLabel = action.description; quick.accessibilityHint = action.help; quick.accessibilityIdentifier = "join-voice-\(id)"
        }
        card.isHidden = !live
        backgroundColor = live ? UIColor(own ? VoicePresentation.callFill : VoicePresentation.roomFill) : .clear
        layer.borderWidth = live ? 1 : 0; layer.borderColor = UIColor(own ? VoicePresentation.callBorder : VoicePresentation.roomBorder).cgColor
        header.backgroundColor = model.selectedChannelID == id ? UIKitStyle.terracotta.withAlphaComponent(0.16) : .clear
        header.layer.cornerRadius = 6
        accessibilityIdentifier = own ? "voice-call-card" : live ? "live-room-\(id)" : nil
        accessibilityLabel = own ? "Your call in #\(channelName)" : nil
    }
}

/// Stable face buttons and controls; only the face grid and clock relayout
/// when the roster, speech state or available width changes.
private final class NativeVoiceCard: UIView {
    private let model: AppModel
    private let channelID: String
    private let present: (UIViewController) -> Void
    private let open: () -> Void
    private let observer = ModelObserver()
    private let status = UIKitStyle.label(size: 11, color: UIKitStyle.muted)
    private let clock = UIKitStyle.label(size: 11, color: UIColor(CaperTheme.voiceSessionGreen))
    private var faces: [String: NativeVoiceFace] = [:]
    private var order: [NativeVoiceFace] = []
    private let overflow = UIKitStyle.label(size: 11, weight: .bold)
    private lazy var mute = IconButton(icon: "mic", label: "Mute microphone", side: 44) { [model] in VoicePresentation.toggleMute(model.voice) }
    private lazy var deafen = IconButton(icon: "headphones", label: "Deafen audio", side: 44) { [model] in VoicePresentation.toggleDeafen(model.voice) }
    private lazy var leave = CaperButton("Leave", kind: .compact, size: 12) { [model] in VoicePresentation.leave(model) }
    private lazy var join = CaperButton("Join", kind: .primary, size: 12) { [weak self] in
        guard let self, let channel else { return }; ChannelVoiceAction(model: model, channel: channel).join()
    }
    private var timer: Timer?
    private var own = false
    private var people: [CallPerson] = []
    private var occupants: [VoiceSpectator] = []
    private var startedAt: Double?
    private var lastHeight: CGFloat = 0
    private var channel: Channel? { model.detail?.channels.first { $0.id == channelID } ?? (model.visitChannel?.id == channelID ? model.visitChannel : nil) }
    init(model: AppModel, channelID: String, present: @escaping (UIViewController) -> Void, open: @escaping () -> Void) {
        self.model = model; self.channelID = channelID; self.present = present; self.open = open
        super.init(frame: .zero)
        for view in [status, clock, overflow, mute, deafen, leave, join] { view.translatesAutoresizingMaskIntoConstraints = true; addSubview(view) }
        status.accessibilityIdentifier = "voice-call-count"; clock.accessibilityLabel = "Voice session duration"; clock.textAlignment = .right
        mute.accessibilityIdentifier = "voice-call-mute"; deafen.accessibilityIdentifier = "voice-call-deafen"; leave.accessibilityIdentifier = "voice-call-leave"
        join.addAction(UIAction { [weak self] _ in if let self, let channel { model.prepareVoiceJoin(channel: channel) } }, for: .touchDown)
        let tap = UITapGestureRecognizer(target: self, action: #selector(opened)); tap.delegate = self; addGestureRecognizer(tap)
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { timer?.invalidate() }
    override func didMoveToWindow() {
        super.didMoveToWindow(); timer?.invalidate(); timer = nil
        if window != nil {
            observer.start { [weak self] in self?.render() }
            timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.updateClock() } }
        } else { observer.stop() }
    }
    @objc private func opened() { if !SwipeTapGuard.swiping { open() } }
    private func render() {
        own = model.voice.isActive(channelID: channelID)
        people = own ? VoicePresentation.people(model) : []; occupants = own ? [] : VoicePresentation.occupants(model, channelID: channelID)
        startedAt = own ? VoicePresentation.startedAt(model) : model.voicePresence.sessionStartedAt(for: channelID) ?? model.visitVoicePresence.sessionStartedAt(for: channelID)
        let pending = own ? VoicePresentation.pending(model.voice) : nil
        status.text = pending ?? (own ? LiveNow.people(people.count) : LiveNow.names(occupants.map(\.name)))
        status.textColor = pending != nil ? UIColor(VoicePresentation.connecting) : UIKitStyle.muted
        status.accessibilityIdentifier = pending != nil ? "voice-call-status" : "voice-call-count"
        mute.isHidden = !own; deafen.isHidden = !own; leave.isHidden = !own; join.isHidden = own || channel == nil
        mute.setImage(UIKitStyle.icon(model.voice.muted ? "mic-off" : "mic")?.resized(to: 18), for: .normal)
        mute.accessibilityLabel = model.voice.muted ? "Unmute microphone" : "Mute microphone"; mute.accessibilityValue = model.voice.muted ? "Muted" : "On"
        deafen.setImage(UIKitStyle.icon(model.voice.deafened ? "headphone-off" : "headphones")?.resized(to: 18), for: .normal)
        deafen.accessibilityLabel = model.voice.deafened ? "Undeafen audio" : "Deafen audio"; deafen.accessibilityValue = model.voice.deafened ? "Deafened" : "On"
        mute.tintColor = model.voice.muted ? UIKitStyle.error : UIKitStyle.text; deafen.tintColor = model.voice.deafened ? UIKitStyle.error : UIKitStyle.text
        leave.setTitle(model.voice.phase == .joining ? "Cancel" : "Leave"); leave.accessibilityLabel = model.voice.phase == .joining ? "Cancel joining voice" : "Leave voice"
        if let channel {
            let action = ChannelVoiceAction(model: model, channel: channel)
            join.setTitle(action.title); join.isEnabled = action.enabled; join.accessibilityLabel = action.description; join.accessibilityHint = action.help
            join.accessibilityIdentifier = "live-join-\(channelID)"
        }
        updateClock(); setNeedsLayout(); invalidateIntrinsicContentSize()
    }
    private func updateClock() { clock.text = startedAt.map { VoicePresentation.duration(startedAt: $0, now: .now) }; clock.accessibilityValue = clock.text }
    private var columns: Int { LiveNow.columns(width: Double(max(1, bounds.width - 24)), pitch: 44) }
    override var intrinsicContentSize: CGSize {
        let slots = min(people.count, LiveNow.faceSlots(columns: columns))
        return CGSize(width: UIView.noIntrinsicMetric, height: own ? CGFloat((slots + columns - 1) / columns) * 44 + 88 : 84)
    }
    override func layoutSubviews() {
        super.layoutSubviews()
        status.frame = CGRect(x: 12, y: 2, width: max(0, bounds.width - 90), height: 18)
        clock.frame = CGRect(x: max(12, bounds.width - 72), y: 2, width: 60, height: 18)
        var next: [String: NativeVoiceFace] = [:]; order = []
        if own {
            let split = LiveNow.fold(people, slots: LiveNow.faceSlots(columns: columns))
            for person in split.shown {
                let face = faces[person.id] ?? makeFace(id: person.isSelf ? nil : person.id)
                face.update(name: person.name, avatarID: person.avatarID, picture: person.picture, speaking: person.speaking,
                    muted: person.muted, deafened: person.deafened, label: person.name + (person.isSelf ? " (you)" : ""), value: VoicePresentation.state(person))
                face.accessibilityIdentifier = "voice-face-\(person.id)"; next[person.id] = face; order.append(face)
            }
            if !split.folded.isEmpty {
                let face = faces["+more"] ?? makeFace(id: nil)
                face.more(split.folded.count, speaking: split.folded.contains(where: \.speaking)); next["+more"] = face; order.append(face)
            }
        } else {
            for person in occupants.prefix(3) {
                let face = faces[person.id] ?? makeFace(id: person.id)
                face.update(name: person.name, avatarID: person.avatarId, picture: person.avatarPicture, label: person.name)
                face.accessibilityIdentifier = "room-face-\(person.id)"; next[person.id] = face; order.append(face)
            }
        }
        for (id, face) in faces where next[id] == nil { face.removeFromSuperview() }; faces = next
        for (index, face) in order.enumerated() {
            if face.superview == nil { addSubview(face) }
            face.frame = own ? CGRect(x: 8 + CGFloat(index % columns) * 44, y: 24 + CGFloat(index / columns) * 44, width: 44, height: 44)
                : CGRect(x: 8 + CGFloat(index) * 26, y: 28, width: 32, height: 32)
        }
        overflow.isHidden = own || occupants.count <= 3; overflow.text = "+\(max(0, occupants.count - 3))"
        overflow.frame = CGRect(x: 8 + CGFloat(order.count) * 26, y: 34, width: 40, height: 20)
        mute.frame = CGRect(x: 8, y: bounds.height - 52, width: 44, height: 44)
        deafen.frame = CGRect(x: 54, y: bounds.height - 52, width: 44, height: 44)
        leave.frame = CGRect(x: max(100, bounds.width - 90), y: bounds.height - 52, width: 78, height: 44)
        join.frame = CGRect(x: max(90, bounds.width - 88), y: 28, width: 76, height: 44)
        let height = intrinsicContentSize.height
        if height != lastHeight { lastHeight = height; invalidateIntrinsicContentSize() }
    }
    private func makeFace(id: String?) -> NativeVoiceFace {
        NativeVoiceFace(open: open) { [weak self] in
            guard let self, own else { return }
            present(NativeVoiceParticipants(model: model, channelID: channelID, expanded: id))
        }
    }
}

extension NativeVoiceCard: UIGestureRecognizerDelegate {
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        var view = touch.view
        while let current = view, current !== self { if current is UIControl { return false }; view = current.superview }
        return true
    }
}

private final class NativeVoiceFace: UIButton {
    private let avatar = AvatarView(side: 36)
    private let badge = UIImageView()
    private let ring = CALayer()
    private let list: () -> Void
    init(open: @escaping () -> Void, list: @escaping () -> Void) {
        self.list = list; super.init(frame: .zero)
        avatar.isUserInteractionEnabled = false; addSubview(avatar)
        NSLayoutConstraint.activate([avatar.centerXAnchor.constraint(equalTo: centerXAnchor), avatar.centerYAnchor.constraint(equalTo: centerYAnchor)])
        addSubview(badge); badge.tintColor = UIKitStyle.error; badge.backgroundColor = UIKitStyle.blackout; badge.layer.cornerRadius = 7
        layer.addSublayer(ring); ring.borderColor = UIColor(CaperTheme.voiceSessionGreen).cgColor; ring.borderWidth = 2
        titleLabel?.font = UIKitStyle.font(12, .bold); setTitleColor(UIKitStyle.text, for: .normal)
        addAction(UIAction { _ in if !SwipeTapGuard.swiping { open() } }, for: .primaryActionTriggered)
        addGestureRecognizer(UILongPressGestureRecognizer(target: self, action: #selector(held)))
        accessibilityCustomActions = [UIAccessibilityCustomAction(name: "Show everyone / Volume") { _ in list(); return true }]
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func held(_ gesture: UILongPressGestureRecognizer) { if gesture.state == .began { cancelTracking(with: nil); list() } }
    func update(name: String, avatarID: Int?, picture: CaperPicture?, speaking: Bool = false, muted: Bool = false, deafened: Bool = false, label: String, value: String = "") {
        setTitle(nil, for: .normal); avatar.isHidden = false; avatar.show(name: name, avatarID: avatarID, picture: picture)
        ring.isHidden = !speaking; ring.cornerRadius = picture == nil ? 22 : ProfilePicturePolicy.cornerRadius(side: 44)
        badge.isHidden = !muted && !deafened; badge.image = UIKitStyle.icon(deafened ? "headphone-off" : "mic-off")?.resized(to: 14)
        accessibilityLabel = label; accessibilityValue = value
    }
    func more(_ count: Int, speaking: Bool) {
        avatar.isHidden = true; badge.isHidden = true; setTitle("+\(count)", for: .normal); ring.isHidden = !speaking; ring.cornerRadius = 22
        accessibilityLabel = "\(count) more in the call"; accessibilityIdentifier = "voice-faces-more"
    }
    override func layoutSubviews() {
        super.layoutSubviews(); ring.frame = bounds
        let scale = min(1, max(0, bounds.width - 4) / 36); avatar.transform = CGAffineTransform(scaleX: scale, y: scale)
        badge.frame = CGRect(x: bounds.width - 14, y: bounds.height - 14, width: 14, height: 14)
    }
}

/// Participant sliders are retained while speech/roster observations refresh.
private final class NativeVoiceParticipants: UIViewController {
    private let model: AppModel
    private let channelID: String
    private var expanded: String?
    private let observer = ModelObserver()
    private let body = UIStackView()
    private let titleLabel = UIKitStyle.label(size: 15, weight: .bold)
    private var rows: [String: NativeParticipantRow] = [:]
    init(model: AppModel, channelID: String, expanded: String?) {
        self.model = model; self.channelID = channelID; self.expanded = expanded
        super.init(nibName: nil, bundle: nil); modalPresentationStyle = .pageSheet
        sheetPresentationController?.detents = [.medium(), .large()]
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); view.backgroundColor = UIKitStyle.surface; view.accessibilityIdentifier = "voice-participants"
        let scroll = UIScrollView(); scroll.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(scroll)
        body.axis = .vertical; body.spacing = 8; body.translatesAutoresizingMaskIntoConstraints = false; scroll.addSubview(body)
        titleLabel.accessibilityIdentifier = "voice-list-title"
        let close = CaperButton("Close", kind: .plain) { [weak self] in self?.dismiss(animated: false) }; close.accessibilityIdentifier = "voice-list-close"
        let header = UIStackView(arrangedSubviews: [titleLabel, close]); header.spacing = 8; body.addArrangedSubview(header)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor), scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor), scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            body.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor, constant: 16), body.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor, constant: -16),
            body.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 16), body.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -16),
            body.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -32),
        ])
    }
    override func viewWillAppear(_ animated: Bool) { super.viewWillAppear(animated); observer.start { [weak self] in self?.render() } }
    override func viewDidDisappear(_ animated: Bool) { super.viewDidDisappear(animated); observer.stop() }
    private func render() {
        guard VoicePresentation.active(model.voice), model.voice.context?.channelID == channelID else { dismiss(animated: false); return }
        let people = VoicePresentation.people(model)
        titleLabel.text = "In #\(model.voice.context?.channelName ?? "") · \(LiveNow.people(people.count))"
        let ids = Set(people.map(\.id))
        for id in Array(rows.keys) where !ids.contains(id) { rows.removeValue(forKey: id)?.removeFromSuperview() }
        for (index, person) in people.enumerated() {
            let row = rows[person.id] ?? NativeParticipantRow(voice: model.voice, id: person.id) { [weak self] in
                guard let self else { return }; expanded = expanded == person.id ? nil : person.id; render()
            }
            rows[person.id] = row; row.update(person, expanded: !person.isSelf && expanded == person.id)
            if !body.arrangedSubviews.indices.contains(index + 1) || body.arrangedSubviews[index + 1] !== row {
                body.removeArrangedSubview(row); body.insertArrangedSubview(row, at: index + 1)
            }
        }
    }
}

private final class NativeParticipantRow: UIStackView {
    private let voice: VoiceClient
    private let id: String
    private let avatar = AvatarView(side: 30)
    private let name = UIKitStyle.label(size: 13)
    private let status = UIKitStyle.label(size: 11, color: UIKitStyle.muted)
    private let volume = UIKitStyle.label(size: 12, weight: .bold)
    private let slider = UISlider()
    private let localMute = UISwitch()
    private let controls = UIStackView()
    private let audio: CaperButton
    init(voice: VoiceClient, id: String, toggle: @escaping () -> Void) {
        self.voice = voice; self.id = id; audio = CaperButton("Audio ⌄", kind: .compact, size: 11, handler: toggle)
        super.init(frame: .zero); axis = .vertical; spacing = 8
        isLayoutMarginsRelativeArrangement = true; directionalLayoutMargins = NSDirectionalEdgeInsets(top: 8, leading: 8, bottom: 8, trailing: 8)
        let words = UIStackView(arrangedSubviews: [name, status]); words.axis = .vertical; words.spacing = 2
        let header = UIStackView(arrangedSubviews: [avatar, words, audio]); header.spacing = 10; header.alignment = .center; addArrangedSubview(header)
        controls.axis = .vertical; controls.spacing = 6; addArrangedSubview(controls)
        slider.minimumValue = 0; slider.maximumValue = 200; slider.tintColor = UIKitStyle.terracottaBright
        slider.addTarget(self, action: #selector(adjusted), for: .valueChanged)
        let reset = UITapGestureRecognizer(target: self, action: #selector(resetGain)); reset.numberOfTapsRequired = 2; slider.addGestureRecognizer(reset)
        localMute.onTintColor = UIKitStyle.terracotta; localMute.accessibilityLabel = "Mute for me"
        localMute.addTarget(self, action: #selector(muted), for: .valueChanged)
        let muteRow = UIStackView(arrangedSubviews: [UIKitStyle.label("Mute for me", size: 12), localMute]); muteRow.alignment = .center
        [volume, slider, muteRow, UIKitStyle.label("Only changes what you hear.", size: 10, color: UIKitStyle.muted)].forEach(controls.addArrangedSubview)
        accessibilityIdentifier = "voice-participant-\(id)"; audio.accessibilityIdentifier = "participant-audio-\(id)"; layer.cornerRadius = 8
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ person: CallPerson, expanded: Bool) {
        avatar.show(name: person.name, avatarID: person.avatarID, picture: person.picture); name.text = person.name + (person.isSelf ? " (you)" : "")
        status.text = person.locallyMuted ? "You muted \(person.name)" : person.speaking ? "Speaking" : person.deafened ? "Deafened" : person.muted ? "Muted" : ""
        status.accessibilityIdentifier = person.locallyMuted ? "participant-local-muted-\(id)" : "participant-status-\(id)"
        audio.isHidden = person.isSelf; audio.accessibilityLabel = "Audio controls for \(person.name)"; audio.accessibilityValue = expanded ? "Expanded" : "Collapsed"
        controls.isHidden = !expanded; backgroundColor = expanded ? UIKitStyle.raised : .clear
        let gain = voice.participantGains[id] ?? 100
        if !slider.isTracking { slider.value = Float(gain) }; volume.text = "User volume · \(gain)%"; slider.accessibilityLabel = "\(person.name) volume"; slider.accessibilityValue = "\(gain)%"
        localMute.isOn = person.locallyMuted
    }
    @objc private func adjusted() {
        let value = slider.isTracking && abs(slider.value - 100) <= 4 ? 100 : Int(slider.value.rounded())
        slider.value = Float(value); voice.setParticipantGain(value, participantID: id); VoicePresentation.sliderFeedback(Double(value))
    }
    @objc private func resetGain() { slider.value = 100; adjusted() }
    @objc private func muted() { voice.setParticipantMuted(localMute.isOn, participantID: id); VoicePresentation.muteFeedback(localMute.isOn) }
}

final class NativeLiveNowView: UIStackView {
    private let model: AppModel
    private let present: (UIViewController) -> Void
    private let observer = ModelObserver()
    private var rows: [String: NativeChannelView] = [:]
    init(model: AppModel, present: @escaping (UIViewController) -> Void) {
        self.model = model; self.present = present; super.init(frame: .zero); axis = .vertical; spacing = 8
        isLayoutMarginsRelativeArrangement = true; directionalLayoutMargins = NSDirectionalEdgeInsets(top: 10, leading: 0, bottom: 2, trailing: 0)
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func didMoveToWindow() { super.didMoveToWindow(); if window != nil { observer.start { [weak self] in self?.render() } } else { observer.stop() } }
    private static func ids(_ model: AppModel) -> [String] {
        var ids: [String] = []
        if VoicePresentation.active(model.voice), let context = model.voice.context,
           !LiveNow.hasRow(context.channelID, rows: model.detail?.channels.filter(\.joined).map(\.id) ?? []) { ids.append(context.channelID) }
        if let channel = model.visitChannel, VoicePresentation.isVisit(model, channel), !model.voice.isActive(channelID: channel.id),
           !VoicePresentation.occupants(model, channelID: channel.id).isEmpty { ids.append(channel.id) }
        return ids
    }
    static func shows(_ model: AppModel) -> Bool { !ids(model).isEmpty }
    private func render() {
        let ids = Self.ids(model)
        for id in Array(rows.keys) where !ids.contains(id) { rows.removeValue(forKey: id)?.removeFromSuperview() }
        for (index, id) in ids.enumerated() {
            let row = rows[id] ?? NativeChannelView(model: model, id: id, external: true, show: { _ in }, present: present) { [model] in
                if model.voice.context?.channelID == id { Task { await model.openVoiceContext() } } else { model.navigationOpen = false }
            }
            rows[id] = row
            if !arrangedSubviews.indices.contains(index) || arrangedSubviews[index] !== row { removeArrangedSubview(row); insertArrangedSubview(row, at: index) }
        }
    }
}

final class NativeNotificationError: UIStackView {
    private let model: AppModel
    private let text = UIKitStyle.label(size: 11, color: UIKitStyle.error)
    private let observer = ModelObserver()
    init(model: AppModel) {
        self.model = model; super.init(frame: .zero); spacing = 8; alignment = .center
        text.numberOfLines = 0; addArrangedSubview(text)
        let dismiss = CaperButton("Dismiss", kind: .plain, size: 11) { [model] in model.notificationError = nil }
        addArrangedSubview(dismiss); dismiss.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        accessibilityIdentifier = "notification-error"
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func didMoveToWindow() { super.didMoveToWindow(); if window != nil { observer.start { [weak self] in self?.text.text = self?.model.notificationError } } else { observer.stop() } }
}

final class NativeVoiceStripController: UIViewController {
    private let model: AppModel
    private let browse: () -> Void
    private let observer = ModelObserver()
    private let body = UIStackView()
    private let open = UIButton(type: .system)
    private let faces = UIStackView()
    private let clock = UIKitStyle.label(size: 11, color: UIColor(CaperTheme.voiceSessionGreen))
    private lazy var mute = IconButton(icon: "mic", label: "Mute microphone", side: 44) { [model] in VoicePresentation.toggleMute(model.voice) }
    private lazy var join = CaperButton("Join", kind: .primary, size: 12) { [weak self] in
        guard let self, let channel = Self.channel(model) else { return }; ChannelVoiceAction(model: model, channel: channel).join()
    }
    private var height: NSLayoutConstraint!
    private var timer: Timer?
    private var availability: Task<Void, Never>?
    private var channelID: String?
    private var faceKey: [String] = []
    private var lastWidth: CGFloat = 0
    init(model: AppModel, browse: @escaping () -> Void) { self.model = model; self.browse = browse; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { timer?.invalidate(); availability?.cancel() }
    static func channel(_ model: AppModel) -> Channel? {
        if model.visit != nil { return model.visitChannel.flatMap { VoicePresentation.isVisit(model, $0) ? $0 : nil } }
        guard model.selectedDirectMessageID == nil, let channel = model.selectedChannel, channel.joined else { return nil }; return channel
    }
    static func inVoice(_ model: AppModel) -> Int { channel(model).map { VoicePresentation.occupants(model, channelID: $0.id).count } ?? 0 }
    override func loadView() {
        view = UIView(); view.translatesAutoresizingMaskIntoConstraints = false
        view.backgroundColor = UIColor(red: 26/255, green: 38/255, blue: 34/255, alpha: 1); view.clipsToBounds = true
        view.accessibilityIdentifier = "voice-strip"
        body.spacing = 6; body.alignment = .center; body.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(body)
        [open, faces, clock, mute, join].forEach(body.addArrangedSubview)
        faces.spacing = -6; open.titleLabel?.font = UIKitStyle.font(13, .bold); open.titleLabel?.lineBreakMode = .byTruncatingTail
        open.contentHorizontalAlignment = .leading; open.tintColor = UIKitStyle.text
        open.setImage(UIKitStyle.icon("volume-2")?.resized(to: 14), for: .normal)
        open.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        open.setContentHuggingPriority(.defaultLow, for: .horizontal)
        open.addAction(UIAction { [browse] _ in if !SwipeTapGuard.swiping { browse() } }, for: .primaryActionTriggered)
        join.addAction(UIAction { [model] _ in if let channel = Self.channel(model) { model.prepareVoiceJoin(channel: channel) } }, for: .touchDown)
        mute.accessibilityIdentifier = "voice-strip-mute"; join.accessibilityIdentifier = "voice-strip-join"
        height = view.heightAnchor.constraint(equalToConstant: 0); height.isActive = true
        NSLayoutConstraint.activate([
            body.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 13), body.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -6),
            body.centerYAnchor.constraint(equalTo: view.centerYAnchor), body.heightAnchor.constraint(equalToConstant: 56),
        ])
    }
    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated); observer.start { [weak self] in self?.render() }
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.updateClock() } }
    }
    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        if lastWidth != view.bounds.width { lastWidth = view.bounds.width; render() }
    }
    override func viewDidDisappear(_ animated: Bool) { super.viewDidDisappear(animated); observer.stop(); timer?.invalidate(); timer = nil; availability?.cancel(); channelID = nil }
    private func updateClock() { clock.text = VoicePresentation.startedAt(model).map { VoicePresentation.duration(startedAt: $0, now: .now) } }
    private func render() {
        let channel = Self.channel(model)
        if channelID != channel?.id {
            channelID = channel?.id; availability?.cancel()
            if let channel { availability = Task { [model] in await model.refreshVoiceAvailability(channel: channel) } }
        }
        let own = VoicePresentation.active(model.voice) && model.voice.context != nil
        let occupants = channel.map { VoicePresentation.occupants(model, channelID: $0.id) } ?? []
        let visible = own || !occupants.isEmpty
        height.constant = visible ? 56 : 0; view.accessibilityElementsHidden = !visible
        open.isEnabled = own; mute.isHidden = !own; join.isHidden = own; clock.isHidden = !own || view.bounds.width < 360
        if own, let context = model.voice.context {
            let pending = VoicePresentation.pending(model.voice)
            open.setTitle(" #\(context.channelName)\(pending.map { " · \($0)" } ?? "")", for: .normal)
            open.accessibilityLabel = "Voice in #\(context.channelName)"; open.accessibilityValue = [pending, LiveNow.people(VoicePresentation.people(model).count)].compactMap { $0 }.joined(separator: ", ")
            open.accessibilityIdentifier = "voice-strip-open"
        } else {
            open.setTitle(" \(occupants.count) talking", for: .normal); open.accessibilityLabel = "\(occupants.count) talking"; open.accessibilityValue = nil; open.accessibilityIdentifier = "voice-strip-count"
            if let channel { let action = ChannelVoiceAction(model: model, channel: channel); join.setTitle(action.title); join.isEnabled = action.enabled; join.accessibilityLabel = action.description; join.accessibilityHint = action.help }
        }
        mute.setImage(UIKitStyle.icon(model.voice.muted ? "mic-off" : "mic")?.resized(to: 18), for: .normal)
        mute.accessibilityLabel = model.voice.muted ? "Unmute microphone" : "Mute microphone"; mute.accessibilityValue = model.voice.muted ? "Muted" : "On"
        let people = VoicePresentation.people(model)
        let faceData: [(String, String, Int?, CaperPicture?)] = own ? people.map { ($0.id, $0.name, $0.avatarID, $0.picture) } : occupants.prefix(2).map { ($0.id, $0.name, $0.avatarId, $0.avatarPicture) }
        let slots = view.bounds.width >= 460 ? 5 : view.bounds.width >= 360 ? 3 : 2
        let key = [String(slots)] + faceData.map { "\($0.0)|\($0.1)|\(String(describing: $0.2))|\(String(describing: $0.3))" }
        if key != faceKey {
            faceKey = key; faces.arrangedSubviews.forEach { $0.removeFromSuperview() }
            let split = LiveNow.fold(faceData, slots: slots)
            for item in split.shown { let avatar = AvatarView(side: 24); avatar.show(name: item.1, avatarID: item.2, picture: item.3); faces.addArrangedSubview(avatar) }
            if !split.folded.isEmpty { faces.addArrangedSubview(UIKitStyle.label("+\(split.folded.count)", size: 11, weight: .bold)) }
        }
        updateClock()
    }
}
