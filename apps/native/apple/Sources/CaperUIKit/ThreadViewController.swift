import CaperCore
import SwiftUI
import UIKit

/// A thread on iPhone (SwiftUI's NativeThreadView in its full-screen cover):
/// back, "Thread in #channel", the root message, its replies and a reply
/// composer with "Also send to #channel". Closing ends the thread in the model.
final class ThreadViewController: UIViewController, UICollectionViewDelegate {
    typealias Item = ConversationViewController.Item

    private let model: AppModel
    private var chat: ChatModel { model.chat }
    /// The replies' first page is loading: the edge row shows the skeleton.
    private var firstLoad = false
    private let subtitle = UIKitStyle.label(size: 11, color: UIKitStyle.muted)
    private let status = UIKitStyle.label(size: 12, color: UIKitStyle.muted)
    private let composer = ComposerView()
    private let broadcast = UIButton(type: .custom)
    private var collection: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, Item>!
    private var rows: [String: MessageRowState] = [:]
    private var days: [String: String] = [:]
    private var runs: [String: BlockedRunState] = [:]
    private var revealed: Set<String> = []
    private var shownRoot: String?
    private var followingTail = true
    /// Nil until the first render, so a thread opened on a focused reply centres it.
    private var lastFocusRevision: Int?
    private let observer = ModelObserver()
    private let composerObserver = ModelObserver()
    private let presentations = UIKitPresentations()
    private var closing = false
    private var lastVisible: String?
    private var broadcastRow: UIView?
    private lazy var fileDrop = UIKitFileDrop(model: model, inThread: true)
    /// The preview note, a failed action, Retry session and the pending note.
    private lazy var notices: UIHostingController<UIKitThreadNotices> = {
        let host = UIHostingController(rootView: UIKitThreadNotices(chat: model.chat, direct: model.selectedDirectMessageID != nil))
        host.sizingOptions = .intrinsicContentSize
        host.view.backgroundColor = .clear
        host.view.setContentHuggingPriority(.required, for: .vertical)
        host.view.setContentCompressionResistancePriority(.required, for: .vertical)
        return host
    }()
    var closed: (() -> Void)?

    init(model: AppModel) {
        self.model = model
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .fullScreen
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private lazy var rowHost: RowHost = {
        var host = RowHost(chat: model.chat, present: { [weak self] in
            if self?.presentedViewController == nil { self?.present($0, animated: false) }
        }) { [weak self] message in
            guard let chat = self?.model.chat else { return }
            Task { _ = await chat.onOpenForward?(message) }
        }
        host.actions = { [weak self] id, picker in self?.openActions(id, picker: picker) }
        host.reactors = { [weak self] id, emoji in self?.presentations.reactors = .init(messageID: id, emoji: emoji) }
        let chat = model.chat
        host.openProfiles(model: model, controller: self, messages: { chat.displayedMessages + chat.displayedThreadMessages }) { [weak self] target in
            self?.presentations.block = target
        }
        return host
    }()

    private lazy var voice = UIKitVoiceComposer(chat: model.chat, inThread: true) { [weak self] words in
        self?.composer.insert(words, spaced: true) ?? false
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.conversation
        view.accessibilityIdentifier = "thread-view"

        let header = UIView()
        header.translatesAutoresizingMaskIntoConstraints = false
        let back = IconButton(icon: "arrow-left", label: direct ? "Back to conversation" : "Back to channel", size: 20) { [weak self] in self?.close() }
        back.tintColor = UIKitStyle.muted
        back.accessibilityIdentifier = "thread-back"
        let title = UIKitStyle.label("Thread", size: 15, weight: .bold)
        title.accessibilityTraits = .header
        subtitle.lineBreakMode = .byTruncatingTail
        let titles = UIStackView(arrangedSubviews: [title, subtitle])
        titles.axis = .vertical
        titles.spacing = 3
        titles.translatesAutoresizingMaskIntoConstraints = false
        let rule = UIView()
        rule.backgroundColor = UIKitStyle.border
        rule.translatesAutoresizingMaskIntoConstraints = false
        [back, titles, rule].forEach(header.addSubview)
        view.addSubview(header)

        let size = NSCollectionLayoutSize(widthDimension: .fractionalWidth(1), heightDimension: .estimated(64))
        let section = NSCollectionLayoutSection(group: .vertical(layoutSize: size, subitems: [NSCollectionLayoutItem(layoutSize: size)]))
        section.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 0, bottom: 8, trailing: 0)
        collection = UICollectionView(frame: .zero, collectionViewLayout: UICollectionViewCompositionalLayout(section: section))
        collection.translatesAutoresizingMaskIntoConstraints = false
        collection.backgroundColor = .clear
        collection.alwaysBounceVertical = true
        collection.showsVerticalScrollIndicator = false
        collection.keyboardDismissMode = .interactive
        collection.delegate = self
        collection.accessibilityIdentifier = "thread-timeline"
        collection.register(MessageCell.self, forCellWithReuseIdentifier: MessageCell.reuse)
        collection.register(DayDividerCell.self, forCellWithReuseIdentifier: DayDividerCell.reuse)
        collection.register(BlockedRunCell.self, forCellWithReuseIdentifier: BlockedRunCell.reuse)
        collection.register(TimelineEdgeCell.self, forCellWithReuseIdentifier: TimelineEdgeCell.reuse)
        dataSource = UICollectionViewDiffableDataSource(collectionView: collection) { [weak self] collection, path, item in
            guard let self else { return nil }
            switch item {
            case .edge:
                let cell = collection.dequeueReusableCell(withReuseIdentifier: TimelineEdgeCell.reuse, for: path) as! TimelineEdgeCell
                cell.retry = { [weak self] in Task { await self?.chat.loadThread() } }
                let chat = self.chat
                cell.configure(chat.threadError != nil ? .failed : self.firstLoad ? .firstLoad : .loading,
                               error: chat.threadError)
                return cell
            case let .day(key):
                let cell = collection.dequeueReusableCell(withReuseIdentifier: DayDividerCell.reuse, for: path) as! DayDividerCell
                cell.configure(self.days[key] ?? "", plain: key == "replies")
                return cell
            case .newMessages:
                // Channels only; threads have no unread line.
                return collection.dequeueReusableCell(withReuseIdentifier: DayDividerCell.reuse, for: path)
            case let .blocked(key):
                let cell = collection.dequeueReusableCell(withReuseIdentifier: BlockedRunCell.reuse, for: path) as! BlockedRunCell
                if let run = self.runs[key] {
                    cell.configure(run) { [weak self] in
                        guard let self else { return }
                        self.revealed = run.toggling(self.revealed)
                        self.render()
                    }
                }
                return cell
            case let .row(id):
                let cell = collection.dequeueReusableCell(withReuseIdentifier: MessageCell.reuse, for: path) as! MessageCell
                cell.row.host = self.rowHost
                if let row = self.rows[id] {
                    cell.retry = row.pending ? { [weak self] in Task { await self?.chat.send(inThread: true, retryPending: true) } } : nil
                    cell.configure(row)
                }
                return cell
            }
        }
        view.addSubview(collection)
        status.numberOfLines = 0
        status.textAlignment = .center
        view.addSubview(status)

        // "Also send to #channel": a checkbox that reads as a switch to VoiceOver.
        var configuration = UIButton.Configuration.plain()
        configuration.imagePadding = 8
        configuration.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0)
        broadcast.configuration = configuration
        broadcast.contentHorizontalAlignment = .leading
        broadcast.translatesAutoresizingMaskIntoConstraints = false
        broadcast.accessibilityIdentifier = "thread-broadcast"
        broadcast.addAction(UIAction { [weak self] _ in
            guard let chat = self?.chat else { return }
            chat.threadBroadcast.toggle()
        }, for: .primaryActionTriggered)
        let broadcastRow = UIView()
        broadcastRow.addSubview(broadcast)
        // Notices, the broadcast toggle and the composer stack, so the preview
        // note takes the composer's place instead of leaving its space.
        let bottom = UIStackView(arrangedSubviews: [notices.view, broadcastRow, composer])
        bottom.axis = .vertical
        bottom.spacing = 4
        bottom.setCustomSpacing(-8, after: broadcastRow)
        bottom.translatesAutoresizingMaskIntoConstraints = false
        addChild(notices)
        view.addSubview(bottom)
        notices.didMove(toParent: self)
        self.broadcastRow = broadcastRow
        composer.textView.accessibilityIdentifier = "thread-composer"
        composer.model = model
        composer.changed = { [weak self] text in self?.chat.threadDraft = text }
        composer.submitted = { [weak self] in self?.send() }
        composer.attachTapped = { [weak self] in
            guard let self, let drawer = UIKitHosting.attachDrawer(chat: self.chat, inThread: true) else { return }
            self.present(drawer, animated: false)
        }
        composer.emojiTapped = { [weak self] in
            guard let self else { return }
            let composer = self.composer
            self.present(UIKitHosting.expressionPicker(chat: self.chat, inThread: true) { composer.insert($0) }, animated: true)
        }

        let host = UIHostingController(rootView: UIKitPresentationHost(state: presentations, model: model, inThread: true))
        host.view.backgroundColor = .clear
        host.view.isUserInteractionEnabled = false
        host.view.translatesAutoresizingMaskIntoConstraints = false
        addChild(host)
        view.insertSubview(host.view, at: 0)
        host.didMove(toParent: self)

        NSLayoutConstraint.activate([
            host.view.widthAnchor.constraint(equalToConstant: 1), host.view.heightAnchor.constraint(equalToConstant: 1),
            host.view.topAnchor.constraint(equalTo: view.topAnchor), host.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            header.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            header.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            header.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            header.heightAnchor.constraint(equalToConstant: 50),
            back.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 13),
            back.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            titles.leadingAnchor.constraint(equalTo: back.trailingAnchor, constant: 5),
            titles.trailingAnchor.constraint(lessThanOrEqualTo: header.trailingAnchor, constant: -18),
            titles.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            rule.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            rule.trailingAnchor.constraint(equalTo: header.trailingAnchor),
            rule.bottomAnchor.constraint(equalTo: header.bottomAnchor),
            rule.heightAnchor.constraint(equalToConstant: 1),
            collection.topAnchor.constraint(equalTo: header.bottomAnchor),
            collection.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            collection.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            status.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            status.centerYAnchor.constraint(equalTo: collection.centerYAnchor),
            status.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -48),
            bottom.topAnchor.constraint(equalTo: collection.bottomAnchor, constant: 4),
            bottom.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            bottom.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            bottom.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor, constant: -8),
            // As short as its contents allow: the timeline takes the slack.
            bottom.heightAnchor.constraint(equalToConstant: 0).withPriority(.defaultLow),
            broadcast.topAnchor.constraint(equalTo: broadcastRow.topAnchor),
            broadcast.bottomAnchor.constraint(equalTo: broadcastRow.bottomAnchor),
            broadcast.leadingAnchor.constraint(equalTo: broadcastRow.leadingAnchor, constant: 18),
            broadcast.trailingAnchor.constraint(equalTo: broadcastRow.trailingAnchor, constant: -18),
            broadcast.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
        ComposerLipView.attach(to: collection, in: view)
        // As SwiftUI's ThreadBackSwipe: a swipe right from anywhere follows
        // the finger back to the channel.
        let swipeBack = DirectionalSwipe(direction: 1)
        swipeBack.allowed = { [weak self] in self.map { !$0.closing && $0.presentedViewController == nil } ?? false }
        swipeBack.moved = { [weak self] distance in
            self?.view.endEditing(true)
            self?.view.transform = CGAffineTransform(translationX: distance, y: 0)
        }
        swipeBack.ended = { [weak self] complete, _ in
            guard let self else { return }
            guard complete else {
                UIView.animate(withDuration: UIKitStyle.reduceMotion ? 0 : UIKitStyle.motionFast) { self.view.transform = .identity }
                return
            }
            self.close(slid: true)
        }
        view.addGestureRecognizer(swipeBack)
        view.addInteraction(fileDrop.interaction)

        observer.start { [weak self] in self?.render() }
        composerObserver.start { [weak self] in self?.renderComposer() }
    }


    override func accessibilityPerformEscape() -> Bool { close(); return true }

    /// `slid`: a swipe already moved the thread aside; it leaves without the
    /// modal's slide down.
    private func close(slid: Bool = false) {
        guard !closing else { return }
        closing = true
        model.threadVisibilityChanged(rootID: nil, seq: nil)
        chat.closeThread()
        dismiss(animated: !slid && !UIKitStyle.reduceMotion) { [weak self] in self?.closed?() }
    }

    // MARK: Rendering

    private var direct: Bool { model.selectedDirectMessageID != nil }

    private func render() {
        guard let rootID = chat.threadRootID else {
            // The model closed it (another conversation, a denied thread).
            if !closing { closing = true; dismiss(animated: false) { [weak self] in self?.closed?() } }
            return
        }
        subtitle.text = direct ? "with \(chat.channelName)" : "in #\(chat.channelName)"
        let viewer = chat.currentAuthor?.id ?? model.account?.id
        let root = chat.displayedMessages.first { $0.id == rootID }
        let replies = chat.displayedThreadMessages.filter { $0.id != rootID }
        let pending = chat.pendingMessage.flatMap { $0.threadRootId == rootID ? $0 : nil }

        var items: [Item] = []
        var nextRows: [String: MessageRowState] = [:]
        var nextDays: [String: String] = [:]
        if let root, !root.isDeleted {
            nextRows[root.clientMessageId] = MessageRowState(message: root, grouped: false, continued: false, viewerID: viewer,
                                                             focused: chat.focusedMessageID == root.id, chat: chat, inThread: true)
            items.append(.row(root.clientMessageId))
            if let count = root.thread?.replyCount, count > 0 {
                nextDays["replies"] = "\(count) \(count == 1 ? "reply" : "replies")"
                items.append(.day("replies"))
            }
        }
        firstLoad = chat.threadLoading && replies.isEmpty
        // Go to message on a reply, as SwiftUI's thread: reveal it if its author
        // is blocked, stop following the newest replies, and centre it.
        let focusChanged = chat.focusRevision != lastFocusRevision
        let target = focusChanged ? chat.focusedMessageID.flatMap { id in replies.first { $0.id == id } } : nil
        // A reply still loading keeps the jump pending until it arrives.
        if target != nil || !chat.threadLoading { lastFocusRevision = chat.focusRevision }
        if let target { revealed.insert(target.id) }
        if firstLoad || chat.threadError != nil || chat.threadHasMore { items.append(.edge) }
        let built = TimelineRows.build(replies, pending: pending, author: chat.currentAuthor, viewerID: viewer,
                                       focusedID: chat.focusedMessageID, chat: chat, error: chat.error,
                                       rejected: chat.sendRejected, inThread: true,
                                       blocked: chat.blockedAuthorIDs, revealed: revealed)
        items += built.items.filter { if case .day = $0 { false } else { true } }
        nextRows.merge(built.rows) { _, new in new }
        nextDays.merge(built.days) { _, new in new }

        if replies.isEmpty && !chat.threadLoading && chat.threadError == nil {
            status.text = chat.isPreview ? "No replies yet." : "No replies yet. Start the thread."
        } else { status.text = nil }
        status.isHidden = status.text == nil

        let switched = rootID != shownRoot
        let survivors = Set(dataSource.snapshot().itemIdentifiers)
        var snapshot = NSDiffableDataSourceSnapshot<Int, Item>()
        snapshot.appendSections([0])
        snapshot.appendItems(items)
        let changed = items.filter { item in
            guard survivors.contains(item) else { return false }
            switch item {
            case let .row(id): return rows[id] != nextRows[id]
            case let .day(key): return days[key] != nextDays[key]
            case let .blocked(key): return runs[key] != built.runs[key]
            case .edge: return true
            case .newMessages: return false
            }
        }
        let reload = changed.filter { if case let .row(id) = $0 { nextRows[id]?.remeasures(from: rows[id]) == true } else { false } }
        snapshot.reloadItems(reload)
        snapshot.reconfigureItems(changed.filter { !reload.contains($0) })
        rows = nextRows
        days = nextDays
        runs = built.runs
        shownRoot = rootID
        if switched { followingTail = true }
        if target != nil { followingTail = false }
        let follow = followingTail
        let targetItem = target.map { Item.row($0.clientMessageId) }
        dataSource.apply(snapshot, animatingDifferences: false) { [weak self] in
            guard let self else { return }
            if let targetItem, let path = self.dataSource.indexPath(for: targetItem) {
                self.collection.layoutIfNeeded()
                self.collection.scrollToItem(at: path, at: .centeredVertically, animated: false)
                return
            }
            guard follow else { return }
            self.collection.layoutIfNeeded()
            let bottom = max(-self.collection.adjustedContentInset.top,
                             self.collection.contentSize.height + self.collection.adjustedContentInset.bottom - self.collection.bounds.height)
            self.collection.setContentOffset(CGPoint(x: 0, y: bottom), animated: false)
        }
        // Marks the thread read; only when what's visible changes, never per render.
        let visible = "\(rootID):\(replies.last?.seq ?? "")"
        if visible != lastVisible {
            lastVisible = visible
            let model = model, seq = replies.last?.seq
            DispatchQueue.main.async { model.threadVisibilityChanged(rootID: rootID, seq: seq) }
        }
    }

    private func renderComposer() {
        // Reply in thread, or Reply while the thread is open, raises the keyboard.
        composer.claimFocus(chat, inThread: true)
        let ready = chat.threadRootID != nil && chat.canInteract && !chat.isPreview
        composer.configure(prompt: "Reply to thread…", enabled: ready, canSend: chat.canPressSend(inThread: true), sending: chat.sending,
                           canAttach: chat.canAttach,
                           attachReason: chat.canAttach ? AttachCapacity.hint(chat.threadAttachmentDrafts.count) : chat.attachUnavailableReason,
                           canPickExpressions: ready)
        composer.show(draft: chat.threadDraft, conversation: "\(chat.editingContext):\(chat.threadRootID ?? "")")
        composer.refreshContext()
        composer.textView.pasteAttachments = chat.canAttach ? { [weak chat] in chat?.pasteAttachments(inThread: true) ?? false } : nil
        composer.showAccessories(ready && UIKitComposerAccessories.shows(chat: chat, inThread: true)
                                 ? UIKitComposerAccessories(chat: chat, inThread: true, present: { [weak self] in
                                     if self?.presentedViewController == nil { self?.present($0, animated: false) }
                                 }) : nil)
        composer.showVoice(button: ready ? voice.button : nil, bar: voice.bar, recording: ready && voice.isRecording)

        let on = chat.threadBroadcast
        let target = direct ? chat.channelName : "#\(chat.channelName)"
        let text = NSMutableAttributedString(string: "Also send to ", attributes: [.font: UIKitStyle.font(11), .foregroundColor: UIKitStyle.muted])
        text.append(NSAttributedString(string: target, attributes: [.font: UIKitStyle.font(11, .bold), .foregroundColor: UIKitStyle.muted]))
        broadcast.configuration?.attributedTitle = AttributedString(text)
        broadcast.configuration?.image = UIImage(systemName: on ? "checkmark.square.fill" : "square",
                                                withConfiguration: UIImage.SymbolConfiguration(pointSize: 18))
        broadcast.configuration?.baseForegroundColor = on ? UIKitStyle.terracottaBright : UIKitStyle.muted
        broadcastRow?.isHidden = !ready
        // A preview shows its note instead of a composer, as SwiftUI.
        composer.isHidden = chat.isPreview
        broadcast.isEnabled = chat.pendingMessage == nil
        broadcast.accessibilityLabel = "Also send to \(target)"
        broadcast.accessibilityValue = on ? "1" : "0"
        // A toggle, as SwiftUI's; set traits replace UIKit's automatic disabled state.
        broadcast.accessibilityTraits = broadcast.isEnabled ? [.button, .toggleButton] : [.button, .toggleButton, .notEnabled]
    }

    private func send() {
        guard chat.canPressSend(inThread: true) else { return }
        followingTail = true
        let chat = chat
        Task { await chat.send(inThread: true) }
    }

    private func openActions(_ id: String, picker: Bool) {
        guard let message = chat.messages.first(where: { $0.id == id }) else { return }
        view.endEditing(true)
        if picker {
            let chat = chat
            present(UIKitHosting.reactionPicker { emoji in
                Task { await chat.setReaction(messageID: id, emoji: emoji, active: true) }
            }, animated: true)
        } else {
            presentations.actions = message
        }
    }

    // MARK: Scrolling

    func scrollViewDidScroll(_ scrollView: UIScrollView) {
        guard scrollView.isTracking || scrollView.isDecelerating else { return }
        let bottom = scrollView.contentSize.height + scrollView.adjustedContentInset.bottom - scrollView.bounds.height
        followingTail = scrollView.contentOffset.y >= bottom - 48
        if scrollView.contentOffset.y + scrollView.adjustedContentInset.top < 400, chat.threadHasMore, !chat.threadLoading {
            let chat = chat
            Task { await chat.loadThread(older: true) }
        }
        if scrollView.contentOffset.y > bottom - 400, chat.threadHasNewer, !chat.threadLoading {
            let chat = chat
            Task { await chat.loadThread(newer: true) }
        }
    }
}
