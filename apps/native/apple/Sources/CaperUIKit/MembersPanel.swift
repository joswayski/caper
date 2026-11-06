import CaperCore
import UIKit

/// The member list on phones (SwiftUI's MembersOverlay and MemberPresenceView):
/// a rounded panel that slides over the conversation from the trailing edge,
/// below its header, over a dimmed conversation. The channel's readers lead;
/// each group splits into Online, Offline and Status unavailable.
final class MembersPanel: UIView, UICollectionViewDelegate, UISearchTextFieldDelegate {
    private enum Item: Hashable {
        case label(String, Int, primary: Bool, group: String)
        case member(String, group: String)
        case note(String)
    }

    private let model: AppModel
    private let dimming = UIView()
    private let panel = UIView()
    private let count = UIKitStyle.label(size: 10, weight: .bold, color: UIKitStyle.muted)
    private lazy var searchButton = IconButton(icon: "search", label: "Search members", size: 16) { [weak self] in self?.startSearch() }
    private lazy var closeButton = IconButton(icon: "x", label: "Close member list", size: 18) { [weak self] in self?.close() }
    private let search = UISearchTextField()
    private lazy var clearSearch = IconButton(icon: "x", label: "Clear and close member search", size: 16) { [weak self] in self?.endSearch() }
    private let searchRow = UIStackView()
    private var collection: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, Item>!
    private var members: [String: Member] = [:]
    private var quiet: Set<String> = []
    private let observer = ModelObserver()
    /// Opens a member's profile, when profiles are available.
    var openProfile: ((Member) -> Void)?
    var closed: (() -> Void)?
    private(set) var isOpen = false

    init(model: AppModel) {
        self.model = model
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        isHidden = true
        dimming.backgroundColor = UIColor.black.withAlphaComponent(0.25)
        dimming.translatesAutoresizingMaskIntoConstraints = false
        dimming.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(dismissTap)))
        dimming.isAccessibilityElement = false
        addSubview(dimming)

        panel.backgroundColor = UIKitStyle.sidebar
        panel.layer.cornerRadius = 16
        panel.layer.cornerCurve = .continuous
        panel.layer.borderWidth = 1
        panel.layer.borderColor = UIKitStyle.border.cgColor
        panel.layer.shadowColor = UIColor.black.cgColor
        panel.layer.shadowOpacity = 0.45
        panel.layer.shadowRadius = 16
        panel.layer.shadowOffset = CGSize(width: -12, height: 0)
        panel.translatesAutoresizingMaskIntoConstraints = false
        panel.accessibilityViewIsModal = true
        panel.accessibilityIdentifier = "members-panel"
        addSubview(panel)
        let swipe = UIPanGestureRecognizer(target: self, action: #selector(swiped(_:)))
        panel.addGestureRecognizer(swipe)

        let title = UIKitStyle.label("Members", size: 12, weight: .bold, color: UIKitStyle.muted)
        title.accessibilityTraits = .header
        searchButton.tintColor = UIKitStyle.muted
        closeButton.tintColor = UIKitStyle.muted
        let bar = UIStackView(arrangedSubviews: [title, count, UIView(), searchButton, closeButton])
        bar.spacing = 6
        bar.alignment = .center
        bar.translatesAutoresizingMaskIntoConstraints = false
        let rule = UIView()
        rule.backgroundColor = UIKitStyle.border
        rule.translatesAutoresizingMaskIntoConstraints = false

        search.placeholder = "Name or username"
        search.font = UIKitStyle.font(12)
        search.textColor = UIKitStyle.text
        search.keyboardAppearance = .dark
        search.autocorrectionType = .no
        search.autocapitalizationType = .none
        search.returnKeyType = .done
        search.delegate = self
        search.accessibilityLabel = "Search members"
        search.addAction(UIAction { [weak self] _ in self?.model.presence.query = self?.search.text ?? "" }, for: .editingChanged)
        clearSearch.tintColor = UIKitStyle.muted
        [search, clearSearch].forEach(searchRow.addArrangedSubview)
        searchRow.spacing = 4
        searchRow.alignment = .center
        searchRow.isHidden = true

        var list = UICollectionLayoutListConfiguration(appearance: .plain)
        list.showsSeparators = false
        list.backgroundColor = .clear
        collection = UICollectionView(frame: .zero, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: list))
        collection.backgroundColor = .clear
        collection.delegate = self
        collection.keyboardDismissMode = .onDrag
        collection.accessibilityIdentifier = "member-list"
        let column = UIStackView(arrangedSubviews: [searchRow, collection])
        column.axis = .vertical
        column.translatesAutoresizingMaskIntoConstraints = false
        [bar, rule, column].forEach(panel.addSubview)

        let labelCell = UICollectionView.CellRegistration<UICollectionViewListCell, Item> { cell, _, item in
            var content = UIListContentConfiguration.cell()
            var background = UIBackgroundConfiguration.clear()
            switch item {
            case let .label(title, count, primary, group):
                let text = NSMutableAttributedString(string: title + "  ", attributes: [.font: UIKitStyle.font(11, .bold)])
                text.append(NSAttributedString(string: "\(count)", attributes: [.font: UIKitStyle.font(10, .bold)]))
                content.attributedText = text
                content.textProperties.color = primary ? UIKitStyle.text : UIKitStyle.muted
                content.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 8, leading: 12, bottom: 2, trailing: 12)
                cell.accessibilityTraits = .header
                cell.accessibilityLabel = "\(title), \(count)"
                if group == "channel" { background.backgroundColor = UIKitStyle.sidebarRaised }
            case let .note(text):
                content.text = text
                content.textProperties.font = UIKitStyle.font(11)
                content.textProperties.color = UIKitStyle.muted
                content.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 8, leading: 12, bottom: 8, trailing: 12)
            case .member: break
            }
            cell.contentConfiguration = content
            cell.backgroundConfiguration = background
        }
        let memberCell = UICollectionView.CellRegistration<MemberCell, Item> { [weak self] cell, _, item in
            guard let self, case let .member(id, group) = item, let member = self.members[id] else { return }
            cell.configure(member, status: self.model.presence.status(for: member), quiet: self.quiet.contains(id),
                           raised: group == "channel", opens: self.openProfile != nil)
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collection) { collection, path, item in
            if case .member = item { return collection.dequeueConfiguredReusableCell(using: memberCell, for: path, item: item) }
            return collection.dequeueConfiguredReusableCell(using: labelCell, for: path, item: item)
        }

        NSLayoutConstraint.activate([
            dimming.topAnchor.constraint(equalTo: topAnchor),
            dimming.leadingAnchor.constraint(equalTo: leadingAnchor),
            dimming.trailingAnchor.constraint(equalTo: trailingAnchor),
            dimming.bottomAnchor.constraint(equalTo: bottomAnchor),
            panel.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            panel.bottomAnchor.constraint(equalTo: safeAreaLayoutGuide.bottomAnchor, constant: -8),
            panel.widthAnchor.constraint(equalToConstant: 280).withPriority(.defaultHigh),
            panel.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, constant: -64),
            panel.trailingAnchor.constraint(equalTo: safeAreaLayoutGuide.trailingAnchor, constant: -8),
            bar.topAnchor.constraint(equalTo: panel.topAnchor),
            bar.leadingAnchor.constraint(equalTo: panel.leadingAnchor, constant: 12),
            bar.trailingAnchor.constraint(equalTo: panel.trailingAnchor, constant: -4),
            bar.heightAnchor.constraint(equalToConstant: 50),
            rule.topAnchor.constraint(equalTo: bar.bottomAnchor),
            rule.leadingAnchor.constraint(equalTo: panel.leadingAnchor),
            rule.trailingAnchor.constraint(equalTo: panel.trailingAnchor),
            rule.heightAnchor.constraint(equalToConstant: 1),
            column.topAnchor.constraint(equalTo: rule.bottomAnchor),
            column.leadingAnchor.constraint(equalTo: panel.leadingAnchor, constant: 6),
            column.trailingAnchor.constraint(equalTo: panel.trailingAnchor, constant: -6),
            column.bottomAnchor.constraint(equalTo: panel.bottomAnchor, constant: -6),
            search.heightAnchor.constraint(equalToConstant: 36),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    // MARK: Presentation

    func open() {
        guard !isOpen else { return }
        isOpen = true
        isHidden = false
        observer.start { [weak self] in self?.render() }
        layoutIfNeeded()
        if UIKitStyle.reduceMotion {
            dimming.alpha = 1
            panel.transform = .identity
        } else {
            // Arrives from past the trailing edge, so its shadow travels with it.
            dimming.alpha = 0
            panel.transform = CGAffineTransform(translationX: panel.bounds.width + 24, y: 0)
            let animator = UIViewPropertyAnimator(duration: 0.24, timingParameters: UIKitStyle.easeOut)
            animator.addAnimations { self.dimming.alpha = 1; self.panel.transform = .identity }
            animator.startAnimation()
        }
        UIAccessibility.post(notification: .screenChanged, argument: panel)
    }

    /// A swipe left on the timeline: the panel follows the finger in.
    func follow(_ distance: CGFloat) {
        guard !isOpen else { return }
        if isHidden {
            isHidden = false
            observer.start { [weak self] in self?.render() }
            layoutIfNeeded()
        }
        let travel = panel.bounds.width + 24
        let shown = min(travel, distance)
        panel.transform = CGAffineTransform(translationX: travel - shown, y: 0)
        dimming.alpha = shown / max(1, travel)
    }

    /// The swipe lifted: it settles open, or (closing being instant) hides.
    func release(opening: Bool) {
        guard !isOpen, !isHidden else { return }
        guard opening else {
            isHidden = true
            observer.stop()
            return
        }
        isOpen = true
        let settle = { self.dimming.alpha = 1; self.panel.transform = .identity }
        if UIKitStyle.reduceMotion { settle() } else {
            let animator = UIViewPropertyAnimator(duration: 0.2, timingParameters: UIKitStyle.easeOut)
            animator.addAnimations(settle)
            animator.startAnimation()
        }
        UIAccessibility.post(notification: .screenChanged, argument: panel)
    }

    /// Closing is instant, as everywhere in Caper.
    func close() {
        guard isOpen else { return }
        isOpen = false
        isHidden = true
        observer.stop()
        endSearch()
        closed?()
    }

    @objc private func dismissTap() { close() }

    @objc private func swiped(_ pan: UIPanGestureRecognizer) {
        let x = max(0, pan.translation(in: self).x)
        switch pan.state {
        case .changed: panel.transform = CGAffineTransform(translationX: x, y: 0)
        case .ended, .cancelled:
            if x > 80 || pan.velocity(in: self).x > 600 { panel.transform = .identity; close() }
            else { UIView.animate(withDuration: UIKitStyle.motionFast) { self.panel.transform = .identity } }
        default: break
        }
    }

    // MARK: Search

    private func startSearch() {
        searchRow.isHidden = false
        searchButton.isHidden = true
        search.becomeFirstResponder()
    }

    private func endSearch() {
        search.text = ""
        model.presence.query = ""
        searchRow.isHidden = true
        searchButton.isHidden = false
        search.resignFirstResponder()
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        textField.resignFirstResponder()
        return true
    }

    // MARK: Content

    private func render() {
        let presence = model.presence
        let detail = model.detail
        count.text = detail.map { "\($0.members.count)" }
        var items: [Item] = []
        var shown: [String: Member] = [:]
        var quietIDs: Set<String> = []
        if detail?.members.isEmpty != false {
            items = [.note("No members to show.")]
        } else if presence.loading {
            items = [.note("Loading members…")]
        } else {
            let visible = presence.visibleMembers.map { model.profiled($0) }
            if visible.isEmpty { items.append(.note("No matching members.")) }
            func sections(_ members: [Member], group: String, quiet: Bool) {
                for section in [PresenceStatus.online, .offline, .unknown] {
                    let rows = members.filter { member in
                        let status = presence.status(for: member)
                        return section == .online ? status == .online || status == .idle : status == section
                    }
                    guard !rows.isEmpty else { continue }
                    let title = section == .online ? "Online" : section == .offline ? "Offline" : "Status unavailable"
                    items.append(.label(title, rows.count, primary: false, group: group))
                    for member in rows {
                        shown[member.id] = member
                        if quiet || section == .offline { quietIDs.insert(member.id) }
                        items.append(.member(member.id, group: group))
                    }
                }
            }
            if let groups = detail?.memberGroups(channelID: model.selectedChannelID) {
                let ids = Set(groups.inChannel.map(\.id))
                let top = visible.filter { ids.contains($0.id) }
                let rest = visible.filter { !ids.contains($0.id) }
                let name = detail?.channels.first { $0.id == model.selectedChannelID }?.name ?? ""
                if !top.isEmpty {
                    items.append(.label("In #\(name)", top.count, primary: true, group: "channel"))
                    sections(top, group: "channel", quiet: false)
                }
                if !rest.isEmpty {
                    items.append(.label("Others in this space", rest.count, primary: false, group: "others"))
                    sections(rest, group: "others", quiet: true)
                }
            } else {
                sections(visible, group: "space", quiet: false)
            }
        }
        let changed = shown != members || quietIDs != quiet
        members = shown
        quiet = quietIDs
        var snapshot = NSDiffableDataSourceSnapshot<Int, Item>()
        snapshot.appendSections([0])
        snapshot.appendItems(items)
        if changed { snapshot.reconfigureItems(items.filter { if case .member = $0 { true } else { false } }) }
        dataSource.apply(snapshot, animatingDifferences: false)
    }

    func collectionView(_ collectionView: UICollectionView, shouldSelectItemAt indexPath: IndexPath) -> Bool {
        guard openProfile != nil, case .member = dataSource.itemIdentifier(for: indexPath) else { return false }
        return true
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: false)
        guard case let .member(id, _) = dataSource.itemIdentifier(for: indexPath), let member = members[id] else { return }
        openProfile?(member)
    }
}

/// A face with its presence dot, then the name.
private final class MemberCell: UICollectionViewListCell {
    private let avatar = AvatarView(side: 30)
    private let dot = UIView()
    private let name = UIKitStyle.label(size: 12, weight: .medium)

    override init(frame: CGRect) {
        super.init(frame: frame)
        dot.translatesAutoresizingMaskIntoConstraints = false
        dot.layer.cornerRadius = 5
        dot.layer.borderWidth = 2
        name.lineBreakMode = .byTruncatingTail
        [avatar, dot, name].forEach(contentView.addSubview)
        NSLayoutConstraint.activate([
            avatar.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 12),
            avatar.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            dot.widthAnchor.constraint(equalToConstant: 10), dot.heightAnchor.constraint(equalToConstant: 10),
            dot.trailingAnchor.constraint(equalTo: avatar.trailingAnchor, constant: 1),
            dot.bottomAnchor.constraint(equalTo: avatar.bottomAnchor, constant: 1),
            name.leadingAnchor.constraint(equalTo: avatar.trailingAnchor, constant: 10),
            name.trailingAnchor.constraint(lessThanOrEqualTo: contentView.trailingAnchor, constant: -12),
            name.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            contentView.heightAnchor.constraint(equalToConstant: 38).withPriority(.init(999)),
        ])
        isAccessibilityElement = true
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ member: Member, status: PresenceStatus, quiet: Bool, raised: Bool, opens: Bool) {
        avatar.show(name: member.displayName, avatarID: member.avatarId, picture: member.avatarPicture)
        name.text = member.displayName
        let offline = status == .offline
        name.textColor = quiet && !offline ? UIKitStyle.muted : UIKitStyle.text
        contentView.alpha = offline ? 0.6 : 1
        let ring = raised ? UIKitStyle.sidebarRaised : UIKitStyle.sidebar
        dot.layer.borderColor = ring.cgColor
        switch status {
        case .online: dot.backgroundColor = UIKitStyle.green
        case .idle: dot.backgroundColor = UIColor(red: 0.72, green: 0.60, blue: 0.35, alpha: 1)
        case .offline, .unknown: dot.backgroundColor = UIKitStyle.border
        }
        var background = UIBackgroundConfiguration.clear()
        if raised { background.backgroundColor = UIKitStyle.sidebarRaised }
        backgroundConfiguration = background
        let state = status == .online ? "online" : status == .idle ? "idle" : status == .offline ? "offline" : "status unavailable"
        accessibilityLabel = opens ? "Open profile for \(member.displayName)" : member.displayName
        accessibilityValue = state
        accessibilityIdentifier = "member-\(member.username)"
        accessibilityTraits = opens ? .button : .staticText
    }
}
