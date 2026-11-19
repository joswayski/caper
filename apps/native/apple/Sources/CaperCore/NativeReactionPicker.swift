import SwiftUI

/// Both native pickers use the same catalog and search order as autocomplete.
enum NativeEmojiPickerCatalog {
    static let categories = [
        ("smileys_people", "Smileys & people", "😀"), ("animals_nature", "Animals & nature", "🐱"),
        ("food_drink", "Food & drink", "🍔"), ("travel_places", "Travel & places", "🚌"),
        ("activities", "Activities", "⚽"), ("objects", "Objects", "👕"),
        ("symbols", "Symbols", "🎵"), ("flags", "Flags", "🏁"),
    ]
    static func choices(query: String, category: String) -> [EmojiCatalogEntry] {
        let term = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return EmojiArtwork.pickerChoices.filter {
            term.isEmpty ? $0.category == category
                : $0.name.lowercased().contains(term) || $0.keywords.lowercased().contains(term)
                    || $0.category.replacingOccurrences(of: "_", with: " ").contains(term)
        }
    }
}

#if os(macOS)
import AppKit

@MainActor final class AppKitReactionPicker: NSViewController, NSTextFieldDelegate, NSCollectionViewDataSource {
    private let pickerTitle: String
    private let close: () -> Void
    private let select: (String) -> Void
    private let search = NSTextField()
    private let heading = AppKitKit.label("", size: 12)
    private let empty = AppKitKit.label("No emoji found. Try another search.")
    private let grid = NSCollectionView()
    private let scroll = NSScrollView()
    private var category = "smileys_people"
    private var choices: [EmojiCatalogEntry] = []
    private var categories: [AppKitKitButton] = []

    init(title: String = "Add reaction", close: @escaping () -> Void, select: @escaping (String) -> Void) {
        pickerTitle = title; self.close = close; self.select = select
        super.init(nibName: nil, bundle: nil)
        preferredContentSize = NSSize(width: 352, height: 420)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        view = NSView(); view.wantsLayer = true; view.layer?.backgroundColor = AppKitKit.surface.cgColor
        view.setAccessibilityIdentifier("reaction-picker"); view.setAccessibilityLabel(pickerTitle)
        let title = AppKitKit.label(pickerTitle, size: 14, weight: .bold)
        let cancel = AppKitKitButton("Cancel", action: close); cancel.keyEquivalent = "\u{1b}"
        let header = NSStackView(views: [title, NSView(), cancel]); header.spacing = 8
        let tabs = NSStackView(); tabs.distribution = .fillEqually; tabs.spacing = 0
        for item in NativeEmojiPickerCatalog.categories {
            // The category name is the accessibility label and tooltip; as a
            // title AppKitKitButton would draw it over the emoji.
            let button = AppKitKitButton("", kind: .plain) { [weak self] in
                self?.category = item.0; self?.search.stringValue = ""; self?.reload()
            }
            button.imagePosition = .imageOnly
            if let entry = EmojiArtwork.entry(for: item.2), let image = EmojiArtwork.image(for: entry) {
                button.image = NSImage(cgImage: image, size: NSSize(width: 22, height: 22))
            }
            button.minimumHeight = 44; button.setAccessibilityLabel(item.1)
            button.setAccessibilityIdentifier("emoji-category-\(item.0)"); button.toolTip = item.1
            tabs.addArrangedSubview(button); categories.append(button)
        }
        search.placeholderString = "Search emoji"; search.setAccessibilityLabel("Search emoji")
        search.setAccessibilityIdentifier("reaction-picker-search"); search.delegate = self
        empty.setAccessibilityIdentifier("reaction-picker-empty")
        let layout = NSCollectionViewFlowLayout()
        layout.itemSize = NSSize(width: 44, height: 44); layout.minimumInteritemSpacing = 8; layout.minimumLineSpacing = 8
        grid.collectionViewLayout = layout; grid.dataSource = self; grid.backgroundColors = [.clear]
        grid.register(AppKitEmojiItem.self, forItemWithIdentifier: NSUserInterfaceItemIdentifier("emoji"))
        scroll.setAccessibilityIdentifier("reaction-picker-grid")
        scroll.documentView = grid; scroll.hasVerticalScroller = true; scroll.drawsBackground = false
        let stack = AppKitKit.stack([header, tabs, search, heading, empty, scroll], spacing: 10)
        view.addSubview(stack)
        for child in [header, tabs, search, heading, empty, scroll] { AppKitKit.fillWidth(child, of: stack) }
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 12),
            stack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -12),
        ])
        reload()
    }
    override func viewDidAppear() { super.viewDidAppear(); view.window?.makeFirstResponder(search) }
    func controlTextDidChange(_ notification: Notification) { reload() }
    private func reload() {
        let searching = !search.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        choices = NativeEmojiPickerCatalog.choices(query: search.stringValue, category: category)
        for (button, item) in zip(categories, NativeEmojiPickerCatalog.categories) {
            let selected = !searching && category == item.0
            button.state = selected ? .on : .off; button.setAccessibilityValue(selected ? "Selected" : "Not selected")
            button.showsSelection = selected
        }
        heading.stringValue = searching ? "Search results" : NativeEmojiPickerCatalog.categories.first { $0.0 == category }?.1 ?? ""
        empty.isHidden = !choices.isEmpty
        grid.reloadData(); scroll.contentView.scroll(to: .zero); scroll.reflectScrolledClipView(scroll.contentView)
    }
    func collectionView(_ collectionView: NSCollectionView, numberOfItemsInSection section: Int) -> Int { choices.count }
    func collectionView(_ collectionView: NSCollectionView, itemForRepresentedObjectAt indexPath: IndexPath) -> NSCollectionViewItem {
        let item = collectionView.makeItem(withIdentifier: NSUserInterfaceItemIdentifier("emoji"), for: indexPath) as! AppKitEmojiItem
        item.configure(choices[indexPath.item], select: select)
        return item
    }
}

@MainActor private final class AppKitEmojiItem: NSCollectionViewItem {
    private var pick: (() -> Void)?
    private lazy var button = AppKitKitButton("", kind: .plain) { [weak self] in self?.pick?() }
    override func loadView() { view = button }
    func configure(_ entry: EmojiCatalogEntry, select: @escaping (String) -> Void) {
        _ = view
        // No title: AppKitKitButton redraws it over the emoji on hover, as on
        // the category tabs. The name stays the label and tooltip (web's title).
        button.imagePosition = .imageOnly
        button.setAccessibilityIdentifier("emoji-\(entry.id)")
        button.image = EmojiArtwork.image(for: entry).map { NSImage(cgImage: $0, size: NSSize(width: 30, height: 30)) }
        button.setAccessibilityLabel(entry.name); button.toolTip = entry.name
        pick = { select(entry.emoji) }
    }
}
#elseif os(iOS)
import UIKit

struct NativeReactionPickerBridge: UIViewControllerRepresentable {
    let title: String
    let embedded: Bool
    let close: () -> Void
    let select: (String) -> Void
    func makeUIViewController(context: Context) -> UIKitReactionPicker {
        UIKitReactionPicker(title: title, embedded: embedded, close: close, select: select)
    }
    func updateUIViewController(_ controller: UIKitReactionPicker, context: Context) {}
}

@MainActor final class UIKitReactionPicker: UIViewController, UICollectionViewDataSource {
    private let select: (String) -> Void
    private let close: () -> Void
    private let embedded: Bool
    private let search = UITextField()
    private let heading = UIKitMessageDialog.label("", bold: true)
    private let empty = UIKitMessageDialog.label("No emoji found. Try another search.")
    private let grid: UICollectionView
    private var category = "smileys_people"
    private var choices: [EmojiCatalogEntry] = []
    private var categories: [UIButton] = []

    init(title: String = "Add reaction", embedded: Bool = false, close: @escaping () -> Void, select: @escaping (String) -> Void) {
        self.select = select; self.close = close; self.embedded = embedded
        let layout = UICollectionViewFlowLayout()
        layout.itemSize = CGSize(width: 44, height: 44); layout.minimumInteritemSpacing = 8; layout.minimumLineSpacing = 8
        grid = UICollectionView(frame: .zero, collectionViewLayout: layout)
        super.init(nibName: nil, bundle: nil); self.title = title
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIColor(CaperTheme.surface); overrideUserInterfaceStyle = .dark
        view.accessibilityIdentifier = "reaction-picker"
        let cancel = UIKitMessageDialog.button("Cancel", action: close)
        let header = UIKitMessageDialog.header(UIKitMessageDialog.label(title ?? "", size: 20, bold: true), cancel, spacing: 8)
        header.isHidden = embedded
        let tabs = UIStackView(); tabs.distribution = .fillEqually
        for item in NativeEmojiPickerCatalog.categories {
            let button = UIButton(type: .system)
            if let entry = EmojiArtwork.entry(for: item.2), let image = EmojiArtwork.image(for: entry) {
                button.setImage(UIImage(cgImage: image, scale: CGFloat(image.width) / 22, orientation: .up).withRenderingMode(.alwaysOriginal), for: .normal)
            }
            button.accessibilityLabel = item.1; button.accessibilityIdentifier = "emoji-category-\(item.0)"
            button.addAction(UIAction { [weak self] _ in
                self?.category = item.0; self?.search.text = ""; self?.reload()
            }, for: .touchUpInside)
            tabs.addArrangedSubview(button); categories.append(button)
        }
        tabs.heightAnchor.constraint(equalToConstant: 44).isActive = true
        search.placeholder = "Search emoji"; search.accessibilityLabel = search.placeholder
        search.accessibilityIdentifier = "reaction-picker-search"; search.borderStyle = .roundedRect
        search.autocorrectionType = .no; search.autocapitalizationType = .none
        search.addTarget(self, action: #selector(reload), for: .editingChanged)
        search.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        empty.accessibilityIdentifier = "reaction-picker-empty"
        grid.backgroundColor = .clear; grid.dataSource = self; grid.keyboardDismissMode = .interactive
        grid.accessibilityIdentifier = "reaction-picker-grid"
        grid.register(UIKitEmojiCell.self, forCellWithReuseIdentifier: "emoji")
        let stack = UIStackView(arrangedSubviews: [header, tabs, search, heading, empty, grid])
        stack.axis = .vertical; stack.spacing = 10; stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            stack.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor, constant: -12),
        ])
        reload()
    }
    override var keyCommands: [UIKeyCommand]? { [UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(cancel))] }
    @objc private func cancel() { close() }
    @objc private func reload() {
        let query = search.text ?? "", searching = !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        choices = NativeEmojiPickerCatalog.choices(query: query, category: category)
        for (button, item) in zip(categories, NativeEmojiPickerCatalog.categories) {
            let selected = !searching && category == item.0
            button.backgroundColor = selected ? UIColor(CaperTheme.border) : .clear
            button.accessibilityTraits = selected ? [.button, .selected] : .button
        }
        heading.text = searching ? "Search results" : NativeEmojiPickerCatalog.categories.first { $0.0 == category }?.1 ?? ""
        empty.isHidden = !choices.isEmpty; grid.reloadData(); grid.setContentOffset(.zero, animated: false)
    }
    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int { choices.count }
    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
        let cell = collectionView.dequeueReusableCell(withReuseIdentifier: "emoji", for: indexPath) as! UIKitEmojiCell
        cell.configure(choices[indexPath.item], select: select); return cell
    }
}

@MainActor private final class UIKitEmojiCell: UICollectionViewCell {
    private let button = UIButton(type: .system)
    private var pick: (() -> Void)?
    override init(frame: CGRect) {
        super.init(frame: frame)
        button.translatesAutoresizingMaskIntoConstraints = false; contentView.addSubview(button)
        NSLayoutConstraint.activate([
            button.leadingAnchor.constraint(equalTo: contentView.leadingAnchor), button.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            button.topAnchor.constraint(equalTo: contentView.topAnchor), button.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])
        button.addAction(UIAction { [weak self] _ in self?.pick?() }, for: .touchUpInside)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func configure(_ entry: EmojiCatalogEntry, select: @escaping (String) -> Void) {
        button.setImage(EmojiArtwork.image(for: entry).map {
            UIImage(cgImage: $0, scale: CGFloat($0.width) / 30, orientation: .up).withRenderingMode(.alwaysOriginal)
        }, for: .normal)
        button.accessibilityIdentifier = "emoji-\(entry.id)"
        button.accessibilityLabel = entry.name; pick = { select(entry.emoji) }
    }
}
#endif
