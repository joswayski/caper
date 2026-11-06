import CaperCore
import SwiftUI
import UIKit

/// Browse on iPhone: the space rail beside the selected space's channels and
/// direct messages, with the account bar beneath. The conversation slides over it.
final class BrowseViewController: UIViewController, UICollectionViewDelegate {
    private enum Item: Hashable { case channel(String), directHeading, requests, selfDirect, direct(String), newMessage }

    private let model: AppModel
    var cached: StartupSnapshot? { didSet { if isViewLoaded { render() } } }
    /// A channel was chosen: show the conversation.
    var opened: (() -> Void)?

    private let rail = UIStackView()
    private let railScroll = UIScrollView()
    private let card = UIView()
    private let spaceName = UIKitStyle.label(size: 17, weight: .bold)
    private let channelsHeading = UIKitStyle.label(size: 12, weight: .bold, color: UIKitStyle.muted)
    private var channels: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, Item>!
    private var channelRows: [String: Channel] = [:]
    private var selectedChannelID: String?
    private var unread: Set<String> = []
    private var directRows: [String: DirectMessageConversation] = [:]
    /// Message requests, pushed over the lists as SwiftUI does on iPhone.
    private lazy var requests: UIHostingController<UIKitMessageRequests> = {
        let host = UIHostingController(rootView: UIKitMessageRequests(model: model))
        host.view.backgroundColor = UIKitStyle.sidebar
        host.view.translatesAutoresizingMaskIntoConstraints = false
        host.view.accessibilityIdentifier = "message-requests-page"
        return host
    }()
    private let accountAvatar = AvatarView(side: 32)
    private let accountName = UIKitStyle.label(size: 14, weight: .bold)
    private lazy var accountMenu: UIButton = {
        let button = UIButton(type: .system)
        button.setImage(UIKitStyle.icon("settings")?.resized(to: 20), for: .normal)
        button.tintColor = UIKitStyle.text
        button.accessibilityLabel = "Account settings"
        button.accessibilityIdentifier = "account-settings"
        button.showsMenuAsPrimaryAction = true
        button.menu = UIMenu(children: [UIAction(title: "Sign out", image: UIKitStyle.icon("log-out"), attributes: .destructive) { [weak self] _ in
            guard let model = self?.model else { return }
            Task { await model.logout() }
        }])
        button.translatesAutoresizingMaskIntoConstraints = false
        return button
    }()
    private let observer = ModelObserver()

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
        spaceName.accessibilityTraits = .header
        spaceName.accessibilityIdentifier = "selected-space-name"
        channelsHeading.accessibilityTraits = .header

        var list = UICollectionLayoutListConfiguration(appearance: .plain)
        list.backgroundColor = .clear
        list.showsSeparators = false
        channels = UICollectionView(frame: .zero, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: list))
        channels.backgroundColor = .clear
        channels.delegate = self
        channels.translatesAutoresizingMaskIntoConstraints = false
        channels.accessibilityIdentifier = "channel-list"
        let registration = UICollectionView.CellRegistration<UICollectionViewListCell, Item> { [weak self] cell, _, item in
            guard let self, case let .channel(id) = item, let channel = self.channelRows[id] else { return }
            self.configure(cell, channel: channel)
        }
        let directRegistration = UICollectionView.CellRegistration<DirectRowCell, Item> { [weak self] cell, _, item in
            self?.configure(cell, item: item)
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: channels) { collection, path, item in
            if case .channel = item { return collection.dequeueConfiguredReusableCell(using: registration, for: path, item: item) }
            return collection.dequeueConfiguredReusableCell(using: directRegistration, for: path, item: item)
        }
        [spaceName, channelsHeading, channels].forEach(card.addSubview)
        NSLayoutConstraint.activate([
            card.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 8),
            card.leadingAnchor.constraint(equalTo: railScroll.trailingAnchor),
            card.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -8),
            railScroll.bottomAnchor.constraint(equalTo: card.bottomAnchor),
            spaceName.topAnchor.constraint(equalTo: card.topAnchor, constant: 18),
            spaceName.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 16),
            spaceName.trailingAnchor.constraint(lessThanOrEqualTo: card.trailingAnchor, constant: -16),
            channelsHeading.topAnchor.constraint(equalTo: spaceName.bottomAnchor, constant: 18),
            channelsHeading.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 16),
            channels.topAnchor.constraint(equalTo: channelsHeading.bottomAnchor, constant: 6),
            channels.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            channels.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            channels.bottomAnchor.constraint(equalTo: card.bottomAnchor),
        ])
    }

    private func buildAccountBar() {
        let bar = UIView()
        bar.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(bar)
        accountName.lineBreakMode = .byTruncatingTail
        let identity = UIStackView(arrangedSubviews: [accountAvatar, accountName])
        identity.spacing = 10
        identity.alignment = .center
        identity.translatesAutoresizingMaskIntoConstraints = false
        identity.isAccessibilityElement = true
        identity.accessibilityIdentifier = "account-identity"
        self.identity = identity
        bar.addSubview(identity)
        bar.addSubview(accountMenu)
        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: card.bottomAnchor),
            bar.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 12),
            bar.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            bar.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -8),
            bar.heightAnchor.constraint(equalToConstant: 52 + 16),
            identity.leadingAnchor.constraint(equalTo: bar.leadingAnchor, constant: 6),
            identity.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
            identity.trailingAnchor.constraint(lessThanOrEqualTo: accountMenu.leadingAnchor, constant: -8),
            accountMenu.trailingAnchor.constraint(equalTo: bar.trailingAnchor),
            accountMenu.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
            accountMenu.widthAnchor.constraint(equalToConstant: 44),
            accountMenu.heightAnchor.constraint(equalToConstant: 44),
        ])
    }
    private var identity: UIStackView!

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
        let joined = detail?.channels.filter(\.joined) ?? []
        channelsHeading.text = "CHANNELS  \(joined.count)"
        channelsHeading.accessibilityLabel = "Channels, \(joined.count)"
        let previous = channelRows
        channelRows = Dictionary(uniqueKeysWithValues: joined.map { ($0.id, $0) })
        var snapshot = NSDiffableDataSourceSnapshot<Int, Item>()
        snapshot.appendSections([0])
        snapshot.appendItems(joined.map { .channel($0.id) })
        // Selection and unread state live in the cell; reconfigure all visible rows cheaply.
        snapshot.reconfigureItems(joined.filter { previous[$0.id] != nil }.map { .channel($0.id) })
        appendDirect(to: &snapshot, live: live)
        dataSource.apply(snapshot, animatingDifferences: false)
        renderRequests(live && model.account != nil && model.showingMessageRequests)

        let account = model.account
        let name = account?.displayName ?? account?.username ?? ""
        accountName.text = name
        accountAvatar.show(name: name, avatarID: account?.avatarId, picture: account?.avatarPicture)
        identity.accessibilityLabel = name.isEmpty ? nil : "Signed in as \(name)"
    }

    private var railKey: [String] = []

    private func renderRail(_ spaces: [Space], selected: String?, unreadSpaces: Set<String>) {
        let key = spaces.map { "\($0.id)|\($0.name)|\($0.id == selected)|\(unreadSpaces.contains($0.id))" }
        guard key != railKey else { return }
        railKey = key
        rail.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for space in spaces {
            let name = space.demo == true ? "Caper" : space.name
            let tile = UIButton(type: .custom)
            tile.translatesAutoresizingMaskIntoConstraints = false
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
    }

    // MARK: Direct messages

    /// Direct messages under the channels: requests, your notes, each DM, New message.
    private func appendDirect(to snapshot: inout NSDiffableDataSourceSnapshot<Int, Item>, live: Bool) {
        guard live, let account = model.account else { directRows = [:]; return }
        let previous = directRows
        let others = model.visibleDirectMessages.filter { $0.peer.id != account.id }
        directRows = Dictionary(model.directMessages.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var items: [Item] = [.directHeading]
        if !model.messageRequests.isEmpty { items.append(.requests) }
        items.append(.selfDirect)
        items += others.map { .direct($0.id) }
        items.append(.newMessage)
        snapshot.appendSections([1])
        snapshot.appendItems(items, toSection: 1)
        // Unread, selection and mute live in the cells; reconfigure the survivors.
        snapshot.reconfigureItems(items.filter {
            if case let .direct(id) = $0 { return previous[id] != nil }
            return dataSource.snapshot().indexOfItem($0) != nil
        })
    }

    private func renderRequests(_ showing: Bool) {
        if showing, requests.parent == nil {
            addChild(requests)
            card.addSubview(requests.view)
            NSLayoutConstraint.activate([
                requests.view.topAnchor.constraint(equalTo: card.topAnchor, constant: 8),
                requests.view.leadingAnchor.constraint(equalTo: card.leadingAnchor),
                requests.view.trailingAnchor.constraint(equalTo: card.trailingAnchor),
                requests.view.bottomAnchor.constraint(equalTo: card.bottomAnchor),
            ])
            requests.didMove(toParent: self)
            UIAccessibility.post(notification: .screenChanged, argument: requests.view)
        } else if !showing, requests.parent != nil {
            requests.willMove(toParent: nil)
            requests.view.removeFromSuperview()
            requests.removeFromParent()
            UIAccessibility.post(notification: .screenChanged, argument: channels)
        }
        [spaceName, channelsHeading, channels].forEach { $0.accessibilityElementsHidden = showing }
    }

    private func configure(_ cell: DirectRowCell, item: Item) {
        switch item {
        case .directHeading:
            cell.heading("DIRECT MESSAGES") { [weak self] in self?.newDirectMessage() }
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
        case .channel:
            break
        }
        if case .directHeading = item { return }
        cell.isAccessibilityElement = true
        cell.accessibilityTraits = cell.accessibilityValue?.hasPrefix("Selected") == true ? [.button, .selected] : .button
    }

    private func newDirectMessage() {
        guard live else { return }
        present(UIKitHosting.newDirectMessage(model: model), animated: true)
    }

    /// Press and hold a DM: notifications on or off, and mute, as on SwiftUI.
    func collectionView(_ collectionView: UICollectionView, contextMenuConfigurationForItemsAt indexPaths: [IndexPath],
                        point: CGPoint) -> UIContextMenuConfiguration? {
        guard live, let path = indexPaths.first, case let .direct(id) = dataSource.itemIdentifier(for: path) else { return nil }
        model.refreshNotificationSettingsIfStale()
        return UIContextMenuConfiguration(actionProvider: { [weak self] _ in
            guard let self else { return nil }
            let scope = NotificationScope.direct(id)
            let model = self.model
            let saving = model.notificationSettings == nil || model.notificationScopesSaving.contains(scope)
            let off = model.notificationSettings?.notificationsOff(scope) ?? false
            let level = UIAction(title: NotificationLabels.directNotificationsTitle(off: off),
                                 attributes: saving ? .disabled : []) { _ in
                Task { await model.setNotificationLevel(off ? nil : .nothing, for: scope) }
            }
            let mute: UIMenuElement
            if let current = model.notificationMute(scope) {
                mute = UIMenu(options: .displayInline, children: [
                    UIAction(title: NotificationLabels.mutedStatus(current), attributes: .disabled) { _ in },
                    UIAction(title: NotificationLabels.muteTitle(for: scope, muted: true),
                             attributes: model.notificationScopesSaving.contains(scope) ? .disabled : []) { _ in
                        Task { await model.setMute(nil, for: scope) }
                    },
                ])
            } else {
                mute = UIMenu(title: NotificationLabels.muteTitle(for: scope, muted: false), options: saving ? .init() : [],
                              children: MutePreset.allCases.map { preset in
                                  UIAction(title: preset.title, attributes: saving ? .disabled : []) { _ in
                                      Task { await model.setMute(preset.until(from: Date()), for: scope) }
                                  }
                              })
            }
            return UIMenu(children: [level, mute])
        })
    }

    private func configure(_ cell: UICollectionViewListCell, channel: Channel) {
        let selected = channel.id == selectedChannelID
        let unread = self.unread.contains(channel.id)
        var content = cell.defaultContentConfiguration()
        content.text = channel.name
        content.textProperties.font = UIKitStyle.font(15, unread || selected ? .bold : .medium)
        content.textProperties.color = selected || unread ? UIKitStyle.text : UIKitStyle.muted
        content.image = UIKitStyle.icon(channel.private ? "lock" : "hash")?.resized(to: 16)
        content.imageProperties.tintColor = selected ? UIKitStyle.text : UIKitStyle.muted
        content.imageToTextPadding = 8
        content.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 10, leading: 8, bottom: 10, trailing: 8)
        cell.contentConfiguration = content
        var background = UIBackgroundConfiguration.listPlainCell()
        background.backgroundColor = selected ? UIKitStyle.text.withAlphaComponent(0.08) : .clear
        background.cornerRadius = 8
        background.backgroundInsets = NSDirectionalEdgeInsets(top: 1, leading: 8, bottom: 1, trailing: 8)
        cell.backgroundConfiguration = background
        cell.accessibilityLabel = "\(channel.name)\(channel.private ? ", private" : "") channel"
        cell.accessibilityValue = [selected ? "Selected" : "", unread ? "Unread activity" : ""].filter { !$0.isEmpty }.joined(separator: ", ")
        cell.accessibilityTraits = selected ? [.button, .selected] : .button
        cell.accessibilityIdentifier = "channel-\(channel.name)"
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: false)
        switch dataSource.itemIdentifier(for: indexPath) {
        case let .channel(id):
            if live, let channel = model.detail?.channels.first(where: { $0.id == id }) {
                Task { await model.select(channel: channel) }
            }
            opened?()
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
    private let avatar = AvatarView(side: 24)
    private let name = UIKitStyle.label(size: 15, weight: .medium)
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
        [icon, avatar, name, youTag, bell, spacer, count, dot].forEach(stack.addArrangedSubview)
        stack.spacing = 9
        stack.setCustomSpacing(6, after: name)
        stack.alignment = .center
        stack.isUserInteractionEnabled = false
        stack.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(stack)
        [icon, avatar, name, youTag, bell, count, dot].forEach { $0.isAccessibilityElement = false }
        NSLayoutConstraint.activate([
            icon.widthAnchor.constraint(equalToConstant: 24), icon.heightAnchor.constraint(equalToConstant: 24),
            count.heightAnchor.constraint(equalToConstant: 18), count.widthAnchor.constraint(greaterThanOrEqualToConstant: 20),
            dot.widthAnchor.constraint(equalToConstant: 8), dot.heightAnchor.constraint(equalToConstant: 8),
            stack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 17),
            stack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -17),
            stack.topAnchor.constraint(equalTo: contentView.topAnchor),
            stack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            contentView.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func reset() {
        [icon, avatar, youTag, bell, count, dot].forEach { $0.isHidden = true }
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
            self.avatar.isHidden = false
        }
        self.name.text = text
        self.name.font = UIKitStyle.font(15, bold ? .bold : .medium)
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
        background.backgroundInsets = NSDirectionalEdgeInsets(top: 1, leading: 8, bottom: 1, trailing: 8)
        backgroundConfiguration = background
    }
}

/// A label with horizontal padding, for counts in capsules.
final class PaddedLabel: UILabel {
    override var intrinsicContentSize: CGSize {
        let size = super.intrinsicContentSize
        return CGSize(width: size.width + 12, height: size.height)
    }
}
