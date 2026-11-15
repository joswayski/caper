#if os(iOS)
import UIKit
import Observation

/// A native presentation owner, not a hidden SwiftUI graph. Message actions
/// capture the conversation; follow-up sheets wait for dismissal to complete.
@MainActor public final class UIKitNativePresentations: UIViewController, UIAdaptivePresentationControllerDelegate {
    private let state: UIKitPresentations
    private let model: AppModel
    private let inThread: Bool
    private let leave: ((@escaping () -> Void) -> Void)?
    private var context: String
    private var author: String?
    private var owned: UIViewController?
    private var kind: String?
    private var closing = false
    private var active = true
    public init(state: UIKitPresentations, model: AppModel, inThread: Bool = false, leave: ((@escaping () -> Void) -> Void)? = nil) {
        self.state = state; self.model = model; self.inThread = inThread; self.leave = leave
        context = model.chat.editingContext; author = model.chat.currentAuthor?.id
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    public override func loadView() { view = UIView(); view.isUserInteractionEnabled = false; view.accessibilityElementsHidden = true }
    public override func viewDidLoad() { super.viewDidLoad(); observe() }
    public override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); render() }
    public override func didMove(toParent parent: UIViewController?) {
        super.didMove(toParent: parent)
        if parent == nil { active = false; (owned as? UIKitMessageDialog)?.stop(); owned?.dismiss(animated: false) }
    }
    private func observe() { guard active else { return }; withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe() } } }
    private func render() {
        let chat = model.chat
        // Read every request even while another sheet is open, so the next
        // action is observed rather than lost behind an early return.
        let action = state.actions, reaction = state.reactors, block = state.block
        let forward = chat.forwardTarget, conversation = chat.forwardConversationTarget
        let frontmost = leave == nil && (chat.threadRootID != nil) == inThread
        if context != chat.editingContext || author != chat.currentAuthor?.id || (chat.isPreview && action != nil) {
            context = chat.editingContext; author = chat.currentAuthor?.id
            state.actions = nil; state.reactors = nil; state.showingEmojiPicker = false
            finish(); return
        }
        guard view.window != nil, !closing, owned == nil else { return }
        if let block { confirmBlock(block); return }
        if let action {
            show(UIKitNativeMessageActions(model: model, message: action, inThread: inThread,
                leave: leave, close: { [weak self] then in self?.finish(then: then) },
                reactors: { [weak self] emoji in self?.state.reactors = .init(messageID: action.id, emoji: emoji) },
                block: { [weak self] in self?.state.block = BlockTarget(author: action.author) }), kind: "actions")
        } else if let reaction {
            show(UIKitNativeReactors(chat: chat, messageID: reaction.messageID, emoji: reaction.emoji,
                viewerID: chat.currentAuthor?.id ?? model.account?.id, close: { [weak self] in self?.finish() }), kind: "reactors")
        } else if frontmost, let forward {
            show(UIKitForwardPicker(chat: chat, message: forward, close: { [weak self] in self?.finish() }), kind: "forward")
        } else if frontmost, let conversation {
            show(UIKitForwardConversation(chat: chat, messageID: conversation.id, close: { [weak self] in self?.finish() }), kind: "conversation")
        }
    }
    private func show(_ controller: UIViewController, kind: String) {
        guard presentedViewController == nil, parent?.presentedViewController == nil else { return }
        self.kind = kind; owned = controller
        UIKitHosting.sheet(controller, background: CaperTheme.raised)
        controller.sheetPresentationController?.detents = [.large()]
        controller.presentationController?.delegate = self
        present(controller, animated: !CaperAccessibility.reduceMotion)
    }
    private func clearRequest() {
        switch kind {
        case "actions": state.actions = nil; state.showingEmojiPicker = false
        case "reactors": state.reactors = nil
        case "forward": model.chat.forwardTarget = nil
        case "conversation": model.chat.forwardConversationTarget = nil
        case "block": state.block = nil
        default: break
        }
        kind = nil
    }
    private func finish(then: (() -> Void)? = nil) {
        guard !closing else { return }
        closing = true; clearRequest()
        let old = owned; owned = nil; (old as? UIKitMessageDialog)?.stop()
        let complete: () -> Void = { [weak self] in self?.closing = false; then?(); self?.render(); CaperFocus.presentationDidClose() }
        // Dismiss from the presenter so an editor or picker stacked on the sheet goes too;
        // a sheet the reader already swiped away has no presenter and finishes at once.
        if let presenter = old?.presentingViewController { presenter.dismiss(animated: false, completion: complete) } else { complete() }
    }
    public func presentationControllerShouldDismiss(_ presentationController: UIPresentationController) -> Bool {
        (owned as? UIKitMessageDialog)?.dismissDisabled != true
    }
    public func presentationControllerDidDismiss(_ presentationController: UIPresentationController) { finish() }
    private func confirmBlock(_ target: BlockTarget) {
        guard target.canBlock, target.id != model.account?.id else { state.block = nil; return }
        let alert = UIAlertController(title: target.confirmationTitle, message: BlockTarget.confirmationMessage, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { [weak self] _ in self?.finish() })
        alert.addAction(UIAlertAction(title: "Block", style: .destructive) { [weak self, model] _ in
            self?.finish()
            Task { do { try await model.block(target) } catch { model.chat.error = FriendlyError.message(for: error) } }
        })
        kind = "block"; owned = alert; present(alert, animated: !CaperAccessibility.reduceMotion)
    }
}

@MainActor private final class UIKitNativeMessageActions: UIKitMessageDialog {
    private let model: AppModel
    private let original: ChatMessage
    private let inThread: Bool
    private let leave: ((@escaping () -> Void) -> Void)?
    private let finish: ((() -> Void)?) -> Void
    private let reactors: (String) -> Void
    private let block: () -> Void
    init(model: AppModel, message: ChatMessage, inThread: Bool, leave: ((@escaping () -> Void) -> Void)?,
         close: @escaping ((() -> Void)?) -> Void, reactors: @escaping (String) -> Void, block: @escaping () -> Void) {
        self.model = model; original = message; self.inThread = inThread; self.leave = leave
        finish = close; self.reactors = reactors; self.block = block
        super.init(title: "Message actions", close: { close(nil) })
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() { super.viewDidLoad(); body.accessibilityIdentifier = "message-actions-sheet"; observe() }
    private var current: ChatMessage {
        let chat = model.chat
        return (chat.displayedMessages + chat.displayedThreadMessages + chat.displayedPins).first { $0.id == original.id } ?? original
    }
    private func afterClosing(_ action: @escaping () -> Void) { finish { [leave] in if let leave { leave(action) } else { action() } } }
    override func render() {
        let chat = model.chat, message = current
        body.arrangedSubviews.forEach { body.removeArrangedSubview($0); $0.removeFromSuperview() }
        func button(_ title: String, id: String? = nil, enabled: Bool = true, action: @escaping () -> Void) {
            let control = Self.button(title, action: action); control.contentHorizontalAlignment = .leading
            control.accessibilityIdentifier = id; control.isEnabled = enabled; body.addArrangedSubview(control)
        }
        if !message.isDeleted {
            let quick = UIStackView(); quick.spacing = 4; quick.distribution = .fillEqually
            for emoji in MessageQuickReactions.emoji {
                let control = Self.button(emoji) { [weak self] in self?.react(emoji, toggle: true) }
                if let artwork = NativePresentation.emojiImage(emoji) {
                    control.configuration?.title = nil
                    control.configuration?.image = UIImage(cgImage: artwork, scale: CGFloat(artwork.width) / 22, orientation: .up).withRenderingMode(.alwaysOriginal)
                }
                control.accessibilityLabel = "React with \(emoji)"; control.accessibilityIdentifier = "quick-reaction-\(emoji)"
                control.isEnabled = chat.canInteract; quick.addArrangedSubview(control)
            }
            body.addArrangedSubview(quick)
            button("Add reaction", id: "message-action-add-reaction", enabled: chat.canInteract) { [weak self] in
                guard let self else { return }
                let picker = UIKitHosting.reactionPicker { [weak self] in self?.react($0, toggle: false) }
                self.present(picker, animated: !CaperAccessibility.reduceMotion)
            }
            button("Reply", enabled: chat.canInteract && chat.pendingMessage == nil) { [weak self] in
                guard let self, self.model.chat.canInteract else { return }
                self.afterClosing { chat.reply(to: self.current, inThread: self.inThread) }
            }
            if chat.canEdit(message) {
                button("Edit message") { [weak self] in
                    guard let self, chat.canEdit(self.current) else { return }
                    let editor = UIKitMessageEditor(chat: chat, message: self.current) { [weak self] in self?.finish(nil) }
                    UIKitHosting.sheet(editor, background: CaperTheme.raised); self.present(editor, animated: !CaperAccessibility.reduceMotion)
                }
            }
            if message.forward == nil, (message.revision ?? 1) > 1 {
                button("Message history") { [weak self] in
                    guard let self else { return }
                    let history = UIKitNativeMessageHistory(chat: chat, message: self.current) { [weak self] in self?.presentedViewController?.dismiss(animated: false) }
                    UIKitHosting.sheet(history, background: CaperTheme.raised); self.present(history, animated: !CaperAccessibility.reduceMotion)
                }
            }
            if let reaction = message.reactions?.first {
                button("View reactions", id: "message-action-view-reactions") { [weak self, reactors] in self?.finish { reactors(reaction.emoji) } }
            }
            button(message.pin == nil ? "Pin message" : "Unpin message", enabled: chat.canInteract && !chat.pendingPins.contains(message.id)) { [weak self] in
                guard let self, chat.canInteract else { return }; let current = self.current
                self.finish { Task { await chat.setPin(messageID: current.id, active: current.pin == nil) } }
            }
            if !inThread { button("Reply in thread") { [weak self] in self?.afterClosing { chat.replyInThread(to: message) } } }
            if chat.canForward { button("Forward message") { [weak self] in self?.afterClosing { chat.forwardTarget = message } } }
            if !message.content.text.isEmpty {
                let links = MessageLinks.segments(in: message.content.text).compactMap(\.href)
                if !links.isEmpty {
                    let control = Self.button("Links") {}; control.showsMenuAsPrimaryAction = true
                    control.menu = UIMenu(children: links.compactMap { link in
                        guard let url = LinkEmbeds.openableURL(link) else { return nil }
                        return UIMenu(title: link, children: [UIAction(title: "Open link") { _ in UIApplication.shared.open(url) },
                            UIAction(title: "Copy link") { _ in UIPasteboard.general.string = link }])
                    }); body.addArrangedSubview(control)
                }
                button("Copy text") { [weak self] in UIPasteboard.general.string = message.content.text; self?.finish(nil) }
            }
        } else if !inThread, message.threadRootId != nil || (message.thread?.replyCount ?? 0) > 0 {
            button("View thread") { [weak self] in self?.afterClosing { chat.replyInThread(to: message) } }
        }
        button("Copy message ID") { [weak self] in UIPasteboard.general.string = message.id; self?.finish(nil) }
        guard !message.isDeleted else { return }
        if chat.canDelete(message) {
            button("Delete message", id: "message-action-delete") { [weak self] in
                guard let self, chat.canDelete(self.current) else { return }
                let alert = UIAlertController(title: MessageDeletion.confirmTitle, message: MessageDeletion.confirmMessage, preferredStyle: .alert)
                alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
                alert.addAction(UIAlertAction(title: "Delete", style: .destructive) { [weak self] _ in
                    guard let self else { return }; let current = self.current
                    self.finish { Task { await chat.deleteMessage(current) } }
                }); self.present(alert, animated: !CaperAccessibility.reduceMotion)
            }
        }
        let viewer = chat.currentAuthor?.id ?? model.account?.id
        if BlockedMessages.canBlock(message.author, viewerID: viewer, blocked: chat.blockedAuthorIDs) {
            button("Block \(message.author.name)", id: "message-action-block") { [weak self, block] in self?.finish(block) }
        } else if BlockedMessages.canUnblock(message.author, viewerID: viewer, blocked: chat.blockedAuthorIDs) {
            button("Unblock \(message.author.name)", id: "message-action-unblock") { [weak self] in chat.unblock(message.author); self?.finish(nil) }
        }
    }
    private func react(_ emoji: String, toggle: Bool) {
        let chat = model.chat, message = current
        guard chat.canInteract, !message.isDeleted, let author = chat.currentAuthor else { return }
        let own = message.reactions?.first { $0.emoji == emoji }?.authorIds.contains(author.id) == true
        finish { Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: !toggle || !own) } }
    }
}

@MainActor final class UIKitNativeReactors: UIKitMessageDialog {
    private let chat: ChatModel
    private let messageID: String
    private let viewerID: String?
    private var selected: String
    private var requested: String?
    init(chat: ChatModel, messageID: String, emoji: String, viewerID: String?, close: @escaping () -> Void) {
        self.chat = chat; self.messageID = messageID; self.viewerID = viewerID; selected = emoji
        super.init(title: "Reactions", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() { super.viewDidLoad(); body.accessibilityIdentifier = "reactors-sheet"; observe() }
    override func render() {
        guard let message = (chat.displayedMessages + chat.displayedThreadMessages + chat.displayedPins).first(where: { $0.id == messageID }),
              !message.isDeleted, let first = message.reactions?.first else { close(); return }
        let reactions = message.reactions ?? []
        if !reactions.contains(where: { $0.emoji == selected }) { selected = first.emoji }
        let key = message.reactionSeq ?? ""
        if requested != key { requested = key; _ = chat.requestReactors(messageID: messageID) }
        body.arrangedSubviews.forEach { body.removeArrangedSubview($0); $0.removeFromSuperview() }
        let tabs = UIStackView(); tabs.spacing = 4
        let scroll = UIScrollView(); scroll.showsHorizontalScrollIndicator = false; scroll.addSubview(tabs)
        tabs.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([tabs.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor),
            tabs.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor), tabs.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            tabs.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor), tabs.heightAnchor.constraint(equalTo: scroll.frameLayoutGuide.heightAnchor),
            scroll.heightAnchor.constraint(equalToConstant: 44)])
        for reaction in reactions {
            let tab = Self.button("\(reaction.emoji) \(reaction.authorIds.count)") { [weak self] in self?.selected = reaction.emoji; self?.render() }
            if let artwork = NativePresentation.emojiImage(reaction.emoji) {
                tab.configuration?.title = String(reaction.authorIds.count)
                tab.configuration?.image = UIImage(cgImage: artwork, scale: CGFloat(artwork.width) / 18, orientation: .up).withRenderingMode(.alwaysOriginal)
                tab.configuration?.imagePadding = 6
            }
            tab.accessibilityLabel = "\(reaction.emoji), \(reaction.authorIds.count) reactions"
            tab.accessibilityIdentifier = "reactors-tab-\(reaction.emoji)"
            if selected == reaction.emoji { tab.accessibilityTraits.insert(.selected); tab.tintColor = UIColor(CaperTheme.terracottaBright) }
            tabs.addArrangedSubview(tab)
        }
        body.addArrangedSubview(scroll)
        let name = Self.label(ReactionSummary.emojiLabel(emojiName: EmojiArtwork.name(for: selected), emoji: selected))
        name.accessibilityIdentifier = "reactors-emoji-name"; body.addArrangedSubview(name)
        switch chat.reactorsState(for: message, emoji: selected, viewerID: viewerID) {
        case .loaded(let people):
            for person in people {
                let avatar = UIKitAvatar(side: 34); avatar.show(name: person.name, avatarID: person.avatarId, picture: person.avatarPicture)
                let words = UIStackView(arrangedSubviews: [Self.label(person.name, size: 14, bold: true)])
                words.axis = .vertical; words.spacing = 2
                if let username = person.username, !username.isEmpty { words.addArrangedSubview(Self.label("@\(username)")) }
                let row = UIStackView(arrangedSubviews: [avatar, words]); row.spacing = 10; row.alignment = .center
                row.accessibilityIdentifier = "reactor-row-\(person.id)"; body.addArrangedSubview(row)
            }
        case .loading:
            let label = Self.label("Loading…"); label.accessibilityIdentifier = "reactors-loading"; body.addArrangedSubview(label)
        case .failed:
            body.addArrangedSubview(Self.label("Couldn’t load reactions"))
            let retry = Self.button("Retry") { [chat, messageID] in _ = chat.requestReactors(messageID: messageID, force: true) }
            retry.accessibilityIdentifier = "reactors-retry"; body.addArrangedSubview(retry)
        }
    }
}

@MainActor final class UIKitNativeMessageHistory: UIKitMessageDialog {
    private let chat: ChatModel
    private let message: ChatMessage
    private var versions: [MessageVersion] = []
    private var selected = 0
    private var loadedRevision: Int?
    private var loading = false
    private var more = false
    private var error: String?
    private var request = 0
    init(chat: ChatModel, message: ChatMessage, close: @escaping () -> Void) {
        self.chat = chat; self.message = message; super.init(title: "Message history", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() { super.viewDidLoad(); observe() }
    override func stop() { super.stop(); request += 1 }
    override func render() {
        if chat.isDeleted(message.id) { close(); return }
        let revision = (chat.messages + chat.pinnedMessages).first { $0.id == message.id }?.revision ?? message.revision ?? 1
        if revision != loadedRevision { loadedRevision = revision; Task { await load(older: false) } }
        body.arrangedSubviews.forEach { body.removeArrangedSubview($0); $0.removeFromSuperview() }
        body.addArrangedSubview(Self.label("\(message.author.name) · Previous versions are retained."))
        if versions.count > 1 { body.addArrangedSubview(comparison(before: versions[1], after: versions[0], current: true)) }
        else if let original = versions.first { body.addArrangedSubview(Self.label("Original version", bold: true)); body.addArrangedSubview(text(original.content.text)) }
        if versions.count > 1 {
            let picker = Self.button(selected == 0 ? "Choose an earlier version…" : "Version \(selected)") {}; picker.showsMenuAsPrimaryAction = true
            picker.accessibilityLabel = "View previous versions"
            picker.menu = UIMenu(children: versions.dropFirst().map { version in
                UIAction(title: Self.versionLabel(version), state: selected == version.revision ? .on : .off) { [weak self] _ in
                    self?.selected = version.revision; self?.render()
                }
            }); body.addArrangedSubview(picker)
            if let version = versions.first(where: { $0.revision == selected }) {
                if let before = versions.first(where: { $0.revision == selected - 1 }) { body.addArrangedSubview(comparison(before: before, after: version, current: false)) }
                else {
                    body.addArrangedSubview(Self.label(version.revision == 1 ? "Original version" : "Version \(version.revision)", bold: true))
                    body.addArrangedSubview(text(version.content.text))
                    if version.revision > 1, more { body.addArrangedSubview(Self.label("Load older versions to compare this change.")) }
                }
            }
        }
        if loading { body.addArrangedSubview(Self.label("Loading versions…")) }
        if let error {
            body.addArrangedSubview(Self.label(error, error: true))
            let retry = Self.button("Retry") { [weak self] in guard let self else { return }; Task { await self.load(older: !self.versions.isEmpty) } }
            retry.isEnabled = !loading; body.addArrangedSubview(retry)
        } else if more {
            let older = Self.button("Load older versions") { [weak self] in Task { await self?.load(older: true) } }
            older.isEnabled = !loading; body.addArrangedSubview(older)
        }
    }
    private func load(older: Bool) async {
        request += 1; let attempt = request; loading = true; error = nil; render()
        do {
            let page = try await chat.messageVersions(message, before: older ? versions.last?.revision : nil)
            guard request == attempt else { return }
            versions = older ? versions + page.versions.filter { item in !versions.contains { $0.revision == item.revision } } : page.versions
            more = page.hasMore; if !older { selected = 0 }
        } catch { if request == attempt { self.error = FriendlyError.message(for: error) } }
        if request == attempt { loading = false; render() }
    }
    private static func versionLabel(_ version: MessageVersion) -> String {
        let date = ChatDateDivider.date(version.createdAt)?.formatted(date: .abbreviated, time: .shortened) ?? version.createdAt
        return "\(version.revision == 1 ? "Original version" : "Version \(version.revision)") · \(date)"
    }
    private func text(_ value: String, tokens: [MessageDiffToken]? = nil, color: UIColor = .clear) -> UITextView {
        let view = UITextView(); view.isEditable = false; view.isScrollEnabled = false; view.backgroundColor = .clear
        view.font = .monospacedSystemFont(ofSize: 12, weight: .regular); view.textColor = UIColor(CaperTheme.text)
        if let tokens {
            let text = NSMutableAttributedString(string: "")
            for token in tokens {
                var attributes: [NSAttributedString.Key: Any] = [.font: view.font!, .foregroundColor: UIColor(CaperTheme.text)]
                if token.changed { attributes[.backgroundColor] = color.withAlphaComponent(0.3) }
                text.append(NSAttributedString(string: token.text, attributes: attributes))
            }; view.attributedText = text
        } else { view.text = value }
        return view
    }
    private func comparison(before: MessageVersion, after: MessageVersion, current: Bool) -> UIView {
        let diff = messageDiff(before: before.content.text, after: after.content.text)
        let columns = UIStackView(); columns.axis = .vertical; columns.spacing = 12
        for (version, title, tokens, color) in [(before, current ? "Previous version" : "Before", diff.0, UIColor(CaperTheme.terracotta)),
                                               (after, current ? "Current version" : "After", diff.1, UIColor(red: 99/255, green: 122/255, blue: 67/255, alpha: 1))] {
            let column = UIStackView(arrangedSubviews: [Self.label(title, size: 14, bold: true), Self.label(Self.versionLabel(version), size: 10), text(version.content.text, tokens: tokens, color: color)])
            column.axis = .vertical; column.spacing = 8; columns.addArrangedSubview(column)
        }
        return columns
    }
}
#endif
