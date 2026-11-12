import AppKit
import CaperCore

/// Native navigation chrome; only a requested management dialog is hosted.
@MainActor final class NavigationSidebar: NativeView, NSSearchFieldDelegate {
    static let railWidth: CGFloat = 60
    private let model: AppModel
    var show: ((WorkspaceSheet) -> Void)?
    var showActivity: (() -> Void)?
    var showSettings: (() -> Void)?
    var showWhatsNew: (() -> Void)?
    var closeNavigation: (() -> Void)?
    var narrow = false { didSet { if oldValue != narrow { needsLayout = true } } }
    var leaveChannel: ((Channel) -> Void)?
    var block: ((BlockTarget) -> Void)?
    private let observation = NativeObservation()
    private let rail = NativeView(NativeStyle.blackout)
    private let railScroll = NSScrollView()
    private var railButtons: [String: NativeButton] = [:]
    private var railOrder: [NativeButton] = []
    private lazy var createSpace = NativeButton.icon("Create space", symbol: "plus") { [weak self] in self?.show?(.createSpace) }
    private let sidebar = NativeView(NativeStyle.sidebar)
    private let channels = NativeSidebarList("Channels", identifier: "sidebar-channels-scroll")
    private let directs = NativeSidebarList("Direct messages", identifier: "sidebar-direct-scroll")
    private var preferredChannelsShare = UserDefaults.standard.object(forKey: "caper.channelSidebarChannelsShare") as? Double ?? 0.5
    private var draggedChannelsShare: Double?
    private lazy var sectionsDivider = NativeSidebarDivider { [weak self] share, commit in
        guard let self else { return }
        if commit {
            self.preferredChannelsShare = share; self.draggedChannelsShare = nil
            UserDefaults.standard.set(share, forKey: "caper.channelSidebarChannelsShare")
        } else { self.draggedChannelsShare = share }
        self.needsLayout = true; self.layoutSubtreeIfNeeded()
    }
    private let search = NSSearchField()
    private let error = NativeStyle.label("", size: 11)
    private var expanded = true
    private var browsing = false
    private var spaceID: String?
    private var requestButtons: [String: NativeButton] = [:]
    private var directRows: [String: NativeDirectRow] = [:]
    private var channelCards: [String: NativeVoiceCard] = [:]
    private var externalCards: [String: NativeVoiceCard] = [:]
    private lazy var voicePanels = NativeVoicePanels(model: model)
    private lazy var activity = NativeActivityEntry { [weak self] in self?.showActivity?() }
    private lazy var selfNotes = NativeDirectRow { [weak self] in
        guard let self else { return }; Task { await self.model.openSelfDirectMessage() }
    }
    private lazy var newMessage = NativeButton("New message") { [weak self] in self?.show?(.newDirectMessage) }
    private lazy var directAdd = NativeButton.icon("New direct message", symbol: "plus") { [weak self] in self?.show?(.newDirectMessage) }
    private lazy var directHeading = NativeSidebarHeading(title: NativeStyle.label("Direct messages", size: 12, bold: true), actions: [directAdd], hoverActions: true)
    private var refreshingDirects = false
    private lazy var directFailure = NativeDirectFailure { [weak self] in
        guard let self, !self.refreshingDirects else { return }
        self.refreshingDirects = true; self.render()
        Task {
            await self.model.refreshDirectMessages()
            self.refreshingDirects = false; self.render()
        }
    }
    private lazy var requests = NativeButton("") { [weak self] in
        guard let self else { return }; self.model.showingMessageRequests.toggle()
    }
    private lazy var space = NativeButton("") { [weak self] in self?.openSpaceMenu() }
    private lazy var close = NativeButton.icon("Close navigation", symbol: "xmark") { [weak self] in self?.closeNavigation?() }
    private lazy var heading = NativeButton("") { [weak self] in
        guard let self else { return }; self.expanded.toggle(); self.render()
    }
    private lazy var create = NativeButton.icon("Create channel", symbol: "plus") { [weak self] in self?.show?(.createChannel) }
    private lazy var options: NativeButton = NativeButton.icon("Channel options", symbol: "ellipsis") { [weak self] in
        guard let self else { return }
        self.popup([NavigationMenuItem("Create channel", enabled: self.model.canCreateChannel) { [weak self] in self?.show?(.createChannel) },
            NavigationMenuItem(self.expanded ? "Collapse channels" : "Expand channels") { [weak self] in
                guard let self else { return }; self.expanded.toggle(); self.render()
            }], at: self.options)
    }
    private lazy var channelHeading = NativeSidebarHeading(title: heading, actions: [create, options], disclosure: true)
    private let accountAvatar = NativeAvatar()
    private let accountPresence = CALayer()
    private let accountName = NativeStyle.label("", size: 13)
    private lazy var account = NativeButton("") { [weak self] in self?.show?(.profile) }
    private lazy var mute = NativeButton.icon("Mute microphone", symbol: "mic") { [weak self] in
        guard let self else { return }; VoicePresentation.toggleMute(self.model.voice)
    }
    private lazy var deafen = NativeButton.icon("Deafen audio", symbol: "headphones") { [weak self] in
        guard let self else { return }; VoicePresentation.toggleDeafen(self.model.voice)
    }
    private lazy var inputOptions: NativeButton = NativeButton.icon("Microphone options", symbol: "chevron.down") { [weak self] in
        guard let self else { return }; self.voicePanels.audio(input: true, from: self.inputOptions)
    }
    private lazy var outputOptions: NativeButton = NativeButton.icon("Audio output options", symbol: "chevron.down") { [weak self] in
        guard let self else { return }; self.voicePanels.audio(input: false, from: self.outputOptions)
    }
    private lazy var settings = NativeButton.icon("Account settings", symbol: "gearshape") { [weak self] in self?.openAccountMenu() }
    /// The shipping app's updater, as SwiftUI's account bar shows it: "Update
    /// available" above the account row and "Check for updates" in its menu.
    /// The development host leaves it nil.
    var updates: DesktopUpdateControls? {
        didSet {
            let hidden = updates?.review == nil
            if updateButton.isHidden != hidden { updateButton.isHidden = hidden; needsLayout = true }
        }
    }
    private lazy var updateButton = NativeButton("Update available") { [weak self] in self?.updates?.review?() }
    private lazy var updateShine = AppKitUpdateShine(button: updateButton)

    init(model: AppModel) {
        self.model = model
        super.init()
        [railScroll, sidebar].forEach { addSubview($0) }
        railScroll.documentView = rail; railScroll.drawsBackground = false
        [space, close, channels, sectionsDivider, directs, updateButton, accountAvatar, accountName, account, mute, inputOptions, deafen, outputOptions, settings].forEach { sidebar.addSubview($0) }
        updateButton.isHidden = true
        updateButton.setAccessibilityIdentifier("desktop-update-action")
        updateButton.toolTip = "Review the update before restarting Caper."
        updateButton.focusChanged = { [weak self] focused in self?.updateShine.focusChanged(focused) }
        sidebar.layer?.addSublayer(accountPresence)
        accountPresence.cornerRadius = 5; accountPresence.borderWidth = 2
        accountPresence.borderColor = NativeStyle.sidebar.cgColor
        accountName.setAccessibilityElement(false)
        newMessage.image = NSImage(systemSymbolName: "plus", accessibilityDescription: nil)
        newMessage.imagePosition = .imageLeft; newMessage.alignment = .left
        newMessage.font = NativeStyle.font(13); newMessage.profileFeedback = true
        newMessage.layer?.backgroundColor = NSColor.clear.cgColor
        newMessage.setAccessibilityIdentifier("new-message")
        directAdd.setAccessibilityIdentifier("new-direct-message")
        selfNotes.open.toolTip = "Your private notes"
        search.placeholderString = "Search channels"
        search.font = NativeStyle.font(13)
        search.delegate = self
        search.setAccessibilityIdentifier("browse-channels-search")
        search.setAccessibilityLabel("Search channels")
        space.setAccessibilityIdentifier("selected-space-name")
        close.setAccessibilityIdentifier("close-navigation")
        space.image = NSImage(systemSymbolName: "chevron.down", accessibilityDescription: nil)
        space.imagePosition = .imageRight
        space.alignment = .left; space.font = NativeStyle.font(15, bold: true)
        heading.alignment = .left; heading.font = NativeStyle.font(12, bold: true)
        heading.setAccessibilityIdentifier("channels-toggle")
        [space, heading, account].forEach { $0.layer?.backgroundColor = NSColor.clear.cgColor }
        create.setAccessibilityIdentifier("create-channel")
        options.setAccessibilityIdentifier("channel-options")
        account.setAccessibilityIdentifier("account-profile")
        account.alignment = .left
        createSpace.setAccessibilityIdentifier("create-space")
        mute.setAccessibilityIdentifier("microphone-toggle")
        deafen.setAccessibilityIdentifier("deafen-toggle")
        inputOptions.setAccessibilityIdentifier("microphone-options")
        outputOptions.setAccessibilityIdentifier("output-options")
        settings.setAccessibilityIdentifier("account-settings-menu")
        error.textColor = NSColor(CaperTheme.error)
        observation.start { [weak self] in self?.render() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { voicePanels.close() }
    }

    override func layout() {
        super.layout()
        railScroll.frame = NSRect(x: 0, y: 0, width: Self.railWidth, height: bounds.height)
        rail.frame.size = NSSize(width: Self.railWidth, height: max(bounds.height, CGFloat(railOrder.count) * 54 + 20))
        for (index, child) in railOrder.enumerated() { child.frame = NSRect(x: (Self.railWidth - 40) / 2, y: 14 + CGFloat(index) * 54, width: 40, height: 40) }
        sidebar.frame = NSRect(x: Self.railWidth, y: 0, width: bounds.width - Self.railWidth, height: bounds.height)
        let width = sidebar.bounds.width
        close.isHidden = !narrow
        close.frame = NSRect(x: width - 44, y: 8, width: 28, height: 34)
        space.frame = NSRect(x: 16, y: 8, width: width - (narrow ? 64 : 32), height: 34)
        // The update button sits between the lists and the account row.
        let reserved: CGFloat = updateButton.isHidden ? 0 : 52
        let footer = max(50, bounds.height - 62 - reserved)
        let available = max(0, footer - 50 - 12)
        sectionsDivider.update(available: available, share: draggedChannelsShare ?? preferredChannelsShare)
        // Whole points: a fractional split makes NSScrollView snap its clip view
        // to the pixel grid and shift the lists' scroll origins to compensate.
        let channelHeight = (available * CGFloat(sectionsDivider.share)).rounded()
        channels.frame = NSRect(x: 0, y: 50, width: width, height: channelHeight)
        sectionsDivider.frame = NSRect(x: 0, y: channels.frame.maxY, width: width, height: 12)
        directs.frame = NSRect(x: 0, y: sectionsDivider.frame.maxY, width: width, height: max(0, available - channelHeight))
        channels.layoutRows(); directs.layoutRows()
        updateButton.frame = NSRect(x: 8, y: footer + 8, width: width - 16, height: 36)
        updateShine.layout()
        let row = footer + reserved
        account.frame = NSRect(x: 16, y: row + 8, width: width - 156, height: 36)
        accountAvatar.frame = NSRect(x: 16, y: row + 11, width: 30, height: 30)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        accountPresence.frame = NSRect(x: accountAvatar.frame.maxX - 8, y: accountAvatar.frame.maxY - 8, width: 10, height: 10)
        CATransaction.commit()
        accountName.frame = NSRect(x: 53, y: row + 17, width: max(0, width - 197), height: 20)
        accountName.isHidden = account.frame.width < 76
        mute.frame = NSRect(x: width - 138, y: row + 10, width: 28, height: 32)
        inputOptions.frame = NSRect(x: width - 110, y: row + 10, width: 16, height: 32)
        deafen.frame = NSRect(x: width - 92, y: row + 10, width: 28, height: 32)
        outputOptions.frame = NSRect(x: width - 64, y: row + 10, width: 16, height: 32)
        settings.frame = NSRect(x: width - 46, y: row + 10, width: 28, height: 32)
    }

    private func render() {
        if spaceID != model.selectedSpaceID { spaceID = model.selectedSpaceID; browsing = false; search.stringValue = "" }
        // Retain controls across model refreshes: detaching a pressed/focused
        // button cancels AppKit mouse tracking and keyboard focus.
        var next: [String: NativeButton] = [:]
        railOrder = []
        for item in model.spaces {
            let key = "space-\(item.id)"
            let button = railButtons[key] ?? NativeButton("") { [weak self, id = item.id] in
                guard let self, let space = self.model.spaces.first(where: { $0.id == id }) else { return }
                Task { await self.model.select(space: space) }
            }
            button.title = String(item.name.prefix(2)).uppercased()
            button.setAccessibilityLabel(item.name); button.toolTip = item.name
            button.layer?.borderWidth = item.id == model.selectedSpaceID ? 2 : 0
            button.layer?.borderColor = NativeStyle.accent.cgColor
            next[key] = button; railOrder.append(button)
        }
        for invitation in model.invitations {
            let key = "invitation-\(invitation.id)"
            let button = railButtons[key] ?? NativeButton("") { [weak self, id = invitation.id] in
                guard let self, let invitation = self.model.invitations.first(where: { $0.id == id }) else { return }
                self.show?(.invitation(invitation))
            }
            button.title = String(invitation.name.prefix(2)).uppercased()
            button.setAccessibilityLabel("Invitation to \(invitation.name)")
            button.toolTip = "Invitation to \(invitation.name)"
            next[key] = button; railOrder.append(button)
        }
        railButtons = next
        railOrder.append(createSpace) // Also joins by invite link: enabled even at the space limit.
        for child in rail.subviews where !railOrder.contains(where: { $0 === child }) { child.removeFromSuperview() }
        for child in railOrder where child.superview == nil { rail.addSubview(child) }
        space.title = model.detail?.space.name ?? "Browse"
        space.setAccessibilityLabel(space.title)
        space.isEnabled = model.detail != nil && model.detail?.space.demo != true
        let joined = model.detail?.channels.filter(\.joined) ?? []
        heading.title = "Channels  \(joined.count)"
        heading.setAccessibilityLabel("Channels, \(joined.count)")
        heading.setAccessibilityValue(expanded ? "Expanded" : "Collapsed")
        channelHeading.setExpanded(expanded)
        if create.isHidden == model.isOwner {
            // The heading lays out only visible actions; ownership often arrives after the first render.
            create.isHidden = !model.isOwner; options.isHidden = !model.isOwner
            channelHeading.needsLayout = true
        }
        create.isEnabled = model.canCreateChannel
        create.toolTip = model.canCreateChannel ? "Create channel" : "Channel limit reached (\(model.limits?.channelsPerSpace ?? 100))"
        channels.beginUpdates(); directs.beginUpdates()
        updateExternalVoice(joined: joined)
        channels.append(channelHeading, height: 44)
        channelCards = channelCards.filter { id, _ in joined.contains { $0.id == id } }
        // Read every row's liveness even while expanded: collapsing renders
        // outside observation, so the tracked render must already depend on it.
        let live = Set(joined.filter { VoicePresentation.isLive(model, $0) }.map(\.id))
        LiveNow.listed(joined, expanded: expanded) { live.contains($0.id) }.forEach { addChannel($0) }
        if browsing {
            channels.append(search, height: 30)
            let query = String(search.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).drop(while: { $0 == "#" }))
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let matches = (model.detail?.channels ?? []).filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) }
            if matches.isEmpty { channels.append(NativeStyle.label(query.isEmpty ? "This space has no channels yet." : "No channels match your search.", size: 12)) }
            matches.forEach { addChannel($0, browsing: true) }
        }
        if model.detail?.channelInvitations.isEmpty == false {
            channels.append(NativeStyle.label("Private invitations", size: 12, bold: true), height: 28)
        }
        for invitation in model.detail?.channelInvitations ?? [] {
            channels.append(NativeButton("#\(invitation.channel.name) · from @\(invitation.inviter.username)") { [weak self] in
                self?.show?(.channelInvitation(invitation))
            })
        }
        directs.append(directHeading, height: 28)
        activity.update(unread: AppKitWorkspacePresenter.activityUnread(model))
        directs.append(activity, height: 28)
        var nextRequests: [String: NativeButton] = [:]
        if !model.messageRequests.isEmpty {
            requests.title = "\(model.showingMessageRequests ? "⌄" : "›")  Message requests  \(model.messageRequests.count)"
            requests.alignment = .left
            requests.setAccessibilityIdentifier("message-requests")
            requests.setAccessibilityLabel("Message requests")
            requests.setAccessibilityValue("\(model.messageRequests.count) \(model.messageRequests.count == 1 ? "request" : "requests"), \(model.showingMessageRequests ? "Expanded" : "Collapsed")")
            directs.append(requests)
            if model.showingMessageRequests {
                for request in model.messageRequests {
                    let button = requestButtons[request.id] ?? NativeButton("") { [weak self, id = request.id] in
                        guard let self, let request = self.model.messageRequests.first(where: { $0.id == id }) else { return }
                        Task { await self.model.select(directMessage: request) }
                    }
                    button.title = "\(request.peer.displayName) · @\(request.peer.username)"
                    button.setAccessibilityLabel(button.title)
                    button.alignment = .left
                    button.setAccessibilityIdentifier("message-request-\(request.id)")
                    button.setAccessibilityValue(model.selectedDirectMessageID == request.id ? "Selected" : "")
                    button.layer?.backgroundColor = model.selectedDirectMessageID == request.id ? NativeStyle.accent.withAlphaComponent(0.16).cgColor : NSColor.clear.cgColor
                    nextRequests[request.id] = button
                    directs.append(button)
                }
            }
        }
        if let account = model.account {
            let conversation = model.directMessages.first { $0.peer.id == account.id }
            selfNotes.update(name: account.displayName ?? account.username ?? "You", avatarID: account.avatarId, picture: account.avatarPicture,
                selected: conversation.map { model.selectedDirectMessageID == $0.id } == true, unread: conversation?.unread == true,
                count: conversation?.unreadCount, muted: false, isSelf: true, busy: model.busy)
            selfNotes.open.setAccessibilityIdentifier("dm-self")
            directs.append(selfNotes, height: 28)
        }
        var nextDirects: [String: NativeDirectRow] = [:]
        for conversation in model.visibleDirectMessages where conversation.peer.id != model.account?.id {
            let row = directRows[conversation.id] ?? NativeDirectRow { [weak self, id = conversation.id] in
                guard let self, let current = self.model.directMessages.first(where: { $0.id == id }) else { return }
                Task { await self.model.select(directMessage: current) }
            }
            let muted = model.notificationsMuted(.direct(conversation.id))
            row.update(name: conversation.peer.displayName, avatarID: conversation.peer.avatarId, picture: conversation.peer.avatarPicture,
                selected: model.selectedDirectMessageID == conversation.id,
                unread: conversation.unread && !muted && !model.chat.blockedAuthorIDs.contains(conversation.peer.id),
                count: conversation.unreadCount, muted: muted, isSelf: false, busy: false)
            row.open.setAccessibilityIdentifier("dm-\(conversation.id)")
            row.options.setAccessibilityIdentifier("dm-options-\(conversation.id)")
            row.options.setAccessibilityLabel("Options for \(conversation.peer.displayName)")
            row.openMenu = { [weak self, weak row] in
                guard let self, let row else { return }
                self.model.refreshNotificationSettingsIfStale()
                var items = NavigationMenus.direct(self.model, id: conversation.id)
                if conversation.blocked {
                    items += [.separator(), NavigationMenuItem("Unblock @\(conversation.peer.username)") { [weak self] in
                        guard let self else { return }
                        Task {
                            do { try await self.model.unblock(accountID: conversation.peer.id) }
                            catch { self.model.chat.error = NativePresentation.errorMessage(error) }
                        }
                    }]
                } else if conversation.peer.canBlock {
                    items += [.separator(), NavigationMenuItem("Block @\(conversation.peer.username)…") { [weak self] in
                        self?.block?(BlockTarget(peer: conversation.peer))
                    }]
                }
                self.popup(items, at: row.options)
            }
            nextDirects[conversation.id] = row
            directs.append(row, height: 28)
        }
        directRows = nextDirects
        directs.append(newMessage, height: 28)
        if let failure = model.directMessagesError {
            directFailure.update(failure, pending: refreshingDirects)
            directs.append(directFailure)
        }
        requestButtons = nextRequests
        accountName.stringValue = model.account?.displayName ?? ""
        account.setAccessibilityLabel("Edit profile for \(model.account?.displayName ?? "your account")")
        if let current = model.account { accountAvatar.update(current) }
        account.setAccessibilityValue(model.ownPresence == nil || model.ownPresence == .unknown
            ? "Status unavailable" : model.ownPresence!.rawValue.capitalized)
        accountPresence.backgroundColor = (model.ownPresence == .online ? NSColor(CaperTheme.green) : model.ownPresence == .idle
            ? NSColor(srgbRed: 0.72, green: 0.60, blue: 0.35, alpha: 1) : NativeStyle.border).cgColor
        mute.image = NSImage(systemSymbolName: model.voice.muted ? "mic.slash" : "mic", accessibilityDescription: nil)
        mute.setAccessibilityLabel(model.voice.muted ? "Unmute microphone" : "Mute microphone")
        mute.setAccessibilityValue(model.voice.muted ? "Muted" : "On")
        mute.contentTintColor = model.voice.muted ? NSColor(CaperTheme.voiceOff) : NativeStyle.muted
        inputOptions.contentTintColor = mute.contentTintColor
        deafen.image = NSImage(systemSymbolName: model.voice.deafened ? "speaker.slash" : "headphones", accessibilityDescription: nil)
        deafen.setAccessibilityLabel(model.voice.deafened ? "Undeafen audio" : "Deafen audio")
        deafen.setAccessibilityValue(model.voice.deafened ? "Deafened" : "On")
        deafen.contentTintColor = model.voice.deafened ? NSColor(CaperTheme.voiceOff) : NativeStyle.muted
        outputOptions.contentTintColor = deafen.contentTintColor
        error.stringValue = model.voice.error ?? model.notificationError ?? model.error ?? ""
        if !error.stringValue.isEmpty { channels.append(error, height: 30) }
        channels.endUpdates(); directs.endUpdates()
        needsLayout = true
    }

    private func addChannel(_ channel: Channel, browsing: Bool = false) {
        let row = (!browsing ? channelCards[channel.id]?.heading as? NavigationRow : nil) ?? NavigationRow(channel.name, symbol: channel.private ? "lock" : "number", selected: model.selectedChannelID == channel.id,
            unread: model.channelUnread(channel.id), muted: model.notificationsMuted(.channel(spaceID: channel.spaceId, channelID: channel.id)),
            open: { [weak self, id = channel.id] in
                guard let self, let channel = self.model.detail?.channels.first(where: { $0.id == id }) else { return }
                Task { await self.model.select(channel: channel) }
            })
        row.update(channel.name, symbol: channel.private ? "lock" : "number", selected: model.selectedChannelID == channel.id,
            unread: model.channelUnread(channel.id), muted: model.notificationsMuted(.channel(spaceID: channel.spaceId, channelID: channel.id)))
        row.open.setAccessibilityIdentifier("\(browsing ? "browse-channel" : "channel")-\(channel.id)")
        let accessibleName = channel.name + (channel.private ? ", private" : "")
        row.open.setAccessibilityLabel(accessibleName)
        if browsing {
            row.detail.stringValue = channel.joined ? "Joined" : "Read-only preview"
            if channel.private { row.open.setAccessibilityLabel("\(accessibleName), \(row.detail.stringValue)") }
            row.open.setAccessibilityHelp(row.detail.stringValue)
        }
        row.options.setAccessibilityIdentifier("\(browsing ? "browse-options" : "channel-options")-\(channel.id)")
        row.options.setAccessibilityLabel("Channel options for \(channel.name)")
        row.openMenu = { [weak self, weak row] in
            guard let self, let row else { return }
            self.model.refreshNotificationSettingsIfStale()
            var items: [NSMenuItem] = []
            if self.model.isOwner {
                if !channel.private {
                    items.append(NavigationMenuItem("Invite people…") { [weak self] in self?.show?(.invitePeople(spaceID: channel.spaceId, channelID: channel.id)) })
                }
                items.append(NavigationMenuItem("Channel settings") { [weak self] in self?.show?(.manageChannel(channel)) })
                items.append(.separator())
            }
            items += NavigationMenus.channel(self.model, channel: channel)
            if channel.joined {
                items.append(.separator())
                items.append(NavigationMenuItem("Leave channel") { [weak self] in self?.leaveChannel?(channel) })
            }
            self.popup(items, at: row.options)
        }
        if browsing { channels.append(row) }
        else {
            let card = channelCards[channel.id] ?? NativeVoiceCard(model: model, channelID: channel.id, heading: row, panels: voicePanels) { [weak self, id = channel.id] in
                guard let self, let channel = self.model.detail?.channels.first(where: { $0.id == id }) else { return }
                Task { await self.model.select(channel: channel) }
            }
            card.changed = { [weak self] in self?.needsLayout = true }
            channelCards[channel.id] = card
            channels.append(card)
        }
    }

    private func updateExternalVoice(joined: [Channel]) {
        var next: [String: NativeVoiceCard] = [:]
        func add(id: String, title: String, open: @escaping () -> Void) {
            let button = externalCards[id]?.heading as? NativeButton ?? NativeButton(title, action: open)
            button.title = title; button.setAccessibilityLabel(title)
            button.setAccessibilityIdentifier("voice-call-channel")
            button.alignment = .left; button.font = NativeStyle.font(13, bold: true)
            button.layer?.backgroundColor = NSColor.clear.cgColor
            let card = externalCards[id] ?? NativeVoiceCard(model: model, channelID: id, heading: button, panels: voicePanels, openChat: open)
            card.changed = { [weak self] in self?.needsLayout = true }
            next[id] = card; channels.append(card)
        }
        if VoicePresentation.active(model.voice), let context = model.voice.context,
           !LiveNow.hasRow(context.channelID, rows: joined.map(\.id)) {
            add(id: context.channelID, title: "#\(context.channelName) · \(context.spaceName)") { [weak self] in
                guard let self else { return }; Task { await self.model.openVoiceContext() }
            }
        }
        if let channel = model.visitChannel, VoicePresentation.isVisit(model, channel), !model.voice.isActive(channelID: channel.id),
           !VoicePresentation.occupants(model, channelID: channel.id).isEmpty {
            add(id: channel.id, title: "#\(channel.name) · \(model.visit?.spaceName ?? "")") { [weak self] in
                // The visited chat is already open; this matches its shipping card.
                self?.model.navigationOpen = false
            }
        }
        for card in externalCards.values where !next.values.contains(where: { $0 === card }) { card.removeFromSuperview() }
        externalCards = next
    }

    private func openSpaceMenu() {
        model.refreshNotificationSettingsIfStale()
        var items: [NSMenuItem] = [NavigationMenuItem(browsing ? "Exit Browse channels" : "Browse channels") { [weak self] in
            guard let self else { return }; self.browsing.toggle(); self.search.stringValue = ""; self.render()
            if self.browsing { self.window?.makeFirstResponder(self.search) }
        }]
        if let space = model.detail?.space, space.demo != true {
            items += [.separator()] + NavigationMenus.space(model, id: space.id) + [.separator()]
            if model.isOwner {
                items.append(NavigationMenuItem("Invite people…") { [weak self] in self?.show?(.invitePeople(spaceID: space.id, channelID: nil)) })
                items.append(NavigationMenuItem("Space settings") { [weak self] in self?.show?(.manageSpace) })
            } else { items.append(NavigationMenuItem("Leave space…") { [weak self] in self?.show?(.leaveSpace) }) }
        }
        popup(items, at: space)
    }

    private func openAccountMenu() {
        var items: [NSMenuItem] = [NavigationMenuItem("Audio test", enabled: model.voice.phase != .leaving) { [weak self] in self?.show?(.audio) }]
        if model.voice.phase == .connected { items.append(NavigationMenuItem("Connection details") { [weak self] in self?.show?(.connection) }) }
        if model.account?.debugEnabled == true { items.append(NavigationMenuItem("Audio diagnostics") { [weak self] in self?.show?(.diagnostics) }) }
        items += [.separator(), NavigationMenuItem("Notifications") { [weak self] in self?.show?(.notifications) },
            NavigationMenuItem("Privacy and blocked accounts") { [weak self] in self?.show?(.privacy) }, .separator(),
            NavigationMenuItem("Settings…") { [weak self] in self?.showSettings?() }]
        if let updates {
            items.append(NavigationMenuItem(updates.isChecking ? "Checking…" : "Check for updates",
                                            enabled: updates.check != nil && !updates.isChecking) { updates.check?() })
        }
        items += [NavigationMenuItem("What’s new") { [weak self] in self?.showWhatsNew?() }, .separator(),
            NavigationMenuItem("Log out") { [weak self] in guard let self else { return }; Task { await self.model.logout() } }]
        popup(items, at: settings)
    }

    private func popup(_ items: [NSMenuItem], at view: NSView) {
        NavigationMenus.menu(items).popUp(positioning: nil, at: NSPoint(x: 0, y: view.bounds.maxY), in: view)
    }

    func controlTextDidChange(_ notification: Notification) { render() }
}

/// Stable children keep keyboard focus and each list's own native scroll origin.
private final class NativeSidebarList: NSScrollView {
    private let rows = NativeView()
    private var items: [(NSView, CGFloat)] = []
    init(_ label: String, identifier: String) {
        super.init(frame: .zero)
        documentView = rows
        drawsBackground = false; hasVerticalScroller = true; autohidesScrollers = true
        setAccessibilityLabel(label); setAccessibilityIdentifier(identifier)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func beginUpdates() { items = [] }
    func append(_ view: NSView, height: CGFloat = 32) {
        items.append((view, height))
        if view.superview !== rows { rows.addSubview(view) }
    }
    func endUpdates() {
        for child in rows.subviews where !items.contains(where: { $0.0 === child }) { child.removeFromSuperview() }
        rows.setAccessibilityChildren(items.map { $0.0 })
        needsLayout = true
    }
    override func layout() { super.layout(); layoutRows() }
    func layoutRows() {
        let origin = contentView.bounds.origin
        let width = max(0, contentSize.width - 32)
        var y: CGFloat = 6
        for (child, fallback) in items {
            let height = (child as? NativeVoiceCard)?.height(width: width)
                ?? (child as? NativeDirectFailure)?.height(width: width) ?? fallback
            child.frame = NSRect(x: 16, y: y, width: width, height: height)
            y += height + 3
        }
        let size = NSSize(width: contentSize.width, height: max(contentSize.height, y + 8))
        if rows.frame.size != size { rows.setFrameSize(size) }
        contentView.scroll(to: NSPoint(x: 0, y: min(max(0, origin.y), max(0, size.height - contentSize.height))))
        reflectScrolledClipView(contentView)
    }
}

/// Recovery belongs to the DM list and only reloads it; it never creates a
/// conversation, accepts a request or retries a message send.
private final class NativeDirectFailure: NativeView {
    private let message = NativeStyle.label("", size: 11)
    private let retry: NativeButton
    init(action: @escaping () -> Void) {
        retry = NativeButton("Retry direct messages", action: action)
        super.init()
        message.lineBreakMode = .byWordWrapping; message.maximumNumberOfLines = 0
        message.textColor = NSColor(CaperTheme.error)
        message.setAccessibilityIdentifier("direct-messages-error")
        retry.font = NativeStyle.font(11, bold: true)
        retry.setAccessibilityIdentifier("retry-direct-messages")
        addSubview(message); addSubview(retry)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ error: String, pending: Bool) {
        message.stringValue = error
        retry.isEnabled = !pending
        needsLayout = true
    }
    private func messageHeight(width: CGFloat) -> CGFloat {
        NativePresentation.textHeight(NSAttributedString(string: message.stringValue,
            attributes: [.font: NativeStyle.font(11)]), width: max(1, width - 4)) + 4
    }
    func height(width: CGFloat) -> CGFloat { messageHeight(width: width) + 36 }
    override func layout() {
        super.layout()
        let height = messageHeight(width: bounds.width)
        message.frame = NSRect(x: 0, y: 0, width: bounds.width, height: height)
        retry.frame = NSRect(x: 0, y: height + 4, width: bounds.width, height: 28)
    }
}

/// A single rotating chevron, not alternating text glyphs. Only its transform
/// animates, and Reduce Motion removes that animation.
private final class NativeSidebarHeading: NativeView {
    private let title: NSView
    private let actions: [NativeButton]
    private let hoverActions: Bool
    private let chevron: CAShapeLayer?
    private var expanded = true
    private var hovered = false
    private var focusedActions: Set<ObjectIdentifier> = []
    private var tracking: NSTrackingArea?
    init(title: NSView, actions: [NativeButton], disclosure: Bool = false, hoverActions: Bool = false) {
        self.title = title; self.actions = actions; self.hoverActions = hoverActions
        chevron = disclosure ? CAShapeLayer() : nil
        super.init()
        addSubview(title)
        for action in actions {
            addSubview(action)
            action.focusChanged = { [weak self, weak action] focused in
                guard let self, let action else { return }
                if focused { self.focusedActions.insert(ObjectIdentifier(action)) }
                else { self.focusedActions.remove(ObjectIdentifier(action)) }
                self.updateActions()
            }
        }
        if let chevron {
            chevron.fillColor = nil; chevron.strokeColor = NativeStyle.muted.cgColor
            chevron.lineWidth = 1.5; chevron.lineCap = .round; chevron.lineJoin = .round
            let path = CGMutablePath(); path.move(to: CGPoint(x: 2, y: 4)); path.addLine(to: CGPoint(x: 6, y: 8)); path.addLine(to: CGPoint(x: 10, y: 4))
            chevron.path = path; layer?.addSublayer(chevron)
        }
        updateActions()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func setExpanded(_ value: Bool) {
        guard value != expanded else { return }
        let from = expanded ? 0.0 : -Double.pi / 2
        expanded = value
        let to = value ? 0.0 : -Double.pi / 2
        CATransaction.begin(); CATransaction.setDisableActions(true)
        chevron?.setAffineTransform(CGAffineTransform(rotationAngle: to))
        CATransaction.commit()
        if window != nil && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            let animation = CABasicAnimation(keyPath: "transform.rotation.z")
            animation.fromValue = from; animation.toValue = to; animation.duration = 0.15
            animation.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
            chevron?.add(animation, forKey: "disclosure")
        }
    }
    override func layout() {
        super.layout()
        var right = bounds.width
        for action in actions.reversed() where !action.isHidden {
            right -= 28
            action.frame = NSRect(x: right, y: (bounds.height - 28) / 2, width: 28, height: 28)
            right -= 2
        }
        let left: CGFloat = chevron == nil ? 0 : 20
        title.frame = NSRect(x: left, y: title is NSButton ? 0 : (bounds.height - 18) / 2,
            width: max(0, right - left), height: title is NSButton ? bounds.height : 18)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        chevron?.bounds = NSRect(x: 0, y: 0, width: 12, height: 12)
        chevron?.position = CGPoint(x: 6, y: bounds.midY)
        CATransaction.commit()
    }
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let hit = super.hitTest(point) else { return nil }
        if hit === self, title is NSButton { return title }
        if hoverActions, actions.contains(where: { $0 === hit }), !hovered && window?.firstResponder !== hit { return self }
        return hit
    }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: .zero, options: [.inVisibleRect, .mouseEnteredAndExited, .activeInKeyWindow], owner: self, userInfo: nil)
        addTrackingArea(area); tracking = area
    }
    override func mouseEntered(with event: NSEvent) { hovered = true; updateActions() }
    override func mouseExited(with event: NSEvent) { hovered = false; updateActions() }
    private func updateActions() {
        for action in actions { action.layer?.opacity = !hoverActions || hovered || focusedActions.contains(ObjectIdentifier(action)) ? 1 : 0 }
    }
}

/// Avatars, badges and the self marker are decorative; one native button owns
/// their hit target and accessible name. Options stay a separate focus stop.
private final class NativeDirectRow: NativeView {
    let open: NativeButton
    let options = NativeButton.icon("Options", symbol: "ellipsis") {}
    var openMenu: (() -> Void)?
    private let avatar = NativeAvatar()
    private let name = NativeStyle.label("", size: 13)
    private let you = NativeStyle.label("you", size: 10, bold: true)
    private let badge = NativeStyle.label("", size: 10, bold: true)
    private let bell = NSImageView()
    private var selected = false
    private var hovered = false
    private var optionsFocused = false
    private var tracking: NSTrackingArea?

    init(action: @escaping () -> Void) {
        open = NativeButton("", action: action)
        super.init()
        layer?.cornerRadius = 6
        open.layer?.backgroundColor = NSColor.clear.cgColor
        [open, avatar, name, you, badge, bell, options].forEach { addSubview($0) }
        [name, you, badge, bell].forEach { $0.setAccessibilityElement(false) }
        you.textColor = NativeStyle.muted
        badge.alignment = .center; badge.wantsLayer = true
        badge.layer?.backgroundColor = NativeStyle.accent.cgColor; badge.layer?.cornerRadius = 8
        bell.image = NSImage(systemSymbolName: "bell.slash", accessibilityDescription: nil)
        bell.contentTintColor = NativeStyle.muted
        options.target = self; options.action = #selector(showMenu)
        options.focusChanged = { [weak self] focused in self?.optionsFocused = focused; self?.updateHover() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func showMenu() { openMenu?() }
    func update(name: String, avatarID: Int?, picture: CaperPicture?, selected: Bool, unread: Bool,
                count: Int?, muted: Bool, isSelf: Bool, busy: Bool) {
        self.selected = selected
        self.name.stringValue = name; self.name.font = NativeStyle.font(13, bold: unread)
        self.name.textColor = selected || unread ? NativeStyle.text : NativeStyle.muted
        avatar.update(name: name, avatarId: avatarID, picture: picture)
        you.isHidden = !isSelf; options.isHidden = isSelf; bell.isHidden = !muted
        badge.isHidden = !unread
        let unreadCount = count ?? 0
        badge.stringValue = unreadCount > 0 ? (unreadCount > 99 ? "99+" : String(unreadCount)) : ""
        // Disabling a focused button hands focus to the window, and enabling it
        // again does not return it: keep keyboard focus on the row while busy.
        let focused = window?.firstResponder === open
        open.isEnabled = !busy
        if focused, window?.firstResponder !== open { window?.makeFirstResponder(open) }
        open.setAccessibilityLabel(name + (isSelf ? ", you" : ""))
        let unreadLabel = unread ? (unreadCount > 0 ? "\(unreadCount) unread messages" : "Unread activity") : ""
        open.setAccessibilityValue([selected ? "Selected" : "", unreadLabel, muted ? "Muted" : ""].filter { !$0.isEmpty }.joined(separator: ", "))
        alphaValue = muted ? 0.55 : 1
        updateHover(); needsLayout = true
    }
    override func layout() {
        super.layout()
        let end = bounds.width - (options.isHidden ? 6 : 32)
        open.frame = NSRect(x: 0, y: 0, width: max(0, end), height: bounds.height)
        avatar.frame = NSRect(x: 9, y: (bounds.height - 20) / 2, width: 20, height: 20)
        let badgeWidth: CGFloat = badge.isHidden ? 0 : badge.stringValue.isEmpty ? 8 : (badge.cell?.cellSize.width ?? 0) + 12
        badge.frame = NSRect(x: end - badgeWidth, y: (bounds.height - (badge.stringValue.isEmpty ? 8 : 18)) / 2,
            width: badgeWidth, height: badge.stringValue.isEmpty ? 8 : 18)
        let youWidth: CGFloat = you.isHidden ? 0 : 23
        let bellWidth: CGFloat = bell.isHidden ? 0 : 18
        let textWidth = max(0, end - 40 - badgeWidth - youWidth - bellWidth - 4)
        name.frame = NSRect(x: 38, y: (bounds.height - 18) / 2, width: min(textWidth, name.cell?.cellSize.width ?? 0), height: 18)
        you.frame = NSRect(x: name.frame.maxX + 4, y: (bounds.height - 16) / 2, width: youWidth, height: 16)
        bell.frame = NSRect(x: name.frame.maxX + 4, y: (bounds.height - 14) / 2, width: bellWidth > 0 ? 14 : 0, height: 14)
        options.frame = NSRect(x: bounds.width - 28, y: 0, width: 28, height: bounds.height)
    }
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let hit = super.hitTest(point) else { return nil }
        return hit === options && (hovered || window?.firstResponder === options) ? options : open
    }
    override func rightMouseDown(with event: NSEvent) { if !options.isHidden { openMenu?() } }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: .zero, options: [.inVisibleRect, .mouseEnteredAndExited, .activeInKeyWindow], owner: self, userInfo: nil)
        addTrackingArea(area); tracking = area
    }
    override func mouseEntered(with event: NSEvent) { hovered = true; updateHover() }
    override func mouseExited(with event: NSEvent) { hovered = false; updateHover() }
    private func updateHover() {
        layer?.backgroundColor = (selected ? NativeStyle.accent.withAlphaComponent(0.16) : hovered ? NSColor.white.withAlphaComponent(0.06) : .clear).cgColor
        options.layer?.opacity = hovered || optionsFocused ? 1 : 0
    }
}

/// The unread count belongs to the same action, not a second focus stop.
private final class NativeActivityEntry: NativeView {
    private let open: NativeButton
    private let count = NativeStyle.label("", size: 13)

    init(action: @escaping () -> Void) {
        open = NativeButton("Notifications", action: action)
        super.init()
        open.image = NSImage(systemSymbolName: "bell", accessibilityDescription: nil)
        open.imagePosition = .imageLeft; open.alignment = .left
        open.font = NativeStyle.font(13); open.profileFeedback = true
        open.layer?.backgroundColor = NSColor.clear.cgColor
        open.setAccessibilityIdentifier("activity-inbox")
        count.textColor = NativeStyle.accent; count.alignment = .right
        count.setAccessibilityElement(false)
        addSubview(open); addSubview(count)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(unread: Int) {
        count.stringValue = unread > 0 ? String(unread) : ""
        open.setAccessibilityValue("\(unread) unread")
        needsLayout = true
    }
    override func layout() {
        super.layout()
        open.frame = bounds
        let width = count.cell?.cellSize.width ?? 0
        count.frame = NSRect(x: bounds.width - width - 6, y: (bounds.height - 20) / 2, width: width, height: 20)
    }
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard super.hitTest(point) != nil else { return nil }
        return open
    }
}

/// A channel or DM keeps its own options separate from opening the conversation.
private final class NavigationRow: NativeView {
    let open: NativeButton
    let options = NativeButton.icon("Options", symbol: "ellipsis") {}
    let detail = NativeStyle.label("", size: 10)
    private let bell = NSImageView()
    var openMenu: (() -> Void)?

    init(_ name: String, symbol: String, selected: Bool, unread: Bool, muted: Bool, open action: @escaping () -> Void) {
        open = NativeButton(name + (unread ? "  •" : ""), action: action)
        super.init(selected ? NativeStyle.accent.withAlphaComponent(0.16) : .clear)
        layer?.cornerRadius = 6
        open.imagePosition = .imageLeft
        open.alignment = .left
        open.layer?.backgroundColor = NSColor.clear.cgColor
        bell.image = NSImage(systemSymbolName: "bell.slash", accessibilityDescription: nil)
        bell.contentTintColor = NativeStyle.muted
        bell.setAccessibilityElement(false)
        detail.textColor = NativeStyle.muted
        detail.setAccessibilityElement(false)
        options.target = self; options.action = #selector(showMenu)
        addSubview(open); addSubview(bell); addSubview(detail); addSubview(options)
        update(name, symbol: symbol, selected: selected, unread: unread, muted: muted)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func showMenu() { openMenu?() }
    func update(_ name: String, symbol: String, selected: Bool, unread: Bool, muted: Bool) {
        layer?.backgroundColor = (selected ? NativeStyle.accent.withAlphaComponent(0.16) : .clear).cgColor
        open.title = name + (unread ? "  •" : "")
        open.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        open.font = NativeStyle.font(13, bold: unread)
        open.contentTintColor = selected || unread ? NativeStyle.text : NativeStyle.muted
        open.setAccessibilityLabel(name)
        open.setAccessibilityValue([selected ? "Selected" : "", unread ? "Unread activity" : "", muted ? "Muted" : ""].filter { !$0.isEmpty }.joined(separator: ", "))
        bell.isHidden = !muted
        needsLayout = true
    }
    override func layout() {
        super.layout()
        let detailWidth = detail.stringValue.isEmpty ? 0 : detail.cell?.cellSize.width ?? 0
        let trailing = bounds.width - 36 - detailWidth
        open.frame = NSRect(x: 6, y: 0, width: trailing - (bell.isHidden ? 0 : 20), height: bounds.height)
        bell.frame = NSRect(x: trailing - 14, y: (bounds.height - 14) / 2, width: 14, height: 14)
        detail.frame = NSRect(x: trailing, y: (bounds.height - 16) / 2, width: detailWidth, height: 16)
        options.frame = NSRect(x: bounds.width - 30, y: 0, width: 28, height: bounds.height)
    }
}
