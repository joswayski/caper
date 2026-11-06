import CaperCore
import SwiftUI
import UIKit

/// The open channel: header, timeline and composer. The timeline is a
/// UICollectionView with a diffable data source and self-sizing cells, so only
/// realized rows are measured; the rest keep estimates until scrolled to.
final class ConversationViewController: UIViewController, UICollectionViewDelegate {
    enum Item: Hashable {
        case edge
        case day(String)
        case row(String)
        /// "N blocked messages — Show", keyed by the run's first message.
        case blocked(String)

        var rowID: String? { if case let .row(id) = self { return id } else { return nil } }
        var isRow: Bool { rowID != nil }
    }

    private let model: AppModel
    /// The last session's conversation, shown until the live one is ready.
    var cached: StartupSnapshot? { didSet { if isViewLoaded { render() } } }
    var browse: (() -> Void)?

    private let header = UIView()
    private let channelTitle = UIKitStyle.label(size: 14, weight: .bold)
    private let subtitle = UIKitStyle.label(size: 11, color: UIKitStyle.muted)
    private lazy var back = IconButton(icon: "arrow-left", label: "Back to Browse", size: 20) { [weak self] in self?.browse?() }
    /// In place of an empty timeline: loading, a failed first load with Try
    /// again, or SwiftUI's "No messages yet." with its note.
    private let status = UIKitStyle.label(size: 12, color: UIKitStyle.muted)
    private let statusTitle = UIKitStyle.label(size: 14, weight: .medium)
    private lazy var tryAgain: CaperButton = {
        let button = CaperButton("Try again", kind: .secondary, size: 13) { [weak self] in
            guard let chat = self?.model.chat else { return }
            Task { await chat.retryLoad() }
        }
        button.accessibilityIdentifier = "chat-try-again"
        return button
    }()
    private lazy var statusStack = UIStackView(arrangedSubviews: [statusTitle, status, tryAgain])
    private let typing = TypingIndicatorView()
    private let composer = ComposerView()
    private lazy var members = MembersPanel(model: model)
    private var lastFocusRevision = 0
    private var capabilitiesKey: String?
    private var collection: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, Item>!

    private var rows: [String: MessageRowState] = [:]
    private var days: [String: String] = [:]
    private var blockedRuns: [String: BlockedRunState] = [:]
    /// Blocked messages the reader chose to show, in memory only.
    private var revealedBlocked: Set<String> = []
    private var edge: TimelineEdge?
    /// The conversation whose rows are shown, and whether it is the live one.
    private var shownKey: String?
    /// The conversation whose whole loaded page is shown, not just its first window.
    private var completeKey: String?
    private static let firstWindow = 16
    private var live = false
    private var followingTail = true
    private var arriving: Set<String> = []
    private var contentSizeObservation: NSKeyValueObservation?
    private let timelineObserver = ModelObserver()
    private let composerObserver = ModelObserver()

    init(model: AppModel) {
        self.model = model
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.conversation
        buildHeader()
        buildTimeline()
        buildComposer()
        view.addSubview(members)
        NSLayoutConstraint.activate([
            members.topAnchor.constraint(equalTo: header.bottomAnchor),
            members.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            members.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            members.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        lastFocusRevision = model.chat.focusRevision
        timelineObserver.start { [weak self] in self?.render() }
        registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (self: Self, _) in
            // Message text is sized from the reader's text size; re-render every row.
            var snapshot = self.dataSource.snapshot()
            snapshot.reconfigureItems(snapshot.itemIdentifiers)
            self.dataSource.apply(snapshot, animatingDifferences: false)
        }
        composerObserver.start { [weak self] in self?.renderComposer() }
        UIKitHosting.warmAttachGallery()
        // SwiftUI presents message actions, who reacted, forwarding and block
        // confirmations from an invisible child, as CaperView's conversation does.
        let host = UIHostingController(rootView: UIKitPresentationHost(state: presentations, model: model))
        host.view.backgroundColor = .clear
        host.view.isUserInteractionEnabled = false
        host.view.translatesAutoresizingMaskIntoConstraints = false
        addChild(host)
        view.insertSubview(host.view, at: 0)
        NSLayoutConstraint.activate([
            host.view.widthAnchor.constraint(equalToConstant: 1), host.view.heightAnchor.constraint(equalToConstant: 1),
            host.view.topAnchor.constraint(equalTo: view.topAnchor), host.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
        ])
        host.didMove(toParent: self)
    }

    // MARK: Layout

    private func buildHeader() {
        header.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(header)
        back.tintColor = UIKitStyle.muted
        back.accessibilityIdentifier = "back-to-browse"

        // Phones: the name is the conversation's menu, as in Slack, holding Pins
        // and Members. It fills the header's width.
        chevron.tintColor = UIKitStyle.muted
        chevron.contentMode = .center
        chevron.setContentHuggingPriority(.required, for: .horizontal)
        channelTitle.lineBreakMode = .byTruncatingTail
        channelTitle.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let titleRow = UIStackView(arrangedSubviews: [channelTitle, chevron])
        titleRow.spacing = 4
        titleRow.alignment = .center
        let titles = UIStackView(arrangedSubviews: [titleRow, subtitle])
        titles.axis = .vertical
        titles.alignment = .leading
        titles.spacing = 1
        titles.isUserInteractionEnabled = false
        titles.translatesAutoresizingMaskIntoConstraints = false
        titleMenu.translatesAutoresizingMaskIntoConstraints = false
        titleMenu.showsMenuAsPrimaryAction = true
        titleMenu.accessibilityIdentifier = "channel-title-menu"
        titleMenu.addSubview(titles)
        titleMenu.menu = UIMenu(children: [UIDeferredMenuElement.uncached { [weak self] completion in
            completion(self?.titleMenuItems() ?? [])
        }])
        connection.setContentHuggingPriority(.required, for: .horizontal)
        connection.setContentCompressionResistancePriority(.required, for: .horizontal)
        connection.accessibilityIdentifier = "chat-connection-status"
        connection.isHidden = true
        let rule = UIView()
        rule.backgroundColor = UIKitStyle.border
        rule.translatesAutoresizingMaskIntoConstraints = false
        [back, titleMenu, connection, rule].forEach(header.addSubview)
        // The conversation's name for VoiceOver's heading rotor, beside its menu, as on web.
        heading.accessibilityTraits = .header
        heading.accessibilityIdentifier = "selected-channel-name"
        header.accessibilityElements = [back, heading, titleMenu, connection]
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            header.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            header.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            header.heightAnchor.constraint(equalToConstant: 50),
            back.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 13),
            back.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            titleMenu.leadingAnchor.constraint(equalTo: back.trailingAnchor, constant: 5),
            titleMenu.topAnchor.constraint(equalTo: header.topAnchor, constant: 3),
            titleMenu.bottomAnchor.constraint(equalTo: header.bottomAnchor, constant: -3),
            titles.leadingAnchor.constraint(equalTo: titleMenu.leadingAnchor, constant: 6),
            titles.trailingAnchor.constraint(lessThanOrEqualTo: titleMenu.trailingAnchor, constant: -6),
            titles.centerYAnchor.constraint(equalTo: titleMenu.centerYAnchor),
            connection.leadingAnchor.constraint(equalTo: titleMenu.trailingAnchor, constant: 5),
            connection.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -10),
            connection.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            rule.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            rule.trailingAnchor.constraint(equalTo: header.trailingAnchor),
            rule.bottomAnchor.constraint(equalTo: header.bottomAnchor),
            rule.heightAnchor.constraint(equalToConstant: 1),
        ])
    }
    private let titleMenu = UIButton(type: .custom)
    private let chevron = UIImageView(image: UIKitStyle.icon("chevron-down")?.resized(to: 14))
    private let connection = UIKitStyle.label(size: 11, weight: .bold, color: UIKitStyle.muted)
    private lazy var heading = UIAccessibilityElement(accessibilityContainer: header)
    private var connectionTimer: Timer?

    /// Pins, then Members with the space's count.
    private func titleMenuItems() -> [UIMenuElement] {
        var items: [UIMenuElement] = [
            UIAction(title: "Pins", image: UIKitStyle.icon("pin")) { [weak self] _ in self?.showPins() },
        ]
        members.openProfile = { [weak self] member in
            guard let self else { return }
            self.present(UIKitHosting.profileCard(model: self.model, member: member) { [weak self] target in
                self?.presentations.block = target
            }, animated: !UIKitStyle.reduceMotion)
        }
        if membersAvailable {
            let title = members.isOpen ? "Hide members" : "Members"
            items.append(UIAction(title: memberCount.map { "\(title) · \($0)" } ?? title, image: UIKitStyle.icon("users")) { [weak self] _ in
                self?.toggleMembers()
            })
        }
        // A 1:1 DM: block or unblock the other person.
        if let conversation = peerConversation {
            let peer = conversation.peer
            if conversation.blocked || model.blockedIDs.contains(peer.id) {
                let unblock = UIAction(title: "Unblock @\(peer.username)") { [weak self] _ in
                    guard let model = self?.model else { return }
                    UIKitDirectBar.unblock(model: model, accountID: peer.id)
                }
                unblock.accessibilityIdentifier = "dm-unblock"
                items.append(UIMenu(options: .displayInline, children: [unblock]))
            } else if peer.canBlock {
                let block = UIAction(title: "Block @\(peer.username)…", attributes: .destructive) { [weak self] _ in
                    self?.presentations.block = BlockTarget(peer: peer)
                }
                block.accessibilityIdentifier = "dm-block"
                items.append(UIMenu(options: .displayInline, children: [block]))
            }
        }
        return items
    }

    private func toggleMembers() {
        if members.isOpen { members.close() } else { view.endEditing(true); members.open() }
    }

    private func showPins() {
        let pins = PinsViewController(model: model)
        pins.requestBlock = { [weak self] target in self?.presentations.block = target }
        if let sheet = pins.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
        present(pins, animated: true)
    }

    /// The member list applies to a channel, not a preview.
    /// A feedback visit has no member list.
    private var membersAvailable: Bool { isLive && !direct && model.visit == nil && !model.previewingChannel && model.detail != nil }

    private func buildTimeline() {
        let size = NSCollectionLayoutSize(widthDimension: .fractionalWidth(1), heightDimension: .estimated(64))
        let section = NSCollectionLayoutSection(group: .vertical(layoutSize: size, subitems: [NSCollectionLayoutItem(layoutSize: size)]))
        section.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 0, bottom: 8, trailing: 0)
        let layout = UICollectionViewCompositionalLayout(section: section)
        collection = UICollectionView(frame: .zero, collectionViewLayout: layout)
        collection.translatesAutoresizingMaskIntoConstraints = false
        collection.backgroundColor = .clear
        collection.alwaysBounceVertical = true
        collection.keyboardDismissMode = .interactive
        collection.delegate = self
        collection.accessibilityIdentifier = "message-timeline"
        collection.register(MessageCell.self, forCellWithReuseIdentifier: MessageCell.reuse)
        collection.register(DayDividerCell.self, forCellWithReuseIdentifier: DayDividerCell.reuse)
        collection.register(BlockedRunCell.self, forCellWithReuseIdentifier: BlockedRunCell.reuse)
        collection.register(TimelineEdgeCell.self, forCellWithReuseIdentifier: TimelineEdgeCell.reuse)
        dataSource = UICollectionViewDiffableDataSource(collectionView: collection) { [weak self] collection, indexPath, item in
            guard let self else { return nil }
            switch item {
            case .edge:
                let cell = collection.dequeueReusableCell(withReuseIdentifier: TimelineEdgeCell.reuse, for: indexPath) as! TimelineEdgeCell
                cell.retry = { [weak self] in self?.loadOlder(force: true) }
                cell.configure(self.edge ?? .beginning)
                return cell
            case let .day(key):
                let cell = collection.dequeueReusableCell(withReuseIdentifier: DayDividerCell.reuse, for: indexPath) as! DayDividerCell
                cell.configure(self.days[key] ?? "")
                return cell
            case let .blocked(key):
                let cell = collection.dequeueReusableCell(withReuseIdentifier: BlockedRunCell.reuse, for: indexPath) as! BlockedRunCell
                if let run = self.blockedRuns[key] {
                    cell.configure(run) { [weak self] in
                        guard let self else { return }
                        self.revealedBlocked = run.toggling(self.revealedBlocked)
                        self.render()
                    }
                }
                return cell
            case let .row(id):
                let cell = collection.dequeueReusableCell(withReuseIdentifier: MessageCell.reuse, for: indexPath) as! MessageCell
                cell.row.host = self.rowHost
                if let row = self.rows[id] {
                    cell.retry = row.pending ? { [weak self] in Task { await self?.model.chat.send(retryPending: true) } } : nil
                    cell.configure(row)
                }
                return cell
            }
        }
        // Self-sizing rows change the content height as they are realized; while
        // following the live end, stay pinned to it.
        contentSizeObservation = collection.observe(\.contentSize, options: [.old, .new]) { [weak self] _, change in
            MainActor.assumeIsolated {
                guard let self, change.oldValue?.height != change.newValue?.height else { return }
                self.pinTailIfFollowing()
            }
        }
        view.addSubview(collection)
        embed(voiceStrip)
        embed(notices)
        for label in [statusTitle, status] { label.textAlignment = .center; label.numberOfLines = 0 }
        statusStack.axis = .vertical
        statusStack.alignment = .center
        statusStack.spacing = 8
        statusStack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(statusStack)
        view.addSubview(typing)
        NSLayoutConstraint.activate([
            voiceStrip.view.topAnchor.constraint(equalTo: header.bottomAnchor),
            voiceStrip.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            voiceStrip.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            collection.topAnchor.constraint(equalTo: voiceStrip.view.bottomAnchor),
            notices.view.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            notices.view.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            notices.view.topAnchor.constraint(equalTo: collection.bottomAnchor),
            collection.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            collection.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            statusStack.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            statusStack.topAnchor.constraint(equalTo: collection.topAnchor, constant: 80),
            statusStack.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -48),
            typing.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 18),
            typing.trailingAnchor.constraint(lessThanOrEqualTo: view.trailingAnchor, constant: -18),
            typing.topAnchor.constraint(equalTo: notices.view.bottomAnchor),
        ])
    }

    private func buildComposer() {
        view.addSubview(composer)
        composer.model = model
        composer.changed = { [weak self] text in
            guard let chat = self?.model.chat else { return }
            chat.draft = text
            chat.draftChanged(text)
        }
        composer.submitted = { [weak self] in self?.send() }
        composer.attachTapped = { [weak self] in
            guard let self, let drawer = UIKitHosting.attachDrawer(chat: self.model.chat) else { return }
            self.present(drawer, animated: false)
        }
        composer.emojiTapped = { [weak self] in
            guard let self else { return }
            let composer = self.composer
            self.present(UIKitHosting.expressionPicker(chat: self.model.chat) { composer.insert($0) }, animated: true)
        }
        NSLayoutConstraint.activate([
            composer.topAnchor.constraint(equalTo: typing.bottomAnchor),
            composer.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            composer.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            composer.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor, constant: -8),
        ])
        // An incoming request or a blocked peer replaces the composer.
        view.addSubview(directBar)
        directBar.isHidden = true
        NSLayoutConstraint.activate([
            directBar.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            directBar.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            directBar.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
        ])
    }

    // MARK: Rendering

    /// Your call, or Join for the open channel's voice: SwiftUI's PhoneVoiceStrip.
    private lazy var voiceStrip = Self.sized(UIKitVoiceStrip(model: model) { [weak self] in self?.browse?() })
    /// A failed action or a voice error, above the composer.
    private lazy var notices = Self.sized(UIKitComposerNotices(model: model))

    /// A SwiftUI child that sizes to its content, collapsing when it draws nothing.
    private static func sized<Content: View>(_ root: Content) -> UIHostingController<Content> {
        let host = UIHostingController(rootView: root)
        host.sizingOptions = .intrinsicContentSize
        host.view.backgroundColor = .clear
        host.view.translatesAutoresizingMaskIntoConstraints = false
        host.view.setContentHuggingPriority(.required, for: .vertical)
        host.view.setContentCompressionResistancePriority(.required, for: .vertical)
        return host
    }

    private func embed(_ child: UIViewController) {
        addChild(child)
        view.addSubview(child.view)
        child.didMove(toParent: self)
    }

    private let directBar = HostedView()
    private var directBarKey: String?
    private var directBarHeight: NSLayoutConstraint?

    /// The open conversation: a channel, or a DM.
    /// The open channel or DM, or the conversation a feedback visit reads.
    private var conversationID: String? { model.selectedChannelID ?? model.selectedDirectMessageID ?? model.visit?.channelID }
    private var direct: Bool { model.selectedDirectMessageID != nil }
    /// The open DM's peer, other than yourself.
    private var peerConversation: DirectMessageConversation? {
        model.selectedDirectMessage.flatMap { $0.peer.id == model.account?.id ? nil : $0 }
    }

    /// Live once the model has opened this conversation; until then, the cache.
    private var isLive: Bool {
        model.phase == .ready && !model.restoringWorkspace && conversationID != nil
            && !(model.chat.loading && model.chat.messages.isEmpty && cached?.messages.isEmpty == false)
    }

    private weak var thread: ThreadViewController?

    /// Opening a thread (View thread, Reply in thread, a jump) covers the conversation.
    private func presentThreadIfNeeded() {
        guard model.chat.threadRootID != nil, thread?.presentingViewController == nil, view.window != nil else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, self.thread?.presentingViewController == nil, self.model.chat.threadRootID != nil else { return }
            let controller = ThreadViewController(model: self.model)
            self.thread = controller
            controller.closed = { [weak self] in self?.thread = nil }
            self.view.endEditing(true)
            let top = self.presentedViewController ?? self
            if top !== self { self.dismiss(animated: false) { self.present(controller, animated: !UIKitStyle.reduceMotion) } }
            else { self.present(controller, animated: !UIKitStyle.reduceMotion) }
        }
    }

    private func render() {
        let chat = model.chat
        presentThreadIfNeeded()
        let live = isLive
        let channelID = live ? conversationID : cached?.channelID
        let direct = live && self.direct
        let channelName = live ? (model.selectedChannel?.name ?? chat.channelName) : (cached?.channelName ?? "")
        let visit = live ? model.visit : nil
        // A DM reads as the person's name; a channel as "# name"; a visit as
        // "#plans · Studio" or "Alice & Bob", from the visited history.
        let title = visit.map { FeedbackInbox.title(channelName: chat.channelName, spaceName: chat.spaceName, direct: $0.direct) }
            ?? (channelName.isEmpty ? "" : direct ? channelName : "# \(channelName.lowercased())")
        channelTitle.text = title
        let inVoice = live ? UIKitVoiceStrip.inVoice(model) : 0
        let parts = [visit == nil ? memberSummary(live: live, channelID: channelID) : nil, inVoice > 0 ? "\(inVoice) in voice" : nil]
        subtitle.text = parts.compactMap { $0 }.isEmpty ? nil : parts.compactMap { $0 }.joined(separator: " · ")
        subtitle.isHidden = subtitle.text == nil
        heading.accessibilityLabel = visit != nil ? "Visiting \(title)" : direct ? "Direct message with \(channelName)" : title
        titleMenu.accessibilityLabel = visit != nil ? "\(title) options"
            : direct ? "\(channelName) options" : "#\(channelName.lowercased()) options"
        titleMenu.accessibilityValue = subtitle.text
        titleMenu.isEnabled = live
        renderConnection(live ? chat.liveState : .connected)

        var messages: [ChatMessage]
        var pending: PendingMessage?
        if live {
            messages = chat.displayedChannelMessages.filter { $0.channelId == channelID }
            pending = chat.pendingMessage.flatMap { $0.threadRootId == nil ? $0 : nil }
            edge = chat.olderError != nil ? .failed : chat.hasMore ? (chat.loadingOlder ? .loading : nil) : .beginning
            if chat.loading && messages.isEmpty { edge = nil }
        } else {
            messages = cached?.messages ?? []
            // A cached page may have older history; say nothing until the live page arrives.
            edge = nil
        }
        // A newly shown conversation paints only its newest rows in the first
        // frame; the rest of the page follows a turn later, above them.
        if channelID != completeKey {
            completeKey = channelID
            if messages.count > Self.firstWindow {
                messages = Array(messages.suffix(Self.firstWindow))
                edge = nil
                AfterFrame.run { [weak self] in self?.render() }
            }
        }
        let viewer = model.account?.id ?? cached?.accountID
        let typingText = live && !chat.typingNames.isEmpty ? Self.typingLabel(chat.typingNames) : ""
        typing.show(typingText)
        var emptyTitle: String?, note: String?, titleColor = UIKitStyle.text
        if live, chat.loadFailed, let error = chat.error { note = error }
        else if live, let error = chat.error, messages.isEmpty { note = error }
        else if live, chat.loading && messages.isEmpty { note = "Loading messages…" }
        else if live, messages.isEmpty, pending == nil {
            if let conversation = model.selectedDirectMessage, conversation.peer.id == model.account?.id {
                emptyTitle = "You can message yourself here to keep notes, reminders, and ideas."
                titleColor = UIKitStyle.muted
            } else {
                emptyTitle = "No messages yet."
                note = model.selectedDirectMessageID == nil ? "Start the conversation in #\(chat.channelName.lowercased())."
                    : FeedbackInbox.emptyDirectNote(peer: model.selectedDirectMessage?.peer, name: chat.channelName)
            }
        }
        statusTitle.text = emptyTitle
        statusTitle.textColor = titleColor
        statusTitle.isHidden = emptyTitle == nil
        status.text = note
        status.isHidden = note == nil
        tryAgain.isHidden = !(live && chat.loadFailed)
        statusStack.isHidden = emptyTitle == nil && note == nil

        apply(messages: messages, pending: pending, author: chat.currentAuthor, viewerID: viewer,
              key: channelID, live: live, error: chat.error, rejected: chat.sendRejected,
              blocked: live ? chat.blockedAuthorIDs : [])
    }

    private func apply(messages: [ChatMessage], pending: PendingMessage?, author: ChatAuthor?, viewerID: String?,
                       key: String?, live: Bool, error: String?, rejected: Bool, blocked: Set<String>) {
        if key != shownKey { revealedBlocked = [] }
        var built = TimelineRows.build(messages, pending: pending, author: author, viewerID: viewerID,
                                       focusedID: live ? model.chat.focusedMessageID : nil, chat: live ? model.chat : nil,
                                       error: error, rejected: rejected, blocked: blocked, revealed: revealedBlocked)
        if edge != nil { built.items.insert(.edge, at: 0) }
        let items = built.items, nextRows = built.rows, nextDays = built.days, nextRuns = built.runs

        let switched = key != shownKey
        // Uploads and GIFs are optional server configuration; recheck per conversation.
        if live, key != capabilitiesKey {
            capabilitiesKey = key
            let chat = model.chat
            Task { await chat.checkUploadAvailability(); await chat.checkKlipyAvailability() }
        }
        let oldItems = dataSource.snapshot().itemIdentifiers
        let oldFirstRow = oldItems.first(where: \.isRow)
        let anchor = switched ? nil : visibleAnchor()
        let atTail = switched || followingTail
        // Rows that are new at the live end while the reader watches it arrive with motion.
        let appended: Set<String> = !switched && live && self.live && atTail
            ? Set(items.compactMap(\.rowID).filter { rows[$0] == nil }) : []
        let prepended = !switched && oldFirstRow != nil && items.first(where: \.isRow) != oldFirstRow

        var snapshot = NSDiffableDataSourceSnapshot<Int, Item>()
        snapshot.appendSections([0])
        snapshot.appendItems(items)
        let survivors = Set(oldItems)
        let changed = items.filter { item in
            guard survivors.contains(item) else { return false }
            switch item {
            case let .row(id): return rows[id] != nextRows[id]
            case let .day(day): return days[day] != nextDays[day]
            case let .blocked(run): return blockedRuns[run] != nextRuns[run]
            case .edge: return true
            }
        }
        let reload = changed.filter { if case let .row(id) = $0 { nextRows[id]?.remeasures(from: rows[id]) == true } else { false } }
        snapshot.reloadItems(reload)
        snapshot.reconfigureItems(changed.filter { !reload.contains($0) })
        rows = nextRows
        days = nextDays
        blockedRuns = nextRuns
        shownKey = key
        self.live = live
        arriving.formUnion(appended)

        // A jump (Go to message) replaces the window and centres its target.
        let jump = live && model.chat.focusRevision != lastFocusRevision ? model.chat.focusedMessageID : nil
        if live { lastFocusRevision = model.chat.focusRevision }
        let jumpItem = jump.flatMap { id in messages.first { $0.id == id } }.map { Item.row($0.clientMessageId) }
        if jumpItem != nil { followingTail = false }

        dataSource.apply(snapshot, animatingDifferences: false) { [weak self] in
            guard let self else { return }
            if let jumpItem, let path = self.dataSource.indexPath(for: jumpItem) {
                self.collection.layoutIfNeeded()
                self.collection.scrollToItem(at: path, at: .centeredVertically, animated: false)
            } else if atTail && jump == nil && !(prepended && anchor != nil) {
                self.scrollToTail(animated: !switched && !appended.isEmpty && !UIKitStyle.reduceMotion)
            } else if let anchor {
                self.restore(anchor)
            }
        }
    }

    // MARK: Scrolling

    private struct Anchor { let item: Item; let offset: CGFloat }

    /// The first visible message row and its distance from the top edge.
    private func visibleAnchor() -> Anchor? {
        let top = collection.contentOffset.y + collection.adjustedContentInset.top
        let visible = collection.indexPathsForVisibleItems.sorted()
        for path in visible {
            guard let item = dataSource.itemIdentifier(for: path), case .row = item,
                  let frame = collection.layoutAttributesForItem(at: path)?.frame, frame.maxY > top else { continue }
            return Anchor(item: item, offset: frame.minY - top)
        }
        return nil
    }

    /// History prepends keep the row under the reader's eye where it was.
    private func restore(_ anchor: Anchor) {
        guard let path = dataSource.indexPath(for: anchor.item) else { return }
        collection.layoutIfNeeded()
        guard let frame = collection.layoutAttributesForItem(at: path)?.frame else { return }
        let target = frame.minY - anchor.offset - collection.adjustedContentInset.top
        collection.contentOffset.y = max(-collection.adjustedContentInset.top, target)
    }

    private var tailOffset: CGFloat {
        max(-collection.adjustedContentInset.top,
            collection.contentSize.height + collection.adjustedContentInset.bottom - collection.bounds.height)
    }

    private func scrollToTail(animated: Bool) {
        followingTail = true
        collection.layoutIfNeeded()
        collection.setContentOffset(CGPoint(x: 0, y: tailOffset), animated: animated)
    }

    private func pinTailIfFollowing() {
        guard followingTail, !collection.isTracking, !collection.isDecelerating else { return }
        let target = tailOffset
        if abs(collection.contentOffset.y - target) > 0.5 { collection.contentOffset.y = target }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        pinTailIfFollowing()
    }

    func scrollViewDidScroll(_ scrollView: UIScrollView) {
        let bottom = scrollView.contentSize.height + scrollView.adjustedContentInset.bottom - scrollView.bounds.height
        // Only the reader's own scrolling changes following or pages history.
        guard scrollView.isTracking || scrollView.isDecelerating else { return }
        // After a jump the window ends before the live end: page newer history instead of following it.
        let nearBottom = scrollView.contentOffset.y >= bottom - 48
        followingTail = nearBottom && !model.chat.hasNewer
        if scrollView.contentOffset.y + scrollView.adjustedContentInset.top < 800 { loadOlder(force: false) }
        if scrollView.contentOffset.y > bottom - 800, live, model.chat.hasNewer, !model.chat.loadingNewer {
            Task { await model.chat.loadNewer() }
        }
    }

    private func loadOlder(force: Bool) {
        let chat = model.chat
        guard live, chat.hasMore, !chat.loadingOlder, !chat.loading, force || chat.olderError == nil else { return }
        Task { await chat.loadOlder() }
    }

    func collectionView(_ collectionView: UICollectionView, willDisplay cell: UICollectionViewCell, forItemAt indexPath: IndexPath) {
        guard case let .row(id) = dataSource.itemIdentifier(for: indexPath), arriving.remove(id) != nil else { return }
        (cell as? MessageCell)?.arrive()
    }

    // MARK: Composer

    private let presentations = UIKitPresentations()
    private lazy var rowHost: RowHost = {
        var host = RowHost(chat: model.chat, present: presentHosted) { [weak self] message in
            guard let chat = self?.model.chat else { return }
            Task { _ = await chat.onOpenForward?(message) }
        }
        host.actions = { [weak self] id, picker in self?.openActions(id, picker: picker) }
        host.reactors = { [weak self] id, emoji in self?.presentations.reactors = .init(messageID: id, emoji: emoji) }
        host.openThread = { [weak self] root in
            guard let chat = self?.model.chat else { return }
            Task { await chat.openThread(root) }
        }
        let chat = model.chat
        host.openProfiles(model: model, controller: self, messages: { chat.displayedChannelMessages }) { [weak self] target in
            self?.presentations.block = target
        }
        return host
    }()

    /// The long-press sheet for a loaded message; `picker` opens its emoji grid.
    private func openActions(_ id: String, picker: Bool) {
        guard let message = model.chat.messages.first(where: { $0.id == id }) else { return }
        view.endEditing(true)
        if picker {
            let chat = model.chat
            present(UIKitHosting.reactionPicker { emoji in
                Task { await chat.setReaction(messageID: id, emoji: emoji, active: true) }
            }, animated: true)
        } else {
            presentations.actions = message
        }
    }
    private lazy var voice = UIKitVoiceComposer(chat: model.chat) { [weak self] words in
        self?.composer.insert(words, spaced: true) ?? false
    }
    private var presentHosted: UIKitHosting.Present {
        { [weak self] controller in self?.presentedViewController == nil ? self?.present(controller, animated: false) : nil }
    }

    private func renderComposer() {
        let chat = model.chat
        let name = model.selectedChannel?.name ?? cached?.channelName ?? chat.channelName
        let ready = isLive && chat.canInteract && !model.previewingChannel
        renderDirectBar()
        composer.configure(prompt: isLive && (direct || model.visit?.direct == true) ? "Message \(chat.channelName)" : "Message #\(name.lowercased())", enabled: ready,
                           canSend: chat.canPressSend(inThread: false),
                           canAttach: chat.canAttach,
                           attachReason: chat.canAttach ? nil : chat.attachUnavailableReason,
                           canPickExpressions: ready)
        composer.show(draft: chat.draft)
        composer.refreshContext()
        composer.textView.pasteAttachments = chat.canAttach ? { [weak chat] in chat?.pasteAttachments() ?? false } : nil
        composer.showAccessories(ready && UIKitComposerAccessories.shows(chat: chat)
                                 ? UIKitComposerAccessories(chat: chat, present: presentHosted) : nil)
        composer.showVoice(button: ready ? voice.button : nil, bar: voice.bar, recording: ready && voice.isRecording)
    }

    /// Accept, Decline or Block in a request; Unblock with a blocked peer.
    private func renderDirectBar() {
        let shows = isLive && UIKitDirectBar.shows(model: model)
        let key = shows ? "\(conversationID ?? "")|\(model.previewingChannel)|\(model.selectedDirectMessage?.status == .incoming)" : nil
        guard key != directBarKey else { return }
        directBarKey = key
        directBar.show(shows ? UIKitDirectBar(model: model) { [weak self] target in self?.presentations.block = target } : nil)
        directBar.isHidden = !shows
        composer.isHidden = shows
        composer.accessibilityElementsHidden = shows
        if shows { view.endEditing(true) }
    }

    private func send() {
        let chat = model.chat
        guard isLive, chat.canPressSend(inThread: false) else { return }
        followingTail = true
        Task { await chat.send() }
    }

    // MARK: Text

    /// The space's member count, under a joined channel's name (SwiftUI's headerSubtitle).
    private func memberSummary(live: Bool, channelID: String?) -> String? {
        guard let detail = live ? model.detail : cached?.detail, detail.space.demo != true, let channelID,
              let channel = detail.channels.first(where: { $0.id == channelID }), channel.joined,
              !live || !model.previewingChannel else { return nil }
        let count = detail.members.count
        return count == 1 ? "1 member" : "\(count) members"
    }

    private var memberCount: Int? {
        guard membersAvailable, let detail = model.detail, detail.space.demo != true, model.selectedChannel?.joined == true else { return nil }
        return detail.members.count
    }

    /// "Offline" or "Connecting…" once the live connection has been down for a second.
    private func renderConnection(_ state: GatewayState) {
        guard state != .connected else {
            connectionTimer?.invalidate(); connectionTimer = nil
            connection.isHidden = true
            return
        }
        connection.text = state == .disconnected ? "Offline" : "Connecting…"
        guard connection.isHidden, connectionTimer == nil else { return }
        connectionTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.connectionTimer = nil
                self?.connection.isHidden = self?.model.chat.liveState == .connected
            }
        }
    }

    static func typingLabel(_ names: [String]) -> String {
        if names.count > 2 { return "Several people are typing…" }
        return "\(names.joined(separator: " and ")) \(names.count == 1 ? "is" : "are") typing…"
    }
}

/// Timeline items from messages: day dividers, grouping (the same person
/// within minutes reads as one block), and the optimistic pending row.
enum TimelineRows {
    struct Built {
        var items: [ConversationViewController.Item] = []
        var rows: [String: MessageRowState] = [:]
        var days: [String: String] = [:]
        var runs: [String: BlockedRunState] = [:]
    }

    @MainActor static func build(_ messages: [ChatMessage], pending: PendingMessage?, author: ChatAuthor?, viewerID: String?,
                                 focusedID: String?, chat: ChatModel?, error: String?, rejected: Bool, inThread: Bool = false,
                                 blocked: Set<String> = [], revealed: Set<String> = []) -> Built {
        var built = Built()
        var previous: ChatMessage?
        // Runs of blocked authors' messages collapse to one row; a revealed run
        // shows its messages after that row, marked as shown.
        var runStarts: [String: BlockedRunState] = [:]
        var hidden: Set<String> = []
        var shown: Set<String> = []
        for entry in UIKitTimelineEntry.entries(messages, blocked: blocked, viewerID: viewerID, revealed: revealed) where entry.blocked {
            guard let first = entry.messages.first else { continue }
            runStarts[first.id] = BlockedRunState(entry: entry)
            let ids = entry.messages.map(\.id)
            if entry.revealed { shown.formUnion(ids) } else { hidden.formUnion(ids) }
        }
        var dayReference: ChatMessage?
        for (index, message) in messages.enumerated() {
            if dayReference.map({ !NativePresentation.sameDay($0.createdAt, message.createdAt) }) ?? true,
               let label = NativePresentation.dayLabel(message.createdAt) {
                let key = "day-\(message.clientMessageId)"
                built.days[key] = label
                built.items.append(.day(key))
                previous = nil
            }
            dayReference = message
            if let run = runStarts[message.id] {
                built.runs[run.key] = run
                built.items.append(.blocked(run.key))
                previous = nil
            }
            if hidden.contains(message.id) { continue }
            let next = index + 1 < messages.count ? messages[index + 1] : nil
            let grouped = NativePresentation.grouped(message, after: previous)
            let continued = next.map {
                runStarts[$0.id] == nil && !hidden.contains($0.id)
                    && NativePresentation.sameDay(message.createdAt, $0.createdAt) && NativePresentation.grouped($0, after: message)
            } ?? false
            built.rows[message.clientMessageId] = MessageRowState(message: message, grouped: grouped, continued: continued, viewerID: viewerID,
                                                                  focused: focusedID == message.id, chat: chat, inThread: inThread)
            if shown.contains(message.id) { built.rows[message.clientMessageId]?.revealedBlocked = true }
            built.items.append(.row(message.clientMessageId))
            previous = message
        }
        if let pending, built.rows[pending.id] == nil {
            let groupedPending = previous.map { $0.author.id == author?.id && NativePresentation.sameDay($0.createdAt, pending.createdAt) } ?? false
            if let last = previous, groupedPending { built.rows[last.clientMessageId]?.continued = true }
            // Read the draft only for a rejected send, so typing doesn't rebuild rows.
            let canEdit = !(rejected && error != nil) || chat.map {
                inThread ? $0.threadDraft.isEmpty && $0.threadAttachmentDrafts.isEmpty : $0.draft.isEmpty && $0.attachmentDrafts.isEmpty
            } ?? true
            built.rows[pending.id] = MessageRowState(pending: pending, author: author, grouped: groupedPending, error: error, rejected: rejected,
                                                     uploadFailed: error != nil && chat?.sendUploadFailed == true, canEdit: canEdit)
            built.items.append(.row(pending.id))
        }
        return built
    }
}
