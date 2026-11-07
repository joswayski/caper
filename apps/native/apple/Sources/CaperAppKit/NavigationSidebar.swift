import AppKit
import CaperCore

/// Native navigation chrome; only a requested management dialog is hosted.
@MainActor final class NavigationSidebar: NativeView, NSSearchFieldDelegate {
    static let railWidth: CGFloat = 60
    private let model: AppModel
    var show: ((WorkspaceSheet) -> Void)?
    var leaveChannel: ((Channel) -> Void)?
    var block: ((BlockTarget) -> Void)?
    private let observation = NativeObservation()
    private let rail = NativeView(NativeStyle.blackout)
    private let railScroll = NSScrollView()
    private var railButtons: [String: NativeButton] = [:]
    private var railOrder: [NativeButton] = []
    private lazy var createSpace = NativeButton.icon("Create space", symbol: "plus") { [weak self] in self?.show?(.createSpace) }
    private let sidebar = NativeView(NativeStyle.sidebar)
    private let rows = NativeView()
    private let scroll = NSScrollView()
    private let search = NSSearchField()
    private let error = NativeStyle.label("", size: 11)
    private var expanded = true
    private var browsing = false
    private var spaceID: String?
    private var rowHeights: [CGFloat] = []
    private var rowOrder: [NSView] = []
    private var requestButtons: [String: NativeButton] = [:]
    private var channelCards: [String: NativeVoiceCard] = [:]
    private var externalCards: [String: NativeVoiceCard] = [:]
    private var externalOrder: [NativeVoiceCard] = []
    private lazy var voicePanels = NativeVoicePanels(model: model)
    private lazy var requests = NativeButton("") { [weak self] in
        guard let self else { return }; self.model.showingMessageRequests.toggle()
    }
    private lazy var space = NativeButton("") { [weak self] in self?.openSpaceMenu() }
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
    private let accountAvatar = NativeAvatar()
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

    init(model: AppModel) {
        self.model = model
        super.init()
        [railScroll, sidebar].forEach { addSubview($0) }
        railScroll.documentView = rail; railScroll.drawsBackground = false
        [space, heading, create, options, search, scroll, error, accountAvatar, accountName, account, mute, inputOptions, deafen, outputOptions, settings].forEach { sidebar.addSubview($0) }
        accountName.setAccessibilityElement(false)
        scroll.documentView = rows
        scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true; scroll.drawsBackground = false
        search.placeholderString = "Search channels"
        search.font = NativeStyle.font(13)
        search.delegate = self
        search.setAccessibilityIdentifier("browse-channels-search")
        search.setAccessibilityLabel("Search channels")
        space.setAccessibilityIdentifier("selected-space-name")
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
        space.frame = NSRect(x: 16, y: 8, width: width - 32, height: 34)
        var externalHeight: CGFloat = 0
        for card in externalOrder {
            let height = card.height(width: width - 32)
            card.frame = NSRect(x: 16, y: 50 + externalHeight, width: width - 32, height: height)
            externalHeight += height + 8
        }
        heading.frame = NSRect(x: 16, y: 54 + externalHeight, width: width - 88, height: 32)
        create.frame = NSRect(x: width - 72, y: 54 + externalHeight, width: 28, height: 32)
        options.frame = NSRect(x: width - 42, y: 54 + externalHeight, width: 28, height: 32)
        search.isHidden = !browsing
        search.frame = NSRect(x: 16, y: 94 + externalHeight, width: width - 32, height: 30)
        let top: CGFloat = (browsing ? 132 : 94) + externalHeight
        let footer = bounds.height - 62
        scroll.frame = NSRect(x: 0, y: top, width: width, height: max(0, footer - top - (error.stringValue.isEmpty ? 0 : 30)))
        error.frame = NSRect(x: 16, y: footer - 30, width: width - 32, height: 28)
        var y: CGFloat = 0
        for (child, defaultHeight) in zip(rowOrder, rowHeights) {
            let height = (child as? NativeVoiceCard)?.height(width: scroll.contentSize.width - 32) ?? defaultHeight
            child.frame = NSRect(x: 16, y: y, width: scroll.contentSize.width - 32, height: height)
            y += height + 3
        }
        rows.frame.size = NSSize(width: scroll.contentSize.width, height: y + 8)
        account.frame = NSRect(x: 16, y: footer + 8, width: width - 156, height: 36)
        accountAvatar.frame = NSRect(x: 16, y: footer + 11, width: 30, height: 30)
        accountName.frame = NSRect(x: 53, y: footer + 17, width: max(0, width - 197), height: 20)
        mute.frame = NSRect(x: width - 138, y: footer + 10, width: 28, height: 32)
        inputOptions.frame = NSRect(x: width - 110, y: footer + 10, width: 16, height: 32)
        deafen.frame = NSRect(x: width - 92, y: footer + 10, width: 28, height: 32)
        outputOptions.frame = NSRect(x: width - 64, y: footer + 10, width: 16, height: 32)
        settings.frame = NSRect(x: width - 46, y: footer + 10, width: 28, height: 32)
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
        heading.title = "\(expanded ? "⌄" : "›")  Channels  \(joined.count)"
        heading.setAccessibilityLabel("Channels, \(joined.count)")
        heading.setAccessibilityValue(expanded ? "Expanded" : "Collapsed")
        create.isHidden = !model.isOwner; options.isHidden = !model.isOwner
        create.isEnabled = model.canCreateChannel
        create.toolTip = model.canCreateChannel ? "Create channel" : "Channel limit reached (\(model.limits?.channelsPerSpace ?? 100))"
        rowOrder = []; rowHeights = []
        updateExternalVoice(joined: joined)
        channelCards = channelCards.filter { id, _ in joined.contains { $0.id == id } }
        LiveNow.listed(joined, expanded: expanded) { VoicePresentation.isLive(model, $0) }.forEach { addChannel($0) }
        if browsing {
            let query = String(search.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).drop(while: { $0 == "#" }))
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let matches = (model.detail?.channels ?? []).filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) }
            addRow(NativeStyle.label("Browse channels", size: 12, bold: true), height: 28)
            if matches.isEmpty { addRow(NativeStyle.label("No channels match your search.", size: 12)) }
            matches.forEach { addChannel($0, browsing: true) }
        }
        for invitation in model.detail?.channelInvitations ?? [] {
            addRow(NativeButton("#\(invitation.channel.name) · from @\(invitation.inviter.username)") { [weak self] in
                self?.show?(.channelInvitation(invitation))
            })
        }
        addRow(NativeStyle.label("Direct messages", size: 12, bold: true), height: 28)
        var nextRequests: [String: NativeButton] = [:]
        if !model.messageRequests.isEmpty {
            requests.title = "\(model.showingMessageRequests ? "⌄" : "›")  Message requests  \(model.messageRequests.count)"
            requests.alignment = .left
            requests.setAccessibilityIdentifier("message-requests")
            requests.setAccessibilityLabel("Message requests")
            requests.setAccessibilityValue("\(model.messageRequests.count) \(model.messageRequests.count == 1 ? "request" : "requests"), \(model.showingMessageRequests ? "Expanded" : "Collapsed")")
            addRow(requests)
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
                    addRow(button)
                }
            }
        }
        if let account = model.account {
            let button = NativeButton("\(account.displayName ?? account.username ?? "You")  · you") { [weak self] in
                guard let self else { return }; Task { await self.model.openSelfDirectMessage() }
            }
            button.setAccessibilityIdentifier("dm-self"); button.alignment = .left
            addRow(button)
        }
        for conversation in model.visibleDirectMessages where conversation.peer.id != model.account?.id {
            let row = NavigationRow(conversation.peer.displayName, symbol: "person", selected: model.selectedDirectMessageID == conversation.id,
                unread: conversation.unread, muted: model.notificationsMuted(.direct(conversation.id)), open: { [weak self] in
                    guard let self else { return }; Task { await self.model.select(directMessage: conversation) }
                })
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
            addRow(row)
        }
        let newMessage = NativeButton("+  New message") { [weak self] in self?.show?(.newDirectMessage) }
        newMessage.setAccessibilityIdentifier("new-message"); newMessage.alignment = .left
        addRow(newMessage)
        requestButtons = nextRequests
        for child in rows.subviews where !rowOrder.contains(where: { $0 === child }) { child.removeFromSuperview() }
        rows.setAccessibilityChildren(rowOrder)
        accountName.stringValue = model.account?.displayName ?? ""
        account.setAccessibilityLabel("Edit profile for \(model.account?.displayName ?? "your account")")
        if let current = model.account { accountAvatar.update(current) }
        mute.image = NSImage(systemSymbolName: model.voice.muted ? "mic.slash" : "mic", accessibilityDescription: nil)
        mute.setAccessibilityLabel(model.voice.muted ? "Unmute microphone" : "Mute microphone")
        mute.setAccessibilityValue(model.voice.muted ? "Muted" : "On")
        deafen.image = NSImage(systemSymbolName: model.voice.deafened ? "speaker.slash" : "headphones", accessibilityDescription: nil)
        deafen.setAccessibilityLabel(model.voice.deafened ? "Undeafen audio" : "Deafen audio")
        deafen.setAccessibilityValue(model.voice.deafened ? "Deafened" : "On")
        error.stringValue = model.voice.error ?? model.notificationError ?? model.error ?? model.directMessagesError ?? ""
        needsLayout = true
    }

    private func addRow(_ view: NSView, height: CGFloat = 32) {
        if view.superview == nil { rows.addSubview(view) }
        rowOrder.append(view); rowHeights.append(height)
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
        if browsing { addRow(row) }
        else {
            let card = channelCards[channel.id] ?? NativeVoiceCard(model: model, channelID: channel.id, heading: row, panels: voicePanels) { [weak self, id = channel.id] in
                guard let self, let channel = self.model.detail?.channels.first(where: { $0.id == id }) else { return }
                Task { await self.model.select(channel: channel) }
            }
            card.changed = { [weak self] in self?.needsLayout = true }
            channelCards[channel.id] = card
            addRow(card)
        }
    }

    private func updateExternalVoice(joined: [Channel]) {
        var next: [String: NativeVoiceCard] = [:]
        externalOrder = []
        func add(id: String, title: String, open: @escaping () -> Void) {
            let button = externalCards[id]?.heading as? NativeButton ?? NativeButton(title, action: open)
            button.title = title; button.setAccessibilityLabel(title)
            button.setAccessibilityIdentifier("voice-call-channel")
            button.alignment = .left; button.font = NativeStyle.font(13, bold: true)
            button.layer?.backgroundColor = NSColor.clear.cgColor
            let card = externalCards[id] ?? NativeVoiceCard(model: model, channelID: id, heading: button, panels: voicePanels, openChat: open)
            card.changed = { [weak self] in self?.needsLayout = true }
            next[id] = card; externalOrder.append(card)
            if card.superview == nil { sidebar.addSubview(card) }
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
            NavigationMenuItem("Log out") { [weak self] in guard let self else { return }; Task { await self.model.logout() } }]
        popup(items, at: settings)
    }

    private func popup(_ items: [NSMenuItem], at view: NSView) {
        NavigationMenus.menu(items).popUp(positioning: nil, at: NSPoint(x: 0, y: view.bounds.maxY), in: view)
    }

    func controlTextDidChange(_ notification: Notification) { render() }
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
