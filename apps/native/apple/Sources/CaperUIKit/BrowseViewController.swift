import CaperCore
import UIKit

/// Browse on iPhone: the space rail beside the selected space's channels, with
/// the account bar beneath. The conversation slides over it.
final class BrowseViewController: UIViewController, UICollectionViewDelegate {
    private enum Item: Hashable { case channel(String) }

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
        dataSource = UICollectionViewDiffableDataSource(collectionView: channels) { collection, path, item in
            collection.dequeueConfiguredReusableCell(using: registration, for: path, item: item)
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
        dataSource.apply(snapshot, animatingDifferences: false)

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
        guard case let .channel(id) = dataSource.itemIdentifier(for: indexPath) else { return }
        if live, let channel = model.detail?.channels.first(where: { $0.id == id }) {
            Task { await model.select(channel: channel) }
        }
        opened?()
    }
}
