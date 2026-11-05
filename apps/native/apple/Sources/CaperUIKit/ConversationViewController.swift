import CaperCore
import UIKit

/// The open channel: header, timeline and composer. The timeline is a
/// UICollectionView with a diffable data source and self-sizing cells, so only
/// realized rows are measured; the rest keep estimates until scrolled to.
final class ConversationViewController: UIViewController, UICollectionViewDelegate {
    enum Item: Hashable {
        case edge
        case day(String)
        case row(String)

        var rowID: String? { if case let .row(id) = self { return id } else { return nil } }
        var isRow: Bool { rowID != nil }
    }

    private let model: AppModel
    /// The last session's conversation, shown until the live one is ready.
    var cached: StartupSnapshot? { didSet { if isViewLoaded { render() } } }
    var browse: (() -> Void)?

    private let header = UIView()
    private let channelTitle = UIKitStyle.label(size: 17, weight: .bold)
    private let subtitle = UIKitStyle.label(size: 12, color: UIKitStyle.muted)
    private lazy var back = IconButton(icon: "arrow-left", label: "Back to Browse", size: 22) { [weak self] in self?.browse?() }
    private let status = UIKitStyle.label(size: 12, color: UIKitStyle.muted)
    private let typing = UIKitStyle.label(size: 11, weight: .medium, color: UIKitStyle.muted)
    private let composer = ComposerView()
    private var collection: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, Item>!

    private var rows: [String: MessageRowState] = [:]
    private var days: [String: String] = [:]
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
        timelineObserver.start { [weak self] in self?.render() }
        registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (self: Self, _) in
            // Message text is sized from the reader's text size; re-render every row.
            var snapshot = self.dataSource.snapshot()
            snapshot.reconfigureItems(snapshot.itemIdentifiers)
            self.dataSource.apply(snapshot, animatingDifferences: false)
        }
        composerObserver.start { [weak self] in self?.renderComposer() }
    }

    // MARK: Layout

    private func buildHeader() {
        header.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(header)
        let titles = UIStackView(arrangedSubviews: [channelTitle, subtitle])
        titles.axis = .vertical
        titles.spacing = 0
        titles.translatesAutoresizingMaskIntoConstraints = false
        titles.isAccessibilityElement = true
        titles.accessibilityTraits = .header
        titles.accessibilityIdentifier = "conversation-title"
        self.titles = titles
        back.tintColor = UIKitStyle.text
        back.accessibilityIdentifier = "back-to-browse"
        let rule = UIView()
        rule.backgroundColor = UIKitStyle.border
        rule.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(back)
        header.addSubview(titles)
        header.addSubview(rule)
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            header.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            header.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            header.heightAnchor.constraint(equalToConstant: 52).withPriority(.defaultHigh),
            back.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 13),
            back.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            titles.leadingAnchor.constraint(equalTo: back.trailingAnchor, constant: 8),
            titles.trailingAnchor.constraint(lessThanOrEqualTo: header.trailingAnchor, constant: -18),
            titles.topAnchor.constraint(greaterThanOrEqualTo: header.topAnchor, constant: 4),
            titles.bottomAnchor.constraint(lessThanOrEqualTo: header.bottomAnchor, constant: -4),
            titles.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            rule.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            rule.trailingAnchor.constraint(equalTo: header.trailingAnchor),
            rule.bottomAnchor.constraint(equalTo: header.bottomAnchor),
            rule.heightAnchor.constraint(equalToConstant: 1),
        ])
    }
    private var titles: UIStackView!

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
            case let .row(id):
                let cell = collection.dequeueReusableCell(withReuseIdentifier: MessageCell.reuse, for: indexPath) as! MessageCell
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
        status.textAlignment = .center
        status.numberOfLines = 0
        view.addSubview(status)
        typing.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(typing)
        NSLayoutConstraint.activate([
            collection.topAnchor.constraint(equalTo: header.bottomAnchor),
            collection.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            collection.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            status.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            status.centerYAnchor.constraint(equalTo: collection.centerYAnchor),
            status.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -48),
            typing.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 18),
            typing.trailingAnchor.constraint(lessThanOrEqualTo: view.trailingAnchor, constant: -18),
            typing.topAnchor.constraint(equalTo: collection.bottomAnchor),
        ])
    }

    private func buildComposer() {
        view.addSubview(composer)
        composer.changed = { [weak self] text in
            guard let chat = self?.model.chat else { return }
            chat.draft = text
            chat.draftChanged(text)
        }
        composer.submitted = { [weak self] in self?.send() }
        NSLayoutConstraint.activate([
            composer.topAnchor.constraint(equalTo: typing.bottomAnchor),
            composer.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            composer.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            composer.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor, constant: -8),
        ])
    }

    // MARK: Rendering

    /// Live once the model has opened this channel; until then, the cache.
    private var isLive: Bool {
        model.phase == .ready && !model.restoringWorkspace && model.selectedChannelID != nil
            && !(model.chat.loading && model.chat.messages.isEmpty && cached?.messages.isEmpty == false)
    }

    private func render() {
        let chat = model.chat
        let live = isLive
        let channelID = live ? model.selectedChannelID : cached?.channelID
        let channelName = live ? (model.selectedChannel?.name ?? chat.channelName) : (cached?.channelName ?? "")
        channelTitle.text = channelName.isEmpty ? "" : "# \(channelName)"
        subtitle.text = memberSummary(live: live, channelID: channelID)
        titles.accessibilityLabel = [channelName.isEmpty ? nil : "\(channelName) channel", subtitle.text].compactMap { $0 }.joined(separator: ", ")

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
        if typing.text != typingText {
            typing.text = typingText
            typing.accessibilityLabel = typingText
        }
        if live, let error = chat.error, messages.isEmpty { status.text = error }
        else if live, chat.loading && messages.isEmpty { status.text = "Loading messages…" }
        else { status.text = nil }
        status.isHidden = status.text == nil

        apply(messages: messages, pending: pending, author: chat.currentAuthor, viewerID: viewer,
              key: channelID, live: live, error: chat.error, rejected: chat.sendRejected)
    }

    private func apply(messages: [ChatMessage], pending: PendingMessage?, author: ChatAuthor?, viewerID: String?,
                       key: String?, live: Bool, error: String?, rejected: Bool) {
        var items: [Item] = []
        var nextRows: [String: MessageRowState] = [:]
        var nextDays: [String: String] = [:]
        if edge != nil { items.append(.edge) }
        var previous: ChatMessage?
        for (index, message) in messages.enumerated() {
            if previous.map({ !NativePresentation.sameDay($0.createdAt, message.createdAt) }) ?? true,
               let label = NativePresentation.dayLabel(message.createdAt) {
                let key = "day-\(message.clientMessageId)"
                nextDays[key] = label
                items.append(.day(key))
                previous = nil
            }
            let next = index + 1 < messages.count ? messages[index + 1] : nil
            let grouped = NativePresentation.grouped(message, after: previous)
            let continued = next.map { NativePresentation.sameDay(message.createdAt, $0.createdAt) && NativePresentation.grouped($0, after: message) } ?? false
            nextRows[message.clientMessageId] = MessageRowState(message: message, grouped: grouped, continued: continued, viewerID: viewerID)
            items.append(.row(message.clientMessageId))
            previous = message
        }
        if let pending, nextRows[pending.id] == nil {
            let groupedPending = previous.map { $0.author.id == author?.id && NativePresentation.sameDay($0.createdAt, pending.createdAt) } ?? false
            if let last = previous, groupedPending { nextRows[last.clientMessageId]?.continued = true }
            nextRows[pending.id] = MessageRowState(pending: pending, author: author, grouped: groupedPending,
                                                   error: error, rejected: rejected)
            items.append(.row(pending.id))
        }

        let switched = key != shownKey
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
            case .edge: return true
            }
        }
        snapshot.reconfigureItems(changed)
        rows = nextRows
        days = nextDays
        shownKey = key
        self.live = live
        arriving.formUnion(appended)

        dataSource.apply(snapshot, animatingDifferences: false) { [weak self] in
            guard let self else { return }
            if atTail && !(prepended && anchor != nil) {
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
        followingTail = scrollView.contentOffset.y >= bottom - 48
        if scrollView.contentOffset.y + scrollView.adjustedContentInset.top < 800 { loadOlder(force: false) }
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

    private func renderComposer() {
        let chat = model.chat
        let name = model.selectedChannel?.name ?? cached?.channelName ?? chat.channelName
        let ready = isLive && chat.canInteract && !model.previewingChannel
        composer.configure(channelName: name, enabled: ready, canSend: chat.canPressSend(inThread: false)
            && !chat.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        composer.show(draft: chat.draft)
    }

    private func send() {
        let chat = model.chat
        guard isLive, chat.canPressSend(inThread: false) else { return }
        followingTail = true
        Task { await chat.send() }
    }

    // MARK: Text

    private func memberSummary(live: Bool, channelID: String?) -> String? {
        guard let detail = live ? model.detail : cached?.detail, let channelID,
              let channel = detail.channels.first(where: { $0.id == channelID }) else { return nil }
        let count = channel.private ? (detail.channelMembers[channelID]?.count ?? 0) : detail.members.count
        return count == 1 ? "1 member" : "\(count) members"
    }

    static func typingLabel(_ names: [String]) -> String {
        switch names.count {
        case 1: return "\(names[0]) is typing…"
        case 2: return "\(names[0]) and \(names[1]) are typing…"
        default: return "Several people are typing…"
        }
    }
}
