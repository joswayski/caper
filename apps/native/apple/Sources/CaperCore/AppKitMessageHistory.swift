#if os(macOS)
import AppKit

/// "Message history": the latest change side by side, then any earlier
/// version chosen from a menu, compared with the one before it.
@MainActor final class AppKitMessageHistory: NSView, AppKitDialogContent {
    private let chat: ChatModel
    private let message: ChatMessage
    private let close: () -> Void
    private let observation = AppKitObservation()
    private let body = AppKitKit.stack([], spacing: 16)
    private var versions: [MessageVersion] = []
    private var selected = 0
    private var loading = false
    private var more = false
    private var error: String?
    private var request = 0
    private var loadedRevision: Int?
    var dismissDisabled: Bool { false }

    init(chat: ChatModel, message: ChatMessage, close: @escaping () -> Void) {
        self.chat = chat; self.message = message; self.close = close
        super.init(frame: .zero)
        let title = AppKitKit.label("Message history", size: 20, weight: .bold, wraps: false)
        let closeButton = AppKitKitButton("Close") { close() }
        closeButton.keyEquivalent = "\u{1b}"
        let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        let header = NSStackView(views: [title, spacer, closeButton]); header.orientation = .horizontal
        let detail = AppKitKit.label("\(message.author.name) · Previous versions are retained.", size: 12, color: AppKitKit.muted)
        let content = AppKitKit.stack([header, detail, body], spacing: 16)
        for view in [header, detail, body] as [NSView] { AppKitKit.fillWidth(view, of: content) }
        let scroll = appKitKitScroll(content, insets: NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20))
        wantsLayer = true
        layer?.backgroundColor = AppKitKit.raised.cgColor
        addSubview(scroll)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor), scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.topAnchor.constraint(equalTo: topAnchor), scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
            heightAnchor.constraint(equalToConstant: 580).withPriority(.defaultHigh),
        ])
        // Reload when the message is edited again; close when it is deleted.
        observation.start(reading: { [chat, message] in
            ((chat.messages + chat.pinnedMessages).first { $0.id == message.id }?.revision ?? message.revision ?? 1,
             chat.isDeleted(message.id))
        }) { [weak self] revision, deleted in
            guard let self else { return }
            if deleted { self.close(); return }
            if revision != self.loadedRevision { self.loadedRevision = revision; Task { await self.load(older: false) } }
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func load(older: Bool) async {
        request += 1
        let current = request
        loading = true; error = nil; render()
        do {
            let page = try await chat.messageVersions(message, before: older ? versions.last?.revision : nil)
            guard current == request else { return }
            versions = older ? versions + page.versions.filter { item in !versions.contains { $0.revision == item.revision } } : page.versions
            more = page.hasMore
            if !older { selected = 0 }
        } catch is CancellationError { return }
        catch { if current == request { self.error = error.localizedDescription } }
        if current == request { loading = false; render() }
    }

    private func render() {
        body.arrangedSubviews.forEach { $0.removeFromSuperview() }
        func add(_ view: NSView, fill: Bool = true) {
            body.addArrangedSubview(view)
            if fill { AppKitKit.fillWidth(view, of: body) }
        }
        if versions.count > 1 { add(comparison(before: versions[1], after: versions[0], current: true)) }
        else if let original = versions.first {
            add(AppKitKit.label("Original version"))
            add(selectable(original.content.text, font: AppKitKit.font(14)))
        }
        if versions.count > 1 {
            add(AppKitKit.divider())
            let picker = NSPopUpButton(frame: .zero, pullsDown: false)
            picker.setAccessibilityLabel("View previous versions")
            picker.addItem(withTitle: "Choose an earlier version…"); picker.lastItem?.tag = 0
            for version in versions.dropFirst() {
                picker.addItem(withTitle: Self.label(version)); picker.lastItem?.tag = version.revision
            }
            picker.selectItem(withTag: selected)
            picker.target = self; picker.action = #selector(choose(_:))
            add(picker, fill: false)
            if let version = versions.first(where: { $0.revision == selected }) {
                if let before = versions.first(where: { $0.revision == selected - 1 }) {
                    add(comparison(before: before, after: version, current: false))
                } else {
                    add(AppKitKit.label(version.revision == 1 ? "Original version" : "Version \(version.revision)"))
                    add(selectable(version.content.text, font: .monospacedSystemFont(ofSize: 12, weight: .regular)))
                    if version.revision > 1 && more {
                        add(AppKitKit.label("Load older versions to compare this change.", color: AppKitKit.muted))
                    }
                }
            }
        }
        if loading {
            let spinner = NSProgressIndicator(); spinner.style = .spinning; spinner.controlSize = .small; spinner.startAnimation(nil)
            let row = NSStackView(views: [spinner, AppKitKit.label("Loading versions…", color: AppKitKit.muted, wraps: false)])
            row.orientation = .horizontal; row.spacing = 8
            add(row, fill: false)
        }
        if let error {
            add(AppKitKit.label(error, color: AppKitKit.error))
            let retry = AppKitKitButton("Retry") { [weak self] in
                guard let self else { return }
                Task { await self.load(older: !self.versions.isEmpty) }
            }
            retry.isEnabled = !loading
            add(retry, fill: false)
        }
        if more && error == nil {
            let older = AppKitKitButton("Load older versions") { [weak self] in Task { await self?.load(older: true) } }
            older.isEnabled = !loading
            add(older, fill: false)
        }
    }

    @objc private func choose(_ sender: NSPopUpButton) {
        selected = sender.selectedTag()
        render()
    }

    private static func label(_ version: MessageVersion) -> String {
        let date = ChatDateDivider.date(version.createdAt)?.formatted(date: .abbreviated, time: .shortened) ?? version.createdAt
        return "\(version.revision == 1 ? "Original version" : "Version \(version.revision)") · \(date)"
    }

    private func selectable(_ text: String, font: NSFont, attributed: NSAttributedString? = nil) -> NSTextField {
        let label = AppKitKit.label(text)
        label.font = font
        if let attributed { label.attributedStringValue = attributed }
        label.isSelectable = true
        return label
    }

    /// Before and after columns, each version's changed runs highlighted.
    private func comparison(before: MessageVersion, after: MessageVersion, current: Bool) -> NSView {
        let diff = messageDiff(before: before.content.text, after: after.content.text)
        func column(_ version: MessageVersion, title: String, tokens: [MessageDiffToken], color: NSColor) -> NSView {
            let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
            let text = NSMutableAttributedString()
            for token in tokens {
                var attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: AppKitKit.text]
                if token.changed { attributes[.backgroundColor] = color.withAlphaComponent(0.3) }
                text.append(NSAttributedString(string: token.text, attributes: attributes))
            }
            let body = selectable(version.content.text, font: font, attributed: text)
            let rows: [NSView] = [AppKitKit.label(title, size: 14, weight: .bold),
                                  AppKitKit.label(Self.label(version), size: 10, color: AppKitKit.muted), body]
            let stack = AppKitKit.stack(rows, spacing: 8)
            for row in rows { AppKitKit.fillWidth(row, of: stack) }
            return stack
        }
        let left = column(before, title: current ? "Previous version" : "Before", tokens: diff.0, color: AppKitKit.terracotta)
        let right = column(after, title: current ? "Current version" : "After", tokens: diff.1,
                           color: NSColor(red: 99/255, green: 122/255, blue: 67/255, alpha: 1))
        let row = NSStackView(views: [left, right])
        row.orientation = .horizontal; row.alignment = .top; row.spacing = 12; row.distribution = .fillEqually
        row.translatesAutoresizingMaskIntoConstraints = false
        return row
    }
}

extension NSLayoutConstraint {
    func withPriority(_ priority: NSLayoutConstraint.Priority) -> NSLayoutConstraint { self.priority = priority; return self }
}
#endif
