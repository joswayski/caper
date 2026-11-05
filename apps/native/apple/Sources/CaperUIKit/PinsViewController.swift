import CaperCore
import UIKit

/// The channel's pinned messages (SwiftUI's PinnedMessagesView): cards with
/// the message and Go to message, newest pin first. Your pins are terracotta,
/// other people's faded gold.
final class PinsViewController: UIViewController, UICollectionViewDelegate {
    private let model: AppModel
    private var collection: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, String>!
    private var rows: [String: (state: MessageRowState, message: ChatMessage)] = [:]
    private let empty = UIStackView()
    private let error = UIKitStyle.label(size: 12, color: UIKitStyle.error)
    private let observer = ModelObserver()

    init(model: AppModel) {
        self.model = model
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .pageSheet
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.conversation
        view.accessibilityIdentifier = "pinned-messages"

        let bar = UIView()
        bar.translatesAutoresizingMaskIntoConstraints = false
        let title = UIKitStyle.label("Pins", size: 15, weight: .bold)
        title.accessibilityTraits = .header
        let close = IconButton(icon: "x", label: "Close pins", size: 18) { [weak self] in self?.dismiss(animated: true) }
        close.tintColor = UIKitStyle.muted
        close.accessibilityIdentifier = "close-pins"
        let rule = UIView()
        rule.backgroundColor = UIKitStyle.border
        rule.translatesAutoresizingMaskIntoConstraints = false
        [title, close, rule].forEach(bar.addSubview)
        view.addSubview(bar)

        error.numberOfLines = 0
        error.isHidden = true

        let icon = UIImageView(image: UIKitStyle.icon("pin")?.resized(to: 28))
        icon.tintColor = UIKitStyle.muted
        icon.contentMode = .center
        let emptyTitle = UIKitStyle.label("No pinned messages", size: 17, weight: .bold)
        let emptyNote = UIKitStyle.label("Pinned messages in this channel will appear here.", size: 13, color: UIKitStyle.muted)
        emptyNote.numberOfLines = 0
        emptyNote.textAlignment = .center
        [icon, emptyTitle, emptyNote].forEach(empty.addArrangedSubview)
        empty.axis = .vertical
        empty.alignment = .center
        empty.spacing = 8
        empty.translatesAutoresizingMaskIntoConstraints = false
        empty.isHidden = true
        view.addSubview(empty)

        let size = NSCollectionLayoutSize(widthDimension: .fractionalWidth(1), heightDimension: .estimated(120))
        let section = NSCollectionLayoutSection(group: .vertical(layoutSize: size, subitems: [NSCollectionLayoutItem(layoutSize: size)]))
        section.interGroupSpacing = 12
        section.contentInsets = NSDirectionalEdgeInsets(top: 12, leading: 12, bottom: 12, trailing: 12)
        collection = UICollectionView(frame: .zero, collectionViewLayout: UICollectionViewCompositionalLayout(section: section))
        collection.backgroundColor = .clear
        collection.delegate = self
        collection.register(PinCardCell.self, forCellWithReuseIdentifier: PinCardCell.reuse)
        dataSource = UICollectionViewDiffableDataSource(collectionView: collection) { [weak self] collection, path, id in
            let cell = collection.dequeueReusableCell(withReuseIdentifier: PinCardCell.reuse, for: path) as! PinCardCell
            guard let self, let row = self.rows[id] else { return cell }
            cell.configure(row.state, mine: row.state.pinMine, jumping: self.model.chat.jumpingToMessage) { [weak self] in
                self?.goTo(row.message)
            }
            return cell
        }
        let column = UIStackView(arrangedSubviews: [error, collection])
        column.axis = .vertical
        column.spacing = 0
        column.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(column)

        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            bar.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            bar.heightAnchor.constraint(equalToConstant: 50),
            title.centerXAnchor.constraint(equalTo: bar.centerXAnchor),
            title.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
            close.trailingAnchor.constraint(equalTo: bar.trailingAnchor, constant: -6),
            close.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
            rule.leadingAnchor.constraint(equalTo: bar.leadingAnchor),
            rule.trailingAnchor.constraint(equalTo: bar.trailingAnchor),
            rule.bottomAnchor.constraint(equalTo: bar.bottomAnchor),
            rule.heightAnchor.constraint(equalToConstant: 1),
            column.topAnchor.constraint(equalTo: bar.bottomAnchor),
            column.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            column.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            empty.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            empty.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            empty.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -48),
        ])
        observer.start { [weak self] in self?.render() }
    }

    private func render() {
        let chat = model.chat
        let viewer = chat.currentAuthor?.id ?? model.account?.id
        let pins = chat.displayedPins
        var next: [String: (state: MessageRowState, message: ChatMessage)] = [:]
        for message in pins {
            next[message.id] = (MessageRowState(message: message, grouped: false, continued: false, viewerID: viewer, inPins: true), message)
        }
        let changed = pins.map(\.id).filter { rows[$0]?.state != next[$0]?.state }
        rows = next
        error.text = chat.jumpError.map { "  \($0)" }
        error.isHidden = chat.jumpError == nil
        empty.isHidden = !pins.isEmpty
        var snapshot = NSDiffableDataSourceSnapshot<Int, String>()
        snapshot.appendSections([0])
        snapshot.appendItems(pins.map(\.id))
        snapshot.reconfigureItems(changed)
        dataSource.apply(snapshot, animatingDifferences: false)
        // The Go to message label follows the jump's progress.
        for case let cell as PinCardCell in collection.visibleCells { cell.setJumping(chat.jumpingToMessage) }
    }

    private func goTo(_ message: ChatMessage) {
        let chat = model.chat
        guard !chat.jumpingToMessage else { return }
        Task { [weak self] in
            if await chat.goToMessage(message) { self?.dismiss(animated: true) }
        }
    }
}

private final class PinCardCell: UICollectionViewCell {
    static let reuse = "pin"
    private let row = MessageRowView()
    private lazy var go = CaperButton("Go to message", kind: .secondary, icon: "arrow-right", size: 13) { [weak self] in self?.action?() }
    private var action: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        contentView.layer.cornerRadius = 8
        contentView.layer.cornerCurve = .continuous
        row.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(row)
        contentView.addSubview(go)
        NSLayoutConstraint.activate([
            row.topAnchor.constraint(equalTo: contentView.topAnchor),
            row.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            row.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            go.topAnchor.constraint(equalTo: row.bottomAnchor),
            go.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -18),
            go.heightAnchor.constraint(equalToConstant: 44),
            go.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -12).withPriority(.init(999)),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ state: MessageRowState, mine: Bool, jumping: Bool, go action: @escaping () -> Void) {
        row.configure(state)
        self.action = action
        contentView.backgroundColor = mine ? UIKitStyle.terracotta.withAlphaComponent(0.12) : UIKitStyle.pinGold.withAlphaComponent(0.06)
        go.accessibilityIdentifier = "go-to-message-\(state.messageID)"
        accessibilityIdentifier = "pinned-message-\(state.messageID)"
        setJumping(jumping)
    }

    func setJumping(_ jumping: Bool) {
        go.setTitle(jumping ? "Loading message…" : "Go to message")
        go.isEnabled = !jumping
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        row.prepareForReuse()
        action = nil
    }
}
