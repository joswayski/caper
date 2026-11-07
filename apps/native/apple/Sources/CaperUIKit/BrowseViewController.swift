import CaperCore
import SwiftUI
import UIKit

/// Browse on iPhone: the space rail beside the selected space's channels and
/// direct messages, with the account bar beneath. The conversation slides over it.
final class BrowseViewController: UIViewController, UICollectionViewDelegate {
    private enum Item: Hashable {
        case notificationError, liveNow, channelsHeader, channel(String), search, browseResult(String), searchEmpty, channelInvitation(String), listError
        case directHeading, activity, feedback, requests, selfDirect, direct(String), newMessage
    }

    private let model: AppModel
    var cached: StartupSnapshot? { didSet { if isViewLoaded { render() } } }
    /// A channel was chosen: show the conversation.
    var opened: (() -> Void)?

    private let rail = UIStackView()
    private let railScroll = UIScrollView()
    private let card = UIView()
    private let spaceName = UIKitStyle.label(size: 15, weight: .bold)
    /// SwiftUI's narrow "Close navigation": back to the open conversation.
    private lazy var closeNavigation = IconButton(icon: "x", label: "Close navigation", size: 16, side: 44) { [weak self] in self?.opened?() }
    private var closeWidth: NSLayoutConstraint!
    private let headerRule = UIView()
    private let spaceMenu = UIButton(type: .custom)
    private let spaceChevron = UIImageView(image: UIKitStyle.icon("chevron-down")?.resized(to: 16))
    /// Channels above SwiftUI's draggable divider, direct messages below it,
    /// each scrolling on its own.
    private var channels: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, Item>!
    private var directs: UICollectionView!
    private var directSource: UICollectionViewDiffableDataSource<Int, Item>!
    private let divider = SectionDivider()
    private var channelsHeight: NSLayoutConstraint!
    private static let shareKey = "caper.channelSidebarChannelsShare"
    private var channelsShare = UserDefaults.standard.object(forKey: shareKey) == nil ? 0.5 : UserDefaults.standard.double(forKey: shareKey)
    private var dragShare: Double?
    private var dragStart: Double?
    /// SwiftUI's Channels toggle: collapsed keeps only live rows.
    private var channelsExpanded = true
    private var channelRows: [String: Channel] = [:]
    private var selectedChannelID: String?
    private var unread: Set<String> = []
    private var directRows: [String: DirectMessageConversation] = [:]
    private var channelInvitations: [String: ChannelInvitation] = [:]
    /// Browse channels: every channel in the space, searchable, joined or not.
    private var browsingChannels = false
    private var channelSearch = ""
    private var browseRows: [String: Channel] = [:]
    private var browsedSpaceID: String?
    /// Message requests, pushed over the lists as SwiftUI does on iPhone.
    private lazy var requests: UIHostingController<UIKitMessageRequests> = {
        let host = UIHostingController(rootView: UIKitMessageRequests(model: model))
        host.view.backgroundColor = UIKitStyle.sidebar
        host.view.translatesAutoresizingMaskIntoConstraints = false
        host.view.accessibilityIdentifier = "message-requests-page"
        return host
    }()
    /// The feedback team's inbox, pushed the same way; it wins over requests.
    private lazy var feedback: UIHostingController<UIKitFeedbackInbox> = {
        let host = UIHostingController(rootView: UIKitFeedbackInbox(model: model))
        host.view.backgroundColor = UIKitStyle.sidebar
        host.view.translatesAutoresizingMaskIntoConstraints = false
        host.view.accessibilityIdentifier = "feedback-page"
        return host
    }()
    private let accountAvatar = AvatarView(side: 30)
    private let accountName = UIKitStyle.label(size: 13, weight: .medium)
    private let presenceDot = UIView()
    private let voiceError = UIStackView()
    private let voiceErrorText = UIKitStyle.label(size: 11, color: UIKitStyle.error)
    private lazy var accountMenu: UIButton = {
        let button = UIButton(type: .system)
        button.setImage(UIKitStyle.icon("settings")?.resized(to: 20), for: .normal)
        button.tintColor = UIKitStyle.text
        button.accessibilityLabel = "Account settings"
        button.accessibilityIdentifier = "account-settings"
        button.showsMenuAsPrimaryAction = true
        button.menu = UIMenu(children: [UIDeferredMenuElement.uncached { [weak self] completion in
            completion(self?.accountMenuItems() ?? [])
        }])
        button.translatesAutoresizingMaskIntoConstraints = false
        return button
    }()
    private let observer = ModelObserver()

    /// SwiftUI's iPhone account menu: audio, notifications, privacy, log out.
    private func accountMenuItems() -> [UIMenuElement] {
        guard live else { return [] }
        let model = self.model
        // SwiftUI's iPhone "Audio settings" section.
        let effects = UIAction(title: "Caper sound effects", state: UIKitAudioSettings.soundEffects ? .on : .off) { _ in
            UIKitAudioSettings.soundEffects.toggle()
        }
        effects.accessibilityIdentifier = "sound-effects"
        let sendOnStop = UIAction(title: VoiceMessage.sendOnStopTitle, state: UIKitAudioSettings.sendVoiceOnStop ? .on : .off) { _ in
            UIKitAudioSettings.sendVoiceOnStop.toggle()
        }
        sendOnStop.accessibilityIdentifier = "voice-messages-send-on-stop"
        var items: [UIMenuElement] = [UIMenu(title: "Audio settings", options: .displayInline, children: [effects, sendOnStop]),
                                      UIAction(title: "Audio test") { [weak self] _ in self?.dialog(.audio) }]
        if model.voice.phase == .connected {
            items.append(UIAction(title: "Connection details") { [weak self] _ in self?.dialog(.connection) })
        }
        if model.account?.debugEnabled == true {
            items.append(UIAction(title: "Audio diagnostics") { [weak self] _ in self?.dialog(.diagnostics) })
        }
        let whatsNew = UIAction(title: "What’s new") { _ in UIKitWhatsNew.open(model: model) }
        whatsNew.accessibilityIdentifier = "open-whats-new"
        items.append(whatsNew)
        let notifications = UIAction(title: "Notifications") { [weak self] _ in self?.dialog(.notifications) }
        notifications.accessibilityIdentifier = "open-notifications"
        let privacy = UIAction(title: "Privacy and blocked accounts") { [weak self] _ in self?.dialog(.privacy) }
        privacy.accessibilityIdentifier = "open-privacy"
        items += [notifications, privacy]
        items.append(UIMenu(options: .displayInline, children: [
            UIAction(title: "Log out", image: UIKitStyle.icon("log-out"), attributes: .destructive) { _ in Task { await model.logout() } },
        ]))
        return items
    }

    init(model: AppModel) {
        self.model = model
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.blackout
        buildRail()
        buildCard()
        buildAccountBar()
        observer.start { [weak self] in self?.render() }
    }

    private func buildRail() {
        railScroll.translatesAutoresizingMaskIntoConstraints = false
        railScroll.showsVerticalScrollIndicator = false
        rail.axis = .vertical
        rail.spacing = 10
        rail.alignment = .center
        rail.translatesAutoresizingMaskIntoConstraints = false
        railScroll.addSubview(rail)
        view.addSubview(railScroll)
        // A group, so VoiceOver names the column when focus enters it.
        railScroll.accessibilityContainerType = .semanticGroup
        railScroll.accessibilityLabel = "Spaces"
        NSLayoutConstraint.activate([
            railScroll.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            railScroll.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            railScroll.widthAnchor.constraint(equalToConstant: 60),
            rail.topAnchor.constraint(equalTo: railScroll.contentLayoutGuide.topAnchor, constant: 14),
            rail.bottomAnchor.constraint(equalTo: railScroll.contentLayoutGuide.bottomAnchor, constant: -14),
            rail.centerXAnchor.constraint(equalTo: railScroll.frameLayoutGuide.centerXAnchor),
        ])
    }

    private func buildCard() {
        card.backgroundColor = UIKitStyle.sidebar
        card.layer.cornerRadius = 16
        card.layer.cornerCurve = .continuous
        card.clipsToBounds = true
        card.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(card)
        // The space name is its menu, as on SwiftUI: notifications, invite, settings, leave.
        spaceName.isAccessibilityElement = false
        spaceChevron.tintColor = UIKitStyle.muted
        spaceChevron.translatesAutoresizingMaskIntoConstraints = false
        spaceMenu.translatesAutoresizingMaskIntoConstraints = false
        spaceMenu.showsMenuAsPrimaryAction = true
        spaceMenu.accessibilityIdentifier = "selected-space-name"
        spaceMenu.accessibilityTraits = [.header, .button]
        spaceMenu.menu = UIMenu(children: [UIDeferredMenuElement.uncached { [weak self] completion in
            completion(self?.spaceMenuItems() ?? [])
        }])
        closeNavigation.tintColor = UIKitStyle.muted
        closeNavigation.translatesAutoresizingMaskIntoConstraints = false
        headerRule.backgroundColor = UIKitStyle.border
        headerRule.translatesAutoresizingMaskIntoConstraints = false

        var list = UICollectionLayoutListConfiguration(appearance: .plain)
        list.backgroundColor = .clear
        list.showsSeparators = false
        channels = UICollectionView(frame: .zero, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: list))
        channels.backgroundColor = .clear
        channels.delegate = self
        channels.translatesAutoresizingMaskIntoConstraints = false
        channels.accessibilityIdentifier = "channel-list"
        directs = UICollectionView(frame: .zero, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: list))
        directs.backgroundColor = .clear
        directs.delegate = self
        directs.translatesAutoresizingMaskIntoConstraints = false
        directs.accessibilityIdentifier = "direct-message-list"
        directs.contentInset.top = 6
        // Joined channels are SwiftUI's own rows: name, unread and mute, quick
        // join and ⋯ options, growing into a voice card in place when live.
        let registration = UICollectionView.CellRegistration<UICollectionViewListCell, Item> { [weak self] cell, _, item in
            guard let self else { return }
            cell.backgroundConfiguration = .clear()
            switch item {
            case let .channel(id):
                guard let channel = self.channelRows[id] else { return }
                cell.contentConfiguration = UIHostingConfiguration {
                    UIKitChannelRow(model: self.model, channel: channel) { [weak self] in self?.dialog($0) }
                }.margins(.horizontal, 16).margins(.vertical, 0)
            case .liveNow:
                cell.contentConfiguration = UIHostingConfiguration { UIKitLiveNow(model: self.model) }.margins(.horizontal, 16).margins(.vertical, 0)
            case .notificationError:
                cell.contentConfiguration = UIHostingConfiguration { UIKitNotificationErrorRow(model: self.model) }.margins(.all, 0)
            default:
                break
            }
        }
        let searchRegistration = UICollectionView.CellRegistration<SearchFieldCell, Item> { [weak self] cell, _, _ in
            cell.changed = { [weak self] text in self?.channelSearch = text; self?.render() }
        }
        let resultRegistration = UICollectionView.CellRegistration<BrowseResultCell, Item> { [weak self] cell, _, item in
            guard let self, case let .browseResult(id) = item, let channel = self.browseRows[id] else { return }
            cell.show(channel, unread: self.model.channelUnread(channel.id), owner: self.model.isOwner) { [weak self] in
                self?.dialog(.manageChannel(channel))
            }
        }
        let invitationRegistration = UICollectionView.CellRegistration<DirectRowCell, Item> { [weak self] cell, _, item in
            guard let self, case let .channelInvitation(id) = item, let invitation = self.channelInvitations[id] else { return }
            cell.show(icon: "lock", text: invitation.channel.name, highlighted: true, count: "Invite", countStyle: .neutral)
            cell.isAccessibilityElement = true
            cell.accessibilityLabel = "Invitation to private channel \(invitation.channel.name) from \(invitation.inviter.displayName)"
            cell.accessibilityTraits = .button
            cell.accessibilityIdentifier = "channel-invitation-\(invitation.id)"
        }
        let directRegistration = UICollectionView.CellRegistration<DirectRowCell, Item> { [weak self] cell, _, item in
            self?.configure(cell, item: item)
        }
        let headerRegistration = UICollectionView.CellRegistration<ChannelsHeaderCell, Item> { [weak self] cell, _, _ in
            self?.configure(cell)
        }
        let provider: UICollectionViewDiffableDataSource<Int, Item>.CellProvider = { collection, path, item in
            switch item {
            case .channelsHeader: return collection.dequeueConfiguredReusableCell(using: headerRegistration, for: path, item: item)
            case .channel, .liveNow, .notificationError: return collection.dequeueConfiguredReusableCell(using: registration, for: path, item: item)
            case .search: return collection.dequeueConfiguredReusableCell(using: searchRegistration, for: path, item: item)
            case .browseResult: return collection.dequeueConfiguredReusableCell(using: resultRegistration, for: path, item: item)
            default: break
            }
            if case .channelInvitation = item { return collection.dequeueConfiguredReusableCell(using: invitationRegistration, for: path, item: item) }
            return collection.dequeueConfiguredReusableCell(using: directRegistration, for: path, item: item)
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: channels, cellProvider: provider)
        directSource = UICollectionViewDiffableDataSource(collectionView: directs, cellProvider: provider)
        buildDivider()
        [spaceName, spaceChevron, spaceMenu, closeNavigation, headerRule, channels, divider, directs].forEach(card.addSubview)
        closeWidth = closeNavigation.widthAnchor.constraint(equalToConstant: 44)
        channelsHeight = channels.heightAnchor.constraint(equalToConstant: 200)
        NSLayoutConstraint.activate([
            card.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 8),
            card.leadingAnchor.constraint(equalTo: railScroll.trailingAnchor),
            card.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -8),
            railScroll.bottomAnchor.constraint(equalTo: card.bottomAnchor),
            // SwiftUI's 50pt header: name and chevron (the space menu), then X, over a rule.
            spaceName.centerYAnchor.constraint(equalTo: card.topAnchor, constant: 25),
            spaceName.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 16),
            spaceName.trailingAnchor.constraint(lessThanOrEqualTo: spaceChevron.leadingAnchor, constant: -6),
            spaceChevron.centerYAnchor.constraint(equalTo: spaceName.centerYAnchor),
            spaceChevron.trailingAnchor.constraint(equalTo: closeNavigation.leadingAnchor, constant: -6),
            closeNavigation.trailingAnchor.constraint(equalTo: card.trailingAnchor, constant: -6),
            closeNavigation.centerYAnchor.constraint(equalTo: spaceName.centerYAnchor),
            closeNavigation.heightAnchor.constraint(equalToConstant: 44),
            closeWidth,
            spaceMenu.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 8),
            spaceMenu.trailingAnchor.constraint(equalTo: spaceChevron.trailingAnchor, constant: 4),
            spaceMenu.centerYAnchor.constraint(equalTo: spaceName.centerYAnchor),
            spaceMenu.heightAnchor.constraint(equalToConstant: 44),
            headerRule.topAnchor.constraint(equalTo: card.topAnchor, constant: 50),
            headerRule.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            headerRule.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            headerRule.heightAnchor.constraint(equalToConstant: 1),
            channels.topAnchor.constraint(equalTo: headerRule.bottomAnchor),
            channels.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            channels.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            channelsHeight,
            divider.topAnchor.constraint(equalTo: channels.bottomAnchor),
            divider.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            divider.heightAnchor.constraint(equalToConstant: Self.dividerHeight),
            directs.topAnchor.constraint(equalTo: divider.bottomAnchor),
            directs.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            directs.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            directs.bottomAnchor.constraint(equalTo: card.bottomAnchor),
        ])
    }

    // MARK: Sections divider

    private static let dividerHeight: CGFloat = 24
    /// Points the two lists share, below the header and above the card's end.
    private var available: Double { max(0, Double(card.bounds.height) - 51 - (model.account == nil ? 0 : Double(Self.dividerHeight))) }
    private var lowerShare: Double { min(96, available / 3) / max(available, 1) }
    private var split: Double { min(max(dragShare ?? channelsShare, lowerShare), 1 - lowerShare) }

    private func buildDivider() {
        divider.translatesAutoresizingMaskIntoConstraints = false
        divider.addGestureRecognizer(UIPanGestureRecognizer(target: self, action: #selector(dragDivider(_:))))
        let reset = UITapGestureRecognizer(target: self, action: #selector(resetDivider))
        reset.numberOfTapsRequired = 2
        divider.addGestureRecognizer(reset)
        divider.step = { [weak self] down in
            guard let self else { return }
            let delta = 10 / max(self.available, 1)
            self.setShare(down ? self.split + delta : self.split - delta)
        }
    }

    @objc private func dragDivider(_ pan: UIPanGestureRecognizer) {
        switch pan.state {
        case .began: dragStart = split
        case .changed:
            dragShare = min(max((dragStart ?? split) + Double(pan.translation(in: card).y) / max(available, 1), lowerShare), 1 - lowerShare)
            layoutSplit()
        default:
            if let dragShare { setShare(dragShare) }
            dragShare = nil; dragStart = nil
        }
    }

    @objc private func resetDivider() { setShare(0.5) }

    private func setShare(_ share: Double) {
        channelsShare = min(max(share, lowerShare), 1 - lowerShare)
        UserDefaults.standard.set(channelsShare, forKey: Self.shareKey)
        layoutSplit()
    }

    private func layoutSplit() {
        let hasDirects = model.account != nil && live
        divider.isHidden = !hasDirects
        directs.isHidden = !hasDirects
        let height = hasDirects ? available * split : max(0, Double(card.bounds.height) - 51)
        if abs(channelsHeight.constant - height) > 0.5 { channelsHeight.constant = height }
        divider.accessibilityValue = "\(Int((split * 100).rounded())) percent channels"
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        layoutSplit()
    }

    /// Channels, its count, and the owner's Create channel and ⋯ menu.
    private func configure(_ cell: ChannelsHeaderCell) {
        let live = self.live
        let count = (live ? model.detail : cached?.detail)?.channels.filter(\.joined).count ?? 0
        let owner = live && model.isOwner
        cell.configure(count: count, expanded: channelsExpanded, owner: owner, canCreate: model.canCreateChannel)
        cell.toggled = { [weak self] in
            guard let self else { return }
            self.channelsExpanded.toggle()
            UIKitBrowse.playToggle(self.channelsExpanded)
            self.render()
        }
        cell.create = { [weak self] in self?.dialog(.createChannel) }
        cell.menu = UIMenu(children: [UIDeferredMenuElement.uncached { [weak self] completion in
            guard let self else { return completion([]) }
            let create = UIAction(title: "Create channel", attributes: self.model.canCreateChannel ? [] : .disabled) { [weak self] _ in
                self?.dialog(.createChannel)
            }
            let fold = UIAction(title: "\(self.channelsExpanded ? "Collapse" : "Expand") channels") { [weak self] _ in
                guard let self else { return }
                self.channelsExpanded.toggle()
                self.render()
            }
            completion([create, fold])
        }])
    }

    /// SwiftUI's phone account bar: a bordered raised box with your voice
    /// error (dismissible) over the avatar and name, which open Profile, and
    /// the account menu.
    private func buildAccountBar() {
        let box = UIStackView()
        box.axis = .vertical
        box.backgroundColor = UIKitStyle.raised
        box.layer.cornerRadius = 6
        box.layer.borderWidth = 1
        box.layer.borderColor = UIKitStyle.border.cgColor
        box.clipsToBounds = true
        box.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(box)

        voiceErrorText.numberOfLines = 0
        let dismiss = IconButton(icon: "x", label: "Dismiss voice error", size: 14, side: 44) { [weak self] in self?.model.voice.error = nil }
        dismiss.tintColor = UIKitStyle.muted
        [voiceErrorText, dismiss].forEach(voiceError.addArrangedSubview)
        voiceError.alignment = .center
        voiceError.spacing = 8
        voiceError.isLayoutMarginsRelativeArrangement = true
        voiceError.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: 9, bottom: 0, trailing: 0)
        voiceError.accessibilityIdentifier = "voice-error"
        voiceError.isHidden = true

        let row = UIView()
        accountName.lineBreakMode = .byTruncatingTail
        // SwiftUI's PresenceDot: a 10pt dot under a 2pt centred stroke, over
        // the avatar's corner; outside the avatar, which clips to its circle.
        presenceDot.layer.cornerRadius = 5.5
        presenceDot.layer.borderWidth = 2
        presenceDot.layer.borderColor = UIKitStyle.raised.cgColor
        presenceDot.translatesAutoresizingMaskIntoConstraints = false
        let identity = UIStackView(arrangedSubviews: [accountAvatar, accountName])
        identity.spacing = 7
        identity.alignment = .center
        identity.isUserInteractionEnabled = false
        identity.translatesAutoresizingMaskIntoConstraints = false
        let profile = UIButton(type: .custom)
        profile.translatesAutoresizingMaskIntoConstraints = false
        profile.addSubview(identity)
        presenceDot.isUserInteractionEnabled = false
        profile.addSubview(presenceDot)
        profile.accessibilityIdentifier = "account-profile"
        profile.addAction(UIAction { [weak self] _ in self?.dialog(.profile) }, for: .primaryActionTriggered)
        self.profile = profile
        row.addSubview(profile)
        row.addSubview(accountMenu)
        accountMenu.tintColor = UIKitStyle.muted
        [voiceError, row].forEach(box.addArrangedSubview)
        NSLayoutConstraint.activate([
            box.topAnchor.constraint(equalTo: card.bottomAnchor, constant: 12),
            box.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 12),
            box.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            box.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -16),
            row.heightAnchor.constraint(equalToConstant: 52),
            presenceDot.widthAnchor.constraint(equalToConstant: 11),
            presenceDot.heightAnchor.constraint(equalToConstant: 11),
            presenceDot.trailingAnchor.constraint(equalTo: accountAvatar.trailingAnchor, constant: 0.5),
            presenceDot.bottomAnchor.constraint(equalTo: accountAvatar.bottomAnchor, constant: 0.5),
            profile.leadingAnchor.constraint(equalTo: row.leadingAnchor, constant: 4),
            profile.topAnchor.constraint(equalTo: row.topAnchor, constant: 4),
            profile.bottomAnchor.constraint(equalTo: row.bottomAnchor, constant: -4),
            profile.trailingAnchor.constraint(equalTo: accountMenu.leadingAnchor, constant: -5),
            identity.leadingAnchor.constraint(equalTo: profile.leadingAnchor),
            identity.centerYAnchor.constraint(equalTo: profile.centerYAnchor),
            identity.trailingAnchor.constraint(lessThanOrEqualTo: profile.trailingAnchor),
            accountMenu.trailingAnchor.constraint(equalTo: row.trailingAnchor, constant: -4),
            accountMenu.centerYAnchor.constraint(equalTo: row.centerYAnchor),
            accountMenu.widthAnchor.constraint(equalToConstant: 44),
            accountMenu.heightAnchor.constraint(equalToConstant: 44),
        ])
    }
    private var profile: UIButton!

    // MARK: Rendering

    private var live: Bool { model.phase == .ready && !model.restoringWorkspace }

    private func render() {
        let live = self.live
        let spaces = live ? model.spaces : cached?.spaces ?? []
        let detail = live ? model.detail : cached?.detail
        let selectedSpace = live ? model.selectedSpaceID : cached?.detail?.space.id
        selectedChannelID = live ? model.selectedChannelID : cached?.channelID
        unread = live ? Set(model.unreadSpaces.flatMap(\.channels).map(\.id)) : []
        renderRail(spaces, selected: selectedSpace, unreadSpaces: live ? Set(model.unreadSpaces.map(\.id)) : [])

        spaceName.text = detail.map { $0.space.demo == true ? "Caper" : $0.space.name } ?? ""
        spaceMenu.accessibilityLabel = spaceName.text
        let menuable = live && detail?.space.demo != true && model.account != nil
        spaceMenu.isEnabled = menuable
        spaceChevron.isHidden = !menuable
        // The X goes back to a conversation, so it shows only when one is open.
        let canClose = live && (model.selectedChannelID != nil || model.selectedDirectMessageID != nil || model.visit != nil)
        closeNavigation.isHidden = !canClose
        closeWidth.constant = canClose ? 44 : 0
        let joined = live ? UIKitBrowse.listedChannels(model, expanded: channelsExpanded) : detail?.channels.filter(\.joined) ?? []
        let previous = channelRows
        let previousBrowse = browseRows
        channelRows = Dictionary(uniqueKeysWithValues: joined.map { ($0.id, $0) })
        var snapshot = NSDiffableDataSourceSnapshot<Int, Item>()
        snapshot.appendSections([0])
        if live && model.notificationError != nil { snapshot.appendItems([.notificationError]) }
        if live && UIKitLiveNow.shows(model) { snapshot.appendItems([.liveNow]) }
        snapshot.appendItems([.channelsHeader])
        snapshot.reconfigureItems(dataSource.snapshot().indexOfItem(.channelsHeader) != nil ? [.channelsHeader] : [])
        snapshot.appendItems(joined.map { .channel($0.id) })
        // Browse channels: a search field, then every matching channel.
        if live, selectedSpace != browsedSpaceID { browsedSpaceID = selectedSpace; browsingChannels = false; channelSearch = "" }
        if live && browsingChannels {
            let search = channelSearch.trimmingCharacters(in: .whitespacesAndNewlines)
            let results = (detail?.channels ?? []).filter { search.isEmpty || $0.name.localizedCaseInsensitiveContains(search) }
            browseRows = Dictionary(results.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            snapshot.appendItems([.search] + results.map { .browseResult($0.id) })
            if results.isEmpty {
                snapshot.appendItems([.searchEmpty])
                snapshot.reconfigureItems(dataSource.snapshot().indexOfItem(.searchEmpty) != nil ? [.searchEmpty] : [])
            }
            snapshot.reconfigureItems(results.filter { previousBrowse[$0.id] != nil }.map { .browseResult($0.id) })
        } else { browseRows = [:] }
        // Private channel invitations, under the channels.
        let invitations = live ? detail?.channelInvitations ?? [] : []
        channelInvitations = Dictionary(invitations.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        snapshot.appendItems(invitations.map { .channelInvitation($0.id) })
        // Selection and unread state live in the cell; reconfigure all visible rows cheaply.
        // Hosted rows observe the model themselves; reconfigure only a changed channel.
        snapshot.reconfigureItems(joined.filter { previous[$0.id] != nil && previous[$0.id] != $0 }.map { .channel($0.id) })
        listError = live ? model.error ?? model.directMessagesError : nil
        if listError != nil {
            snapshot.appendItems([.listError])
            snapshot.reconfigureItems(dataSource.snapshot().indexOfItem(.listError) != nil ? [.listError] : [])
        }
        dataSource.apply(snapshot, animatingDifferences: false)
        var directSnapshot = NSDiffableDataSourceSnapshot<Int, Item>()
        appendDirect(to: &directSnapshot, live: live)
        directSource.apply(directSnapshot, animatingDifferences: false)
        layoutSplit()
        let showingFeedback = live && model.isFeedbackTeam && model.showingFeedback
        render(page: feedback, showing: showingFeedback)
        render(page: requests, showing: !showingFeedback && live && model.account != nil && model.showingMessageRequests)
        [spaceMenu, closeNavigation, channels, divider, directs].forEach { $0.accessibilityElementsHidden = feedback.parent != nil || requests.parent != nil }

        let account = model.account
        let name = account?.displayName ?? account?.username ?? ""
        accountName.text = name
        accountAvatar.show(name: name, avatarID: account?.avatarId, picture: account?.avatarPicture)
        profile.accessibilityLabel = name.isEmpty ? "Sign in to edit your profile" : "Edit profile for \(name)"
        let presence = live ? model.ownPresence : nil
        presenceDot.backgroundColor = switch presence {
        case .online?: UIKitStyle.green
        case .idle?: UIColor(red: 0.72, green: 0.60, blue: 0.35, alpha: 1)
        default: UIKitStyle.border
        }
        profile.accessibilityValue = presence.map { $0.rawValue.prefix(1).uppercased() + $0.rawValue.dropFirst() } ?? "Status unavailable"
        let error = live ? model.voice.error : nil
        voiceErrorText.text = error
        voiceError.isHidden = error == nil
    }

    private var railKey: [String] = []
    private var listError: String?

    private func renderRail(_ spaces: [Space], selected: String?, unreadSpaces: Set<String>) {
        // Live tiles are SwiftUI's and follow selection, unread and voice themselves.
        let key = spaces.map { live ? "\($0.id)|\($0.name)|\($0.iconAsset ?? "")"
            : "\($0.id)|\($0.name)|\($0.id == selected)|\(unreadSpaces.contains($0.id))|\($0.iconAsset ?? "")" }
            + [live ? "live" : "", "\(model.canCreateSpace)"] + (live ? model.invitations.map { "invite|\($0.id)|\($0.name)" } : [])
        guard key != railKey else { return }
        railKey = key
        rail.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for space in spaces {
            if live {
                let tile = HostedView()
                tile.show(UIKitSpaceRailTile(model: model, space: space))
                NSLayoutConstraint.activate([tile.widthAnchor.constraint(equalToConstant: 44), tile.heightAnchor.constraint(equalToConstant: 44)])
                rail.addArrangedSubview(tile)
                continue
            }
            // The cached first paint: a plain tile until the live workspace is in.
            let name = space.demo == true ? "Caper" : space.name
            let tile = UIButton(type: .custom)
            tile.translatesAutoresizingMaskIntoConstraints = false
            // The owner's icon over the initial, once it loads.
            if let picture = space.iconPicture {
                let icon = AvatarView(side: 44, corner: 12)
                icon.show(name: name, avatarID: nil, picture: picture)
                icon.isUserInteractionEnabled = false
                tile.addSubview(icon)
                NSLayoutConstraint.activate([icon.centerXAnchor.constraint(equalTo: tile.centerXAnchor), icon.centerYAnchor.constraint(equalTo: tile.centerYAnchor)])
            }
            tile.setTitle(String(name.prefix(1)).uppercased(), for: .normal)
            tile.titleLabel?.font = UIKitStyle.font(16, .bold, scaled: false)
            tile.setTitleColor(UIKitStyle.text, for: .normal)
            tile.backgroundColor = UIKitStyle.surface
            tile.layer.cornerRadius = 12
            tile.layer.cornerCurve = .continuous
            let isSelected = space.id == selected
            tile.layer.borderWidth = isSelected ? 2 : 0
            tile.layer.borderColor = UIKitStyle.terracotta.cgColor
            tile.accessibilityLabel = name
            tile.accessibilityValue = [isSelected ? "Selected" : "", unreadSpaces.contains(space.id) ? "Unread messages" : ""]
                .filter { !$0.isEmpty }.joined(separator: ", ")
            if isSelected { tile.accessibilityTraits.insert(.selected) }
            tile.accessibilityIdentifier = "space-\(space.id)"
            tile.addAction(UIAction { [weak self] _ in
                guard let self, self.live, let space = self.model.spaces.first(where: { $0.id == space.id }) else { return }
                Task { await self.model.select(space: space) }
            }, for: .primaryActionTriggered)
            NSLayoutConstraint.activate([tile.widthAnchor.constraint(equalToConstant: 44), tile.heightAnchor.constraint(equalToConstant: 44)])
            rail.addArrangedSubview(tile)
        }
        guard live else { return }
        // Invitations to spaces, dashed; then create or join a space.
        if !model.invitations.isEmpty {
            let heading = UIKitStyle.label("INVITES", size: 8, weight: .bold, color: UIKitStyle.muted)
            heading.accessibilityLabel = "Pending invitations"
            rail.addArrangedSubview(heading)
        }
        for invitation in model.invitations {
            let tile = railTile(String(invitation.name.prefix(1)).uppercased(), dashed: UIKitStyle.terracottaBright)
            tile.accessibilityLabel = "Invitation to \(invitation.name)"
            tile.accessibilityIdentifier = "space-invitation-\(invitation.id)"
            tile.addAction(UIAction { [weak self] _ in self?.dialog(.invitation(invitation)) }, for: .primaryActionTriggered)
            rail.addArrangedSubview(tile)
        }
        let create = railTile(nil, dashed: UIKitStyle.border)
        create.setImage(UIKitStyle.icon("plus")?.resized(to: 20), for: .normal)
        create.tintColor = UIKitStyle.terracottaBright
        create.accessibilityLabel = model.canCreateSpace ? "Create space" : "Join a space"
        create.accessibilityIdentifier = "create-space"
        // Stays enabled at the space limit: the dialog also joins by invite link.
        create.addAction(UIAction { [weak self] _ in self?.dialog(.createSpace) }, for: .primaryActionTriggered)
        rail.addArrangedSubview(create)
    }

    private func railTile(_ title: String?, dashed color: UIColor) -> UIButton {
        let tile = UIButton(type: .custom)
        tile.translatesAutoresizingMaskIntoConstraints = false
        tile.setTitle(title, for: .normal)
        tile.titleLabel?.font = UIKitStyle.font(16, .bold, scaled: false)
        tile.setTitleColor(UIKitStyle.text, for: .normal)
        tile.backgroundColor = UIKitStyle.surface
        tile.layer.cornerRadius = 12
        tile.layer.cornerCurve = .continuous
        let border = CAShapeLayer()
        border.path = UIBezierPath(roundedRect: CGRect(x: 0.5, y: 0.5, width: 43, height: 43), cornerRadius: 12).cgPath
        border.fillColor = nil
        border.strokeColor = color.cgColor
        border.lineDashPattern = [4, 4]
        tile.layer.addSublayer(border)
        NSLayoutConstraint.activate([tile.widthAnchor.constraint(equalToConstant: 44), tile.heightAnchor.constraint(equalToConstant: 44)])
        return tile
    }

    // MARK: Direct messages

    /// Direct messages under the channels: requests, your notes, each DM, New message.
    private func appendDirect(to snapshot: inout NSDiffableDataSourceSnapshot<Int, Item>, live: Bool) {
        guard live, let account = model.account else { directRows = [:]; return }
        let previous = directRows
        let others = model.visibleDirectMessages.filter { $0.peer.id != account.id }
        directRows = Dictionary(model.directMessages.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        // SwiftUI's order: Notifications, Feedback (the team), requests.
        var items: [Item] = [.directHeading, .activity]
        _ = (UIKitActivity.unread(model), model.feedbackWaiting, model.visit?.channelID)
        if model.isFeedbackTeam { items.append(.feedback) }
        if !model.messageRequests.isEmpty { items.append(.requests) }
        items.append(.selfDirect)
        items += others.map { .direct($0.id) }
        items.append(.newMessage)
        snapshot.appendSections([1])
        snapshot.appendItems(items, toSection: 1)
        // Unread, selection and mute live in the cells; reconfigure the survivors.
        snapshot.reconfigureItems(items.filter {
            if case let .direct(id) = $0 { return previous[id] != nil }
            return directSource.snapshot().indexOfItem($0) != nil
        })
    }

    /// A page pushed over the lists (requests, the feedback inbox), as on iPhone.
    private func render(page: UIViewController, showing: Bool) {
        if showing, page.parent == nil {
            addChild(page)
            card.addSubview(page.view)
            NSLayoutConstraint.activate([
                page.view.topAnchor.constraint(equalTo: card.topAnchor, constant: 8),
                page.view.leadingAnchor.constraint(equalTo: card.leadingAnchor),
                page.view.trailingAnchor.constraint(equalTo: card.trailingAnchor),
                page.view.bottomAnchor.constraint(equalTo: card.bottomAnchor),
            ])
            page.didMove(toParent: self)
            UIAccessibility.post(notification: .screenChanged, argument: page.view)
        } else if !showing, page.parent != nil {
            page.willMove(toParent: nil)
            page.view.removeFromSuperview()
            page.removeFromParent()
            UIAccessibility.post(notification: .screenChanged, argument: channels)
        }
    }

    private func configure(_ cell: DirectRowCell, item: Item) {
        switch item {
        case .directHeading:
            cell.heading("Direct messages") { [weak self] in self?.newDirectMessage() }
        case .searchEmpty:
            cell.note(channelSearch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                      ? "This space has no channels yet." : "No channels match your search.", id: "browse-channels-empty")
        case .listError:
            cell.note(listError ?? "", id: "browse-error", color: UIKitStyle.error, size: 11)
        case .activity:
            let unread = UIKitActivity.unread(model)
            cell.show(icon: "bell", text: "Notifications", highlighted: true, unread: unread > 0 ? unread : nil)
            cell.accessibilityLabel = "Notifications"
            cell.accessibilityValue = unread > 0 ? "\(unread) unread" : ""
            cell.accessibilityHint = "Opens your notifications"
            cell.accessibilityIdentifier = "activity-inbox"
        case .feedback:
            let waiting = model.feedbackWaiting
            cell.show(icon: "chevron-right", text: FeedbackInbox.title, highlighted: model.visit != nil,
                      unread: waiting > 0 ? waiting : nil)
            cell.accessibilityLabel = FeedbackInbox.title
            cell.accessibilityValue = waiting > 0 ? "\(waiting) feedback conversations waiting for a reply" : ""
            cell.accessibilityHint = "Opens the feedback inbox"
            cell.accessibilityIdentifier = "feedback-inbox"
        case .requests:
            let count = model.messageRequests.count
            cell.show(icon: "chevron-right", text: "Message requests", count: "\(count)", countStyle: .neutral)
            cell.accessibilityLabel = "Message requests"
            cell.accessibilityValue = "\(count) \(count == 1 ? "request" : "requests")"
            cell.accessibilityHint = "Opens the list of requests"
            cell.accessibilityIdentifier = "message-requests"
        case .selfDirect:
            guard let account = model.account else { return }
            let conversation = model.directMessages.first { $0.peer.id == account.id }
            let name = account.displayName ?? account.username ?? "You"
            let selected = conversation.map { model.selectedDirectMessageID == $0.id } == true
            let unread = conversation?.unread == true
            cell.show(avatar: (name, account.avatarId, account.avatarPicture), text: name, bold: unread,
                      highlighted: unread, selected: selected, tag: "you", unread: unread ? conversation?.unreadCount ?? 0 : nil)
            cell.accessibilityLabel = "\(name), you"
            cell.accessibilityValue = selected ? "Selected" : unread ? "Unread" : ""
            cell.accessibilityIdentifier = "dm-self"
        case let .direct(id):
            guard let conversation = directRows[id] else { return }
            let peer = conversation.peer
            let selected = model.selectedDirectMessageID == id
            let muted = model.notificationsMuted(.direct(id))
            let unread = conversation.unread && !muted && !model.chat.blockedAuthorIDs.contains(peer.id)
            cell.show(avatar: (peer.displayName, peer.avatarId, peer.avatarPicture), text: peer.displayName,
                      bold: unread, highlighted: unread, selected: selected, muted: muted,
                      unread: unread ? conversation.unreadCount ?? 0 : nil)
            let state = selected ? "Selected" : unread ? "Unread" : ""
            cell.accessibilityLabel = peer.displayName
            cell.accessibilityValue = muted ? (state.isEmpty ? "Muted" : state + ", Muted") : state
            cell.accessibilityIdentifier = "dm-\(id)"
        case .newMessage:
            cell.show(icon: "plus", text: "New message")
            cell.accessibilityLabel = "New message"
            cell.accessibilityIdentifier = "new-message"
        case .channel, .channelInvitation, .liveNow, .search, .browseResult, .channelsHeader, .notificationError:
            break
        }
        switch item { case .directHeading, .searchEmpty, .listError: return; default: break }
        cell.isAccessibilityElement = true
        // The value already says "Selected", as SwiftUI's; a selected trait would repeat it.
        cell.accessibilityTraits = .button
    }

    private func newDirectMessage() {
        guard live else { return }
        present(UIKitHosting.newDirectMessage(model: model), animated: true)
    }

    /// Press and hold a DM: notifications on or off, and mute, as on SwiftUI.
    /// Press and hold a channel or DM for its options, as SwiftUI's menus offer.
    func collectionView(_ collectionView: UICollectionView, contextMenuConfigurationForItemsAt indexPaths: [IndexPath],
                        point: CGPoint) -> UIContextMenuConfiguration? {
        guard live, let path = indexPaths.first else { return nil }
        let model = self.model
        switch source(for: collectionView).itemIdentifier(for: path) {
        case let .direct(id):
            model.refreshNotificationSettingsIfStale()
            return UIContextMenuConfiguration(actionProvider: { _ in UIMenu(children: NotificationMenus.direct(model, conversationID: id)) })
        default:
            return nil
        }
    }

    /// The space name's menu: notifications, then Invite and Space settings, or Leave.
    private func spaceMenuItems() -> [UIMenuElement] {
        guard live else { return [] }
        let browse = UIAction(title: browsingChannels ? "Exit Browse channels" : "Browse channels") { [weak self] _ in
            guard let self else { return }
            self.browsingChannels.toggle()
            self.channelSearch = ""
            self.render()
            // The search field sits under the joined channels: bring it into the pane.
            if self.browsingChannels, let path = self.dataSource.indexPath(for: .search) {
                self.channels.scrollToItem(at: path, at: .top, animated: !UIKitStyle.reduceMotion)
            }
        }
        browse.accessibilityIdentifier = "browse-channels"
        var items: [UIMenuElement] = [browse]
        guard let space = model.detail?.space, space.demo != true, model.account != nil else { return items }
        model.refreshNotificationSettingsIfStale()
        items.append(UIMenu(options: .displayInline, children: NotificationMenus.space(model, spaceID: space.id)))
        if model.isOwner {
            items.append(UIMenu(options: .displayInline, children: [
                UIAction(title: "Invite people…") { [weak self] _ in
                    self?.dialog(.invitePeople(spaceID: space.id, channelID: nil))
                },
                UIAction(title: "Space settings") { [weak self] _ in self?.dialog(.manageSpace) },
            ]))
        } else {
            items.append(UIAction(title: "Leave space…", attributes: .destructive) { [weak self] _ in self?.dialog(.leaveSpace) })
        }
        return items
    }

    private func source(for collection: UICollectionView) -> UICollectionViewDiffableDataSource<Int, Item> {
        collection === directs ? directSource : dataSource
    }

    /// A workspace dialog over Browse.
    func dialog(_ item: WorkspaceSheet) {
        guard presentedViewController == nil else { return }
        present(UIKitHosting.workspaceDialog(model: model, item), animated: false)
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: false)
        switch source(for: collectionView).itemIdentifier(for: indexPath) {
        case let .browseResult(id):
            if live, let channel = browseRows[id] { Task { await model.select(channel: channel) } }
        case .channel, .liveNow, .search, .channelsHeader, .searchEmpty, .listError, .notificationError:
            break
        case let .channelInvitation(id):
            if let invitation = channelInvitations[id] { dialog(.channelInvitation(invitation)) }
        case .activity:
            guard presentedViewController == nil else { return }
            present(UIKitActivity.inbox(model: model), animated: !UIKitStyle.reduceMotion)
        case .feedback:
            model.showingFeedback = true
        case .requests:
            model.showingMessageRequests = true
        case .selfDirect:
            // The workspace slides to the conversation once the model selects it.
            Task { await model.openSelfDirectMessage() }
        case let .direct(id):
            guard let conversation = directRows[id] else { return }
            if model.selectedDirectMessageID == id { opened?() }
            Task { await model.select(directMessage: conversation) }
        case .newMessage:
            newDirectMessage()
        case .directHeading, nil:
            break
        }
    }
}

/// A row in Browse's direct-message section: a 24pt avatar or icon, the name,
/// an optional "you" tag, a muted bell and an unread badge or count.
final class DirectRowCell: UICollectionViewListCell {
    enum CountStyle { case neutral, unread }

    private let icon = UIImageView()
    /// SwiftUI's 20pt avatar in a 24pt column, aligned with the icons.
    private let avatar = AvatarView(side: 20)
    private let avatarBox = UIView()
    private let name = UIKitStyle.label(size: 13, weight: .medium)
    private let youTag = UIKitStyle.label(size: 10, weight: .bold, color: UIKitStyle.muted)
    private let bell = UIImageView(image: UIKitStyle.icon("bell-off")?.resized(to: 14))
    private let count = PaddedLabel()
    private let dot = UIView()
    private let stack = UIStackView()
    private var add: IconButton?
    private var addAction: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        icon.tintColor = UIKitStyle.muted
        icon.contentMode = .center
        bell.tintColor = UIKitStyle.muted
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        count.font = UIKitStyle.font(10, .bold)
        count.textColor = UIKitStyle.text
        count.textAlignment = .center
        count.layer.cornerRadius = 9
        count.clipsToBounds = true
        dot.backgroundColor = UIKitStyle.terracottaBright
        dot.layer.cornerRadius = 4
        let spacer = UIView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        avatar.translatesAutoresizingMaskIntoConstraints = false
        avatarBox.addSubview(avatar)
        [icon, avatarBox, name, youTag, bell, spacer, count, dot].forEach(stack.addArrangedSubview)
        stack.spacing = 9
        stack.setCustomSpacing(6, after: name)
        stack.alignment = .center
        stack.isUserInteractionEnabled = false
        stack.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(stack)
        [icon, avatar, name, youTag, bell, count, dot].forEach { $0.isAccessibilityElement = false }
        leading = stack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 25)
        trailing = stack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -25)
        NSLayoutConstraint.activate([
            icon.widthAnchor.constraint(equalToConstant: 24), icon.heightAnchor.constraint(equalToConstant: 24),
            avatarBox.widthAnchor.constraint(equalToConstant: 24), avatarBox.heightAnchor.constraint(equalToConstant: 20),
            avatar.leadingAnchor.constraint(equalTo: avatarBox.leadingAnchor),
            avatar.centerYAnchor.constraint(equalTo: avatarBox.centerYAnchor),
            leading, trailing,
            count.heightAnchor.constraint(equalToConstant: 18), count.widthAnchor.constraint(greaterThanOrEqualToConstant: 20),
            dot.widthAnchor.constraint(equalToConstant: 8), dot.heightAnchor.constraint(equalToConstant: 8),
            stack.topAnchor.constraint(equalTo: contentView.topAnchor),
            stack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            contentView.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    private var leading: NSLayoutConstraint!
    private var trailing: NSLayoutConstraint!

    private func reset() {
        [icon, avatarBox, youTag, bell, count, dot].forEach { $0.isHidden = true }
        leading.constant = 25
        trailing.constant = -25
        name.numberOfLines = 1
        name.isAccessibilityElement = false
        name.accessibilityTraits = []
        accessibilityIdentifier = nil
        add?.isHidden = true
        stack.isUserInteractionEnabled = false
        contentView.alpha = 1
        accessibilityHint = nil
        accessibilityValue = nil
        accessibilityElements = nil
        backgroundConfiguration = .clear()
    }

    /// "DIRECT MESSAGES" with its New direct message button.
    func heading(_ text: String, add action: @escaping () -> Void) {
        reset()
        addAction = action
        if add == nil {
            let button = IconButton(icon: "plus", label: "New direct message", size: 16, side: 36) { [weak self] in self?.addAction?() }
            button.tintColor = UIKitStyle.muted
            button.accessibilityIdentifier = "new-direct-message"
            stack.addArrangedSubview(button)
            add = button
        }
        add?.isHidden = false
        leading.constant = 16
        trailing.constant = -6
        stack.isUserInteractionEnabled = true
        name.text = text
        name.font = UIKitStyle.font(12, .bold)
        name.textColor = UIKitStyle.muted
        name.accessibilityTraits = .header
        name.isAccessibilityElement = true
        name.accessibilityLabel = "Direct messages"
        isAccessibilityElement = false
        accessibilityIdentifier = "direct-messages-heading"
        accessibilityElements = [name, add as Any]
    }

    func show(icon name: String? = nil, avatar: (String, Int?, CaperPicture?)? = nil, text: String, bold: Bool = false,
              highlighted: Bool = false, selected: Bool = false, tag: String? = nil, muted: Bool = false,
              unread: Int? = nil, count text2: String? = nil, countStyle: CountStyle = .unread) {
        reset()
        self.name.isAccessibilityElement = false
        if let name {
            icon.image = UIKitStyle.icon(name)?.resized(to: 16).withRenderingMode(.alwaysTemplate)
            icon.isHidden = false
        }
        if let avatar {
            self.avatar.show(name: avatar.0, avatarID: avatar.1, picture: avatar.2)
            avatarBox.isHidden = false
        }
        self.name.text = text
        self.name.font = UIKitStyle.font(13, bold ? .bold : .medium)
        self.name.textColor = selected || highlighted ? UIKitStyle.text : UIKitStyle.muted
        youTag.text = tag
        youTag.isHidden = tag == nil
        bell.isHidden = !muted
        contentView.alpha = muted ? 0.55 : 1
        if let text2 {
            count.text = text2
            count.backgroundColor = UIKitStyle.raised
            count.layer.borderColor = UIKitStyle.border.cgColor
            count.layer.borderWidth = countStyle == .neutral ? 1 : 0
            count.isHidden = false
        } else if let unread {
            if unread > 0 {
                count.text = unread > 99 ? "99+" : String(unread)
                count.backgroundColor = UIKitStyle.terracotta
                count.layer.borderWidth = 0
                count.isHidden = false
            } else { dot.isHidden = false }
        }
        var background = UIBackgroundConfiguration.listPlainCell()
        background.backgroundColor = selected ? UIKitStyle.terracotta.withAlphaComponent(0.16) : .clear
        background.cornerRadius = 6
        background.backgroundInsets = NSDirectionalEdgeInsets(top: 1, leading: 16, bottom: 1, trailing: 16)
        backgroundConfiguration = background
    }

    /// A line of text in the list: Browse's empty search, or an error.
    func note(_ text: String, id: String, color: UIColor = UIKitStyle.muted, size: CGFloat = 12) {
        reset()
        leading.constant = 16
        trailing.constant = -16
        name.text = text
        name.numberOfLines = 0
        name.font = UIKitStyle.font(size)
        name.textColor = color
        isAccessibilityElement = true
        accessibilityLabel = text
        accessibilityTraits = .staticText
        accessibilityIdentifier = id
    }
}

/// SwiftUI's Channels header: a toggle whose chevron turns ("Channels" and
/// the count), with the owner's Create channel and Channel options.
final class ChannelsHeaderCell: UICollectionViewListCell {
    var toggled: (() -> Void)?
    var create: (() -> Void)?
    var menu: UIMenu? { didSet { options.menu = menu } }
    private let toggle = UIButton(type: .custom)
    private let chevron = UIImageView(image: UIKitStyle.icon("chevron-down")?.resized(to: 16).withRenderingMode(.alwaysTemplate))
    private let title = UIKitStyle.label("Channels", size: 12, weight: .bold, color: UIKitStyle.muted)
    private let count = UIKitStyle.label(size: 10, weight: .bold, color: UIKitStyle.muted)
    private lazy var add = IconButton(icon: "plus", label: "Create channel", size: 16, side: 44) { [weak self] in self?.create?() }
    private let options = UIButton(type: .system)

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundConfiguration = .clear()
        chevron.tintColor = UIKitStyle.muted
        let label = UIStackView(arrangedSubviews: [chevron, title, count])
        label.spacing = 6
        label.alignment = .center
        label.isUserInteractionEnabled = false
        label.translatesAutoresizingMaskIntoConstraints = false
        toggle.addSubview(label)
        toggle.translatesAutoresizingMaskIntoConstraints = false
        toggle.accessibilityIdentifier = "channels-toggle"
        toggle.addAction(UIAction { [weak self] _ in self?.toggled?() }, for: .primaryActionTriggered)
        add.tintColor = UIKitStyle.muted
        add.accessibilityIdentifier = "create-channel"
        options.setImage(UIKitStyle.icon("ellipsis")?.resized(to: 16), for: .normal)
        options.tintColor = UIKitStyle.muted
        options.showsMenuAsPrimaryAction = true
        options.accessibilityLabel = "Channel options"
        options.accessibilityIdentifier = "channel-options"
        options.translatesAutoresizingMaskIntoConstraints = false
        let row = UIStackView(arrangedSubviews: [toggle, UIView(), add, options])
        row.alignment = .center
        row.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(row)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: toggle.leadingAnchor),
            label.trailingAnchor.constraint(equalTo: toggle.trailingAnchor),
            label.centerYAnchor.constraint(equalTo: toggle.centerYAnchor),
            toggle.heightAnchor.constraint(equalToConstant: 44),
            options.widthAnchor.constraint(equalToConstant: 44), options.heightAnchor.constraint(equalToConstant: 44),
            row.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            row.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -6),
            row.topAnchor.constraint(equalTo: contentView.topAnchor),
            row.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(count value: Int, expanded: Bool, owner: Bool, canCreate: Bool) {
        count.text = "\(value)"
        let turn = { self.chevron.transform = expanded ? .identity : CGAffineTransform(rotationAngle: -.pi / 2) }
        if UIKitStyle.reduceMotion || window == nil { turn() }
        else { UIView.animate(withDuration: 0.15, delay: 0, options: .curveEaseOut, animations: turn) }
        toggle.accessibilityLabel = "Channels, \(value)"
        toggle.accessibilityValue = expanded ? "Expanded" : "Collapsed"
        add.isHidden = !owner
        add.isEnabled = canCreate
        add.accessibilityHint = canCreate ? nil : "Channel limit reached."
        options.isHidden = !owner
    }
}

/// The rule between Channels and Direct messages: drag to resize, double-tap
/// to even them out; VoiceOver adjusts it like a slider.
final class SectionDivider: UIView {
    var step: ((_ down: Bool) -> Void)?
    override init(frame: CGRect) {
        super.init(frame: frame)
        let line = UIView()
        line.backgroundColor = UIKitStyle.border
        line.translatesAutoresizingMaskIntoConstraints = false
        addSubview(line)
        NSLayoutConstraint.activate([
            line.leadingAnchor.constraint(equalTo: leadingAnchor), line.trailingAnchor.constraint(equalTo: trailingAnchor),
            line.centerYAnchor.constraint(equalTo: centerYAnchor), line.heightAnchor.constraint(equalToConstant: 1),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .adjustable
        accessibilityLabel = "Channels and direct messages divider"
        accessibilityHint = "Drag to resize the channel and direct message lists."
        accessibilityIdentifier = "sidebar-sections-resize"
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func accessibilityIncrement() { step?(true) }
    override func accessibilityDecrement() { step?(false) }
}

/// A label with horizontal padding, for counts in capsules.
final class PaddedLabel: UILabel {
    var padding: CGFloat = 12
    override var intrinsicContentSize: CGSize {
        let size = super.intrinsicContentSize
        return CGSize(width: size.width + padding, height: size.height)
    }
}

/// Browse channels' search field.
final class SearchFieldCell: UICollectionViewListCell, UITextFieldDelegate {
    private let field = UISearchTextField()
    var changed: ((String) -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        field.placeholder = "Search channels"
        // Its name once text replaces the placeholder.
        field.accessibilityLabel = "Search channels"
        field.font = UIKitStyle.font(14)
        field.textColor = UIKitStyle.text
        field.backgroundColor = UIKitStyle.raised
        field.autocapitalizationType = .none
        field.autocorrectionType = .no
        field.returnKeyType = .done
        field.delegate = self
        field.accessibilityIdentifier = "browse-channels-search"
        field.translatesAutoresizingMaskIntoConstraints = false
        field.addAction(UIAction { [weak self] _ in self?.changed?(self?.field.text ?? "") }, for: .editingChanged)
        contentView.addSubview(field)
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            field.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),
            field.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 6),
            field.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -6),
            field.heightAnchor.constraint(equalToConstant: 40),
        ])
        backgroundConfiguration = .clear()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool { textField.resignFirstResponder(); return true }
}

/// A Browse channels result: icon and name, unread dot, Joined or
/// Read-only preview, and Manage for owners.
final class BrowseResultCell: UICollectionViewListCell {
    private let icon = UIImageView()
    private let name = UIKitStyle.label(size: 15, weight: .medium, color: UIKitStyle.muted)
    private let dot = UIView()
    private let state = UIKitStyle.label(size: 10, color: UIKitStyle.muted)
    private lazy var manage = IconButton(icon: "settings", label: "Manage", size: 16, side: 44) { [weak self] in self?.manageAction?() }
    private var manageAction: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        icon.tintColor = UIKitStyle.muted
        icon.contentMode = .center
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        dot.backgroundColor = UIKitStyle.text
        dot.layer.cornerRadius = 3
        manage.tintColor = UIKitStyle.muted
        let spacer = UIView()
        let row = UIStackView(arrangedSubviews: [icon, name, spacer, dot, state, manage])
        row.spacing = 8
        row.alignment = .center
        row.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(row)
        NSLayoutConstraint.activate([
            icon.widthAnchor.constraint(equalToConstant: 20),
            dot.widthAnchor.constraint(equalToConstant: 6), dot.heightAnchor.constraint(equalToConstant: 6),
            row.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 25),
            row.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -12),
            row.topAnchor.constraint(equalTo: contentView.topAnchor),
            row.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            contentView.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
        backgroundConfiguration = .clear()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func show(_ channel: Channel, unread: Bool, owner: Bool, manage action: @escaping () -> Void) {
        icon.image = UIKitStyle.icon(channel.private ? "lock" : "hash")?.resized(to: 16)
        name.text = channel.name
        dot.isHidden = !unread
        state.text = channel.joined ? "Joined" : "Read-only preview"
        manageAction = action
        manage.isHidden = !owner
        manage.accessibilityLabel = "Manage \(channel.name)"
        // VoiceOver: the row opens the channel; Manage is its own button.
        isAccessibilityElement = false
        contentView.accessibilityElements = owner ? [name, manage] : [name]
        name.accessibilityLabel = "\(channel.name), \(state.text ?? "")"
        name.accessibilityValue = unread ? "Unread activity" : ""
        name.accessibilityTraits = .button
        accessibilityIdentifier = "browse-channel-\(channel.name)"
    }
}
