#if os(macOS)
import AppKit

/// ⌘K: find a channel, DM, space or person by name, unread first. ↑ and ↓
/// move the highlight while the field keeps focus; Return opens it.
@MainActor final class AppKitQuickSwitcher: AppKitSettingsDialog, NSTextFieldDelegate {
    private let model: AppModel
    private var items: [SwitcherItem] = []
    private let close: () -> Void
    private let search = AppKitKitField(height: 36)
    private let list = AppKitKit.stack([], spacing: 2)
    private let empty = AppKitSettingsDialog.note("")
    private var shown: [SwitcherItem] = []
    private var rows: [AppKitKitButton] = []
    private var selected = 0

    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model; self.close = close
        super.init(title: "Quick switcher", detail: "Channels, direct messages, spaces and people. Unread first; Return opens.",
                   closeLabel: "Close quick switcher", identifier: "quick-switcher", close: close)
        search.placeholderString = "Find a channel, DM, space or person"
        search.setAccessibilityLabel("Find a conversation"); search.delegate = self
        setRows([search, list, empty])
        preferHeight(520)
        // Other spaces' channels join the list as they load (as on web).
        observation.start(reading: { [weak self] in self?.model.switcherSpaces.count ?? 0 }) { [weak self] _ in
            guard let self else { return }
            self.items = self.model.switcherItems(); self.fill()
        }
        Task { [model] in await model.loadSwitcherSpaces() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var initialFocus: NSView? { search }

    private func fill() {
        // A refill keeps the highlighted destination when it still matches.
        let kept = shown.indices.contains(selected) ? shown[selected].id : nil
        shown = model.rankedSwitcherItems(items, query: search.stringValue)
        list.arrangedSubviews.forEach { $0.removeFromSuperview() }
        rows = shown.enumerated().map { index, item in
            let row = AppKitKitButton("\(item.name)  ·  \(item.detail)\(item.unread ? "  ●" : "")", kind: .plain) { [weak self] in
                self?.choose(index)
            }
            row.minimumHeight = 36; row.alignment = .left
            row.setAccessibilityLabel("\(item.name), \(item.detail)\(item.unread ? ", unread" : "")")
            row.setAccessibilityIdentifier("quick-switcher-\(item.id)")
            list.addArrangedSubview(row); AppKitKit.fillWidth(row, of: list)
            return row
        }
        empty.stringValue = search.stringValue.trimmingCharacters(in: .whitespaces).isEmpty ? "Nothing to switch to yet." : "No matches."
        empty.isHidden = !shown.isEmpty
        select(kept.flatMap { id in shown.firstIndex { $0.id == id } } ?? 0)
    }

    private func select(_ index: Int) {
        selected = shown.isEmpty ? 0 : (index % shown.count + shown.count) % shown.count
        for (i, row) in rows.enumerated() { row.showsSelection = i == selected }
        if rows.indices.contains(selected) { rows[selected].scrollToVisible(rows[selected].bounds) }
    }

    private func choose(_ index: Int) {
        guard shown.indices.contains(index) else { return }
        let item = shown[index]
        close()
        Task { [model] in await model.open(item) }
    }

    func controlTextDidChange(_ notification: Notification) { selected = 0; shown = []; fill() }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        // An input method composing in the field keeps its keys.
        guard !textView.hasMarkedText() else { return false }
        switch selector {
        case #selector(NSResponder.moveDown(_:)): select(selected + 1); return true
        case #selector(NSResponder.moveUp(_:)): select(selected - 1); return true
        case #selector(NSResponder.insertNewline(_:)): choose(selected); return true
        default: return false
        }
    }
}

/// ⌘/: every keyboard shortcut, in Apple's key names.
@MainActor final class AppKitShortcutHelp: AppKitSettingsDialog {
    init(close: @escaping () -> Void) {
        super.init(title: "Keyboard shortcuts", detail: "Also in the menu bar, under Go and Help.",
                   closeLabel: "Close keyboard shortcuts", identifier: "keyboard-shortcuts", close: close)
        var rows: [NSView] = []
        for section in KeyboardNavigation.shortcuts {
            rows.append(Self.heading(section.title))
            for row in section.rows {
                let action = AppKitKit.label(row.action, size: 13)
                let keys = AppKitKit.label(row.keys, size: 13, color: AppKitKit.muted)
                keys.setContentHuggingPriority(.required, for: .horizontal)
                action.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
                let line = NSStackView(views: [action, keys]); line.orientation = .horizontal; line.distribution = .fill
                line.setAccessibilityElement(true); line.setAccessibilityLabel("\(row.action): \(row.keys)")
                rows.append(line)
            }
        }
        setRows(rows)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}
#endif
