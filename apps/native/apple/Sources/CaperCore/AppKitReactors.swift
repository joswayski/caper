#if os(macOS)
import AppKit

/// Who reacted, by emoji: tabs in chip order, then each person. Closes when
/// the last reaction goes away; removing the selected emoji selects the first.
@MainActor final class AppKitReactors: NSView, AppKitDialogContent {
    private let chat: ChatModel
    private let messageID: String
    private let viewerID: String?
    private let close: () -> Void
    private var selected: String
    private let observation = AppKitObservation()
    private let tabs = NSStackView()
    private let people = AppKitKit.stack([], spacing: 0)
    private var requested: String?
    var dismissDisabled: Bool { false }

    init(chat: ChatModel, messageID: String, emoji: String, viewerID: String?, close: @escaping () -> Void) {
        self.chat = chat; self.messageID = messageID; self.viewerID = viewerID; self.close = close; selected = emoji
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = AppKitKit.raised.cgColor

        let title = AppKitKit.label("Reactions", size: 20, weight: .bold, wraps: false)
        let done = AppKitKitButton("Done") { close() }
        done.keyEquivalent = "\u{1b}"
        let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        let header = NSStackView(views: [title, spacer, done]); header.orientation = .horizontal
        header.edgeInsets = NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)
        header.translatesAutoresizingMaskIntoConstraints = false

        tabs.orientation = .horizontal; tabs.spacing = 4
        tabs.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12)
        tabs.translatesAutoresizingMaskIntoConstraints = false
        let tabScroll = NSScrollView(); tabScroll.drawsBackground = false
        tabScroll.hasHorizontalScroller = false; tabScroll.hasVerticalScroller = false
        tabScroll.translatesAutoresizingMaskIntoConstraints = false
        tabScroll.documentView = tabs
        let line = AppKitKit.divider()
        let list = appKitKitScroll(people, insets: NSEdgeInsets(top: 0, left: 0, bottom: 8, right: 0))

        // Contain, so this identifier does not replace each tab's and row's own.
        let content = NSView(); content.translatesAutoresizingMaskIntoConstraints = false
        content.setAccessibilityElement(true); content.setAccessibilityRole(.group)
        content.setAccessibilityIdentifier("reactors-sheet")
        for view in [tabScroll, line, list] as [NSView] { content.addSubview(view) }
        addSubview(header); addSubview(content)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 420).withPriority(.defaultHigh),
            header.leadingAnchor.constraint(equalTo: leadingAnchor), header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.topAnchor.constraint(equalTo: topAnchor),
            content.leadingAnchor.constraint(equalTo: leadingAnchor), content.trailingAnchor.constraint(equalTo: trailingAnchor),
            content.topAnchor.constraint(equalTo: header.bottomAnchor), content.bottomAnchor.constraint(equalTo: bottomAnchor),
            tabScroll.leadingAnchor.constraint(equalTo: content.leadingAnchor), tabScroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            tabScroll.topAnchor.constraint(equalTo: content.topAnchor), tabScroll.heightAnchor.constraint(equalToConstant: 44),
            tabs.topAnchor.constraint(equalTo: tabScroll.contentView.topAnchor),
            tabs.leadingAnchor.constraint(equalTo: tabScroll.contentView.leadingAnchor),
            tabs.heightAnchor.constraint(equalToConstant: 44),
            line.leadingAnchor.constraint(equalTo: content.leadingAnchor), line.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            line.topAnchor.constraint(equalTo: tabScroll.bottomAnchor),
            list.leadingAnchor.constraint(equalTo: content.leadingAnchor), list.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            list.topAnchor.constraint(equalTo: line.bottomAnchor), list.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ])

        observation.start(reading: { [weak self] () -> ([MessageReaction], String?, ReactorsState?) in
            guard let self else { return ([], nil, nil) }
            let message = self.chat.messages.first { $0.id == self.messageID }
            let reactions = message?.reactions ?? []
            let selected = reactions.contains { $0.emoji == self.selected } ? self.selected : reactions.first?.emoji
            let state = message.flatMap { message in selected.map { self.chat.reactorsState(for: message, emoji: $0, viewerID: viewerID) } }
            return (reactions, message?.reactionSeq, state)
        }) { [weak self] reactions, sequence, _ in self?.update(reactions, sequence: sequence) }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func update(_ reactions: [MessageReaction], sequence: String?) {
        // Opening again after a change, or a change while open, refetches.
        let key = sequence ?? ""
        if requested != key { requested = key; _ = chat.requestReactors(messageID: messageID) }
        guard let first = reactions.first else { close(); return }
        if !reactions.contains(where: { $0.emoji == selected }) { selected = first.emoji }
        render(reactions)
    }

    private func render(_ reactions: [MessageReaction]) {
        tabs.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for reaction in reactions {
            let tab = AppKitReactorTab(emoji: reaction.emoji, count: reaction.authorIds.count, selected: reaction.emoji == selected) { [weak self] in
                guard let self else { return }
                self.selected = reaction.emoji
                self.render(self.chat.messages.first { $0.id == self.messageID }?.reactions ?? [])
            }
            tabs.addArrangedSubview(tab)
            if reaction.emoji == selected { DispatchQueue.main.async { tab.scrollToVisible(tab.bounds) } }
        }

        people.arrangedSubviews.forEach { $0.removeFromSuperview() }
        guard let message = chat.messages.first(where: { $0.id == messageID }),
              let reaction = reactions.first(where: { $0.emoji == selected }) else { return }
        func add(_ view: NSView) { people.addArrangedSubview(view); AppKitKit.fillWidth(view, of: people) }
        let name = AppKitKit.label(ReactionSummary.emojiLabel(emojiName: EmojiArtwork.name(for: reaction.emoji), emoji: reaction.emoji),
                                   size: 12, weight: .medium, color: AppKitKit.muted)
        name.setAccessibilityIdentifier("reactors-emoji-name")
        add(padded(name, NSEdgeInsets(top: 14, left: 16, bottom: 6, right: 16)))
        switch chat.reactorsState(for: message, emoji: reaction.emoji, viewerID: viewerID) {
        case .loaded(let reactors):
            for person in reactors { add(row(person)) }
        case .loading:
            let spinner = NSProgressIndicator(); spinner.style = .spinning; spinner.controlSize = .small; spinner.startAnimation(nil)
            let loading = NSStackView(views: [spinner, AppKitKit.label("Loading…", size: 13, color: AppKitKit.muted, wraps: false)])
            loading.orientation = .horizontal; loading.spacing = 8
            loading.setAccessibilityElement(true); loading.setAccessibilityRole(.staticText)
            loading.setAccessibilityLabel("Loading…"); loading.setAccessibilityIdentifier("reactors-loading")
            add(padded(loading, NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)))
        case .failed:
            let retry = AppKitKitButton("Retry") { [weak self] in
                guard let self else { return }
                _ = self.chat.requestReactors(messageID: self.messageID, force: true)
            }
            retry.setAccessibilityIdentifier("reactors-retry")
            let failed = AppKitKit.stack([AppKitKit.label("Couldn’t load reactions", size: 13, color: AppKitKit.muted), retry], spacing: 10)
            add(padded(failed, NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)))
        }
    }

    private func padded(_ view: NSView, _ insets: NSEdgeInsets) -> NSView {
        let box = NSView(); box.translatesAutoresizingMaskIntoConstraints = false
        view.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(view)
        NSLayoutConstraint.activate([
            view.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: insets.left),
            view.trailingAnchor.constraint(lessThanOrEqualTo: box.trailingAnchor, constant: -insets.right),
            view.topAnchor.constraint(equalTo: box.topAnchor, constant: insets.top),
            view.bottomAnchor.constraint(equalTo: box.bottomAnchor, constant: -insets.bottom),
        ])
        return box
    }

    private func row(_ person: ReactorPerson) -> NSView {
        let avatar = AppKitAvatar()
        avatar.translatesAutoresizingMaskIntoConstraints = false
        avatar.update(name: person.name, avatarId: person.avatarId, picture: person.avatarPicture)
        var lines: [NSView] = [AppKitKit.label(person.name, size: 14, weight: .bold, wraps: false)]
        var label = person.name
        if let username = person.username, !username.isEmpty {
            lines.append(AppKitKit.label("@\(username)", size: 12, color: AppKitKit.muted, wraps: false))
            label += ", @\(username)"
        }
        let text = AppKitKit.stack(lines, spacing: 2)
        let row = NSStackView(views: [avatar, text]); row.orientation = .horizontal; row.spacing = 10; row.alignment = .centerY
        row.edgeInsets = NSEdgeInsets(top: 7, left: 16, bottom: 7, right: 16)
        row.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([avatar.widthAnchor.constraint(equalToConstant: 34), avatar.heightAnchor.constraint(equalToConstant: 34)])
        row.setAccessibilityElement(true); row.setAccessibilityRole(.staticText)
        row.setAccessibilityLabel(label); row.setAccessibilityIdentifier("reactor-row-\(person.id)")
        return row
    }
}

/// One emoji tab: its artwork and count, underlined in terracotta when selected.
@MainActor private final class AppKitReactorTab: NSButton {
    private let handler: () -> Void
    private let isChosen: Bool

    init(emoji: String, count: Int, selected: Bool, action: @escaping () -> Void) {
        handler = action; isChosen = selected
        super.init(frame: .zero)
        title = ""; isBordered = false; focusRingType = .none
        target = self; self.action = #selector(fire)
        translatesAutoresizingMaskIntoConstraints = false
        let art: NSView
        if let entry = EmojiArtwork.entry(for: emoji), let image = EmojiArtwork.image(for: entry) {
            let view = NSImageView(image: NSImage(cgImage: image, size: NSSize(width: 20, height: 20)))
            view.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([view.widthAnchor.constraint(equalToConstant: 20), view.heightAnchor.constraint(equalToConstant: 20)])
            art = view
        } else {
            let glyph = NSTextField(labelWithString: emoji); glyph.font = .systemFont(ofSize: 16)
            art = glyph
        }
        let number = AppKitKit.label("\(count)", size: 12, weight: .bold, color: selected ? AppKitKit.text : AppKitKit.muted, wraps: false)
        let stack = NSStackView(views: [art, number]); stack.orientation = .horizontal; stack.spacing = 5
        stack.translatesAutoresizingMaskIntoConstraints = false
        art.setAccessibilityElement(false); number.setAccessibilityElement(false)
        addSubview(stack)
        let underline = NSView(); underline.wantsLayer = true
        underline.layer?.backgroundColor = (selected ? AppKitKit.terracotta : .clear).cgColor
        underline.translatesAutoresizingMaskIntoConstraints = false
        addSubview(underline)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 44),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            underline.leadingAnchor.constraint(equalTo: leadingAnchor), underline.trailingAnchor.constraint(equalTo: trailingAnchor),
            underline.bottomAnchor.constraint(equalTo: bottomAnchor), underline.heightAnchor.constraint(equalToConstant: 2),
        ])
        setAccessibilityLabel("\(emoji), \(count) \(count == 1 ? "person" : "people")")
        setAccessibilityIdentifier("reactors-tab-\(emoji)")
        setAccessibilitySelected(selected)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func fire() { handler() }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .pointingHand) }
}
#endif
