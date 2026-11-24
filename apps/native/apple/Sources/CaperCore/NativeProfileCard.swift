import SwiftUI
import UniformTypeIdentifiers

/// The retained attachment editor can temporarily replace the native card.
struct ProfileComposerPresentationSize: PreferenceKey {
    static let defaultValue: CGSize? = nil
    static func reduce(value: inout CGSize?, nextValue: () -> CGSize?) { value = nextValue() ?? value }
}

private func profilePictureRequest(_ person: MentionCardPerson) -> MediaViewerRequest? {
    guard let picture = person.picture else { return nil }
    let type = UTType(filenameExtension: picture.url.pathExtension)
    let item = ChatAttachment(id: picture.asset, kind: .image, contentType: type?.preferredMIMEType ?? "image/*",
        name: "\(person.title)’s profile picture", size: 0, url: picture.url.absoluteString)
    return MediaViewerRequest(items: [item], start: 0, chat: nil, hideSpoiler: { _ in })
}

#if os(macOS)
import AppKit

@MainActor final class AppKitProfileCard: NSViewController {
    private let state: NativeProfileActions
    private let close: () -> Void
    private let requestBlock: (BlockTarget) -> Void
    private let dismissChanged: (ProfileDismissAction?) -> Void
    private let observation = AppKitObservation()
    private let body = AppKitKit.stack([], spacing: 12)
    private let choices = AppKitKit.stack([], spacing: 6)
    private var choiceButtons: [String: AppKitKitButton] = [:]
    private var spaceIDs: [String] = []
    private let status = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var more: AppKitKitButton!
    private var message: AppKitKitButton!
    private var done: AppKitKitButton!
    private var composer: AppKitQuickComposer?
    private var overlay: NSViewController?
    private var composerSize: CGSize?
    private var chrome: [NSView] = []

    init(model: AppModel, person: MentionCardPerson, requestBlock: @escaping (BlockTarget) -> Void,
         close: @escaping () -> Void, dismissChanged: @escaping (ProfileDismissAction?) -> Void) {
        state = NativeProfileActions(model: model, person: person, close: close)
        self.close = close; self.requestBlock = requestBlock; self.dismissChanged = dismissChanged
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: state.person.isSelf ? 280 : 320, height: 240))
        view.wantsLayer = true; view.layer?.backgroundColor = AppKitKit.surface.cgColor
        view.setAccessibilityElement(true); view.setAccessibilityRole(.group); view.setAccessibilityIdentifier("mention-card")
        let person = state.person
        let avatar = AppKitAvatar(); avatar.frame = NSRect(x: 0, y: 0, width: 48, height: 48)
        avatar.update(name: person.avatarName, avatarId: person.avatarId, picture: person.picture)
        let picture = NSView(); picture.translatesAutoresizingMaskIntoConstraints = false; picture.addSubview(avatar)
        NSLayoutConstraint.activate([picture.widthAnchor.constraint(equalToConstant: 48), picture.heightAnchor.constraint(equalToConstant: 48)])
        if person.picture != nil {
            let open = icon("", label: "Open \(person.title)’s profile picture", id: "mention-card-picture") { [weak self] in self?.openPicture() }
            open.frame = avatar.frame; open.translatesAutoresizingMaskIntoConstraints = true
            let menu = NSMenu()
            for (title, action) in [("Open full-size picture", #selector(openPicture)), ("Copy image", #selector(copyPicture)), ("Copy image link", #selector(copyPictureLink)), ("Open in browser", #selector(openPictureURL))] {
                let item = NSMenuItem(title: title, action: action, keyEquivalent: ""); item.target = self; menu.addItem(item)
            }
            open.menu = menu; picture.addSubview(open)
        }
        let title = AppKitKit.label(person.title, size: 15, weight: .bold, wraps: false)
        title.setAccessibilityIdentifier("mention-card-title")
        let subtitle = AppKitKit.label(person.subtitle ?? "", size: 13, color: AppKitKit.muted, wraps: false)
        subtitle.setAccessibilityIdentifier("mention-card-username"); subtitle.isHidden = person.subtitle == nil
        let identity = AppKitKit.stack([title, subtitle], spacing: 2)
        more = icon("ellipsis", label: "More profile actions", id: "mention-card-more") { [weak self] in self?.showMore() }
        message = icon("bubble.left", label: "Open conversation", id: "mention-card-message") { [state] in Task { await state.openConversation() } }
        done = icon("xmark", label: "Close profile", id: "mention-card-close") { [weak self] in
            guard let self, !self.state.draft.preventsImplicitDismissal else { return }; close()
        }
        let header = NSStackView(views: [picture, identity, more, message, done]); header.orientation = .horizontal; header.spacing = 6
        let you = AppKitKit.label("You", size: 13, color: AppKitKit.muted); you.isHidden = !person.isSelf
        you.setAccessibilityIdentifier("mention-card-you")
        let edit = AppKitKitButton("Edit profile", kind: .primary) { [weak self] in self?.editProfile() }
        edit.setAccessibilityIdentifier("mention-card-edit-profile"); edit.isHidden = !person.isSelf || state.model.account?.id != person.id
        status.setAccessibilityIdentifier("mention-card-status"); error.setAccessibilityIdentifier("mention-card-error")
        choices.setAccessibilityLabel("Spaces you own")
        chrome = [header, you, edit, choices, status, error]
        for row in chrome { body.addArrangedSubview(row); AppKitKit.fillWidth(row, of: body) }
        let scroll = appKitKitScroll(body, insets: NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16))
        view.addSubview(scroll)
        // A wrapping draft grows the body inside the scroll view without
        // laying out the card, so the popover would keep its old height.
        body.postsFrameChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(bodyResized), name: NSView.frameDidChangeNotification, object: body)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor), scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: view.topAnchor), scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
    }
    override func viewDidAppear() {
        super.viewDidAppear()
        observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
    }
    override func viewDidDisappear() { super.viewDidDisappear(); observation.stop() }
    @objc private func bodyResized() { view.needsLayout = true }
    override func viewDidLayout() {
        super.viewDidLayout()
        guard overlay == nil else { return }
        let next = composerSize.map { CGSize(width: $0.width + 32, height: $0.height + 32) }
            ?? CGSize(width: state.person.isSelf ? 280 : 320, height: min(620, max(140, body.fittingSize.height + 32)))
        if preferredContentSize != next { preferredContentSize = next }
    }
    private func icon(_ symbol: String, label: String, id: String, action: @escaping () -> Void) -> AppKitKitButton {
        let button = AppKitKitButton("", kind: .plain, action: action)
        if !symbol.isEmpty {
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
            button.imagePosition = .imageOnly; button.contentTintColor = AppKitKit.muted
            NSLayoutConstraint.activate([button.widthAnchor.constraint(equalToConstant: 28), button.heightAnchor.constraint(equalToConstant: 32)])
        }
        button.setAccessibilityLabel(label); button.setAccessibilityIdentifier(id); button.toolTip = label
        return button
    }
    private func render() {
        guard !state.draft.discarded else { close(); return }
        let expanded = composerSize != nil
        more.isHidden = expanded || (!state.canInvite && !state.canBlock); more.isEnabled = !state.busy
        message.isHidden = expanded || !state.canMessage; message.isEnabled = !state.busy
        message.setAccessibilityLabel(state.pending == "message" ? "Opening…" : "Open conversation")
        done.isEnabled = !state.draft.preventsImplicitDismissal
        chrome.first?.isHidden = expanded
        status.stringValue = state.status ?? ""; status.isHidden = expanded || state.status == nil
        error.stringValue = state.error ?? ""; error.isHidden = expanded || state.error == nil
        choices.isHidden = expanded || !state.choosingSpace || !state.canInvite
        let spaces = state.ownedSpaces
        if spaceIDs != spaces.map(\.id) {
            choices.arrangedSubviews.forEach { $0.removeFromSuperview() }; choiceButtons = [:]; spaceIDs = spaces.map(\.id)
            for space in spaces {
                let button = AppKitKitButton(space.name) { [state] in Task { await state.invite(space) } }
                choices.addArrangedSubview(button); AppKitKit.fillWidth(button, of: choices); choiceButtons[space.id] = button
            }
        }
        for space in spaces {
            choiceButtons[space.id]?.title = state.pending == space.id ? "Sending…" : space.name + (state.invited[space.id].map { " · \($0)" } ?? "")
            choiceButtons[space.id]?.isEnabled = !state.busy && state.invited[space.id] == nil
        }
        if state.canMessage {
            if composer == nil {
                let host = AppKitQuickComposer(state: NativeQuickDraft(model: state.model, person: state.person, close: close))
                body.addArrangedSubview(host); AppKitKit.fillWidth(host, of: body); composer = host
            }
            composer?.state.enabled = state.pending == nil
        } else { composer?.stop(); composer?.removeFromSuperview(); composer = nil }
        view.needsLayout = true
    }
    private func showMore() {
        guard !state.busy else { return }
        let menu = NSMenu()
        if state.canInvite {
            let item = NSMenuItem(title: "Invite to space", action: #selector(chooseSpace), keyEquivalent: "")
            item.target = self; item.setAccessibilityIdentifier("mention-card-invite"); menu.addItem(item)
        }
        if state.canBlock {
            let item = NSMenuItem(title: state.blocked ? "Unblock" : "Block", action: #selector(block), keyEquivalent: "")
            item.target = self; item.setAccessibilityIdentifier("mention-card-block"); item.caperRole = state.blocked ? nil : .destructive; menu.addItem(item)
        }
        CaperMenu.show(menu, below: more)
    }
    @objc private func chooseSpace() { if !state.busy { state.choosingSpace.toggle() } }
    @objc private func block() {
        guard !state.busy, state.canBlock else { return }
        if state.blocked { Task { await state.unblock() } }
        else { let target = state.blockTarget; close(); requestBlock(target) }
    }
    private func editProfile() {
        let controller = AppKitProfilePresentation(model: state.model, close: close)
        showOverlay(controller, size: CGSize(width: 520, height: 620))
        dismissChanged(ProfileDismissAction(id: UUID(), action: { [weak controller] in controller?.requestClose() }))
    }
    @objc private func openPicture() {
        guard let request = profilePictureRequest(state.person) else { return }
        showOverlay(AppKitMediaViewer(request: request) { [weak self] in self?.closeOverlay() }, size: CGSize(width: 720, height: 600))
        dismissChanged(ProfileDismissAction(id: UUID(), action: { [weak self] in self?.closeOverlay() }))
    }
    private func showOverlay(_ host: NSViewController, size: CGSize) {
        guard overlay == nil else { return }
        overlay = host; addChild(host)
        view.subviews.forEach { $0.isHidden = true }
        host.view.frame = view.bounds; host.view.autoresizingMask = [.width, .height]; view.addSubview(host.view)
        preferredContentSize = size
    }
    private func closeOverlay() {
        overlay?.view.removeFromSuperview(); overlay?.removeFromParent(); overlay = nil
        view.subviews.forEach { $0.isHidden = false }; dismissChanged(nil); view.needsLayout = true
    }
    @objc private func openPictureURL() { if let url = state.person.picture?.url { NSWorkspace.shared.open(url) } }
    @objc private func copyPictureLink() {
        guard let url = state.person.picture?.url else { return }
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(url.absoluteString, forType: .string)
    }
    @objc private func copyPicture() {
        guard let url = state.person.picture?.url else { return }
        Task { [weak self] in
            if (try? await copyMediaImage(url)) != true, let window = self?.view.window {
                let alert = NSAlert(); alert.messageText = "Couldn’t copy image. Try opening it and saving it instead."
                alert.addButton(withTitle: "OK"); alert.beginSheetModal(for: window, completionHandler: nil)
            }
        }
    }
}
#elseif os(iOS)
import UIKit
import Observation

@MainActor final class UIKitProfileCard: UIViewController, UIAdaptivePresentationControllerDelegate, UIContextMenuInteractionDelegate {
    private let model: AppModel
    private let person: MentionCardPerson
    private let requestBlock: (BlockTarget) -> Void
    private lazy var state = NativeProfileActions(model: model, person: person) { [weak self] in self?.dismiss(animated: true) }
    private let body = UIStackView()
    private let choices = UIStackView()
    private var choiceButtons: [String: UIButton] = [:]
    private var spaceIDs: [String] = []
    private let status = UIKitMessageDialog.label("")
    private let error = UIKitMessageDialog.label("", error: true)
    private var more: UIButton!
    private var message: UIButton!
    private var done: UIButton!
    private var composer: UIKitQuickComposer?
    private var editor: UIKitProfilePresentation?
    private var dismissAction: ProfileDismissAction?
    private var active = false

    init(model: AppModel, person: MentionCardPerson, requestBlock: @escaping (BlockTarget) -> Void) {
        self.model = model; self.person = person; self.requestBlock = requestBlock
        super.init(nibName: nil, bundle: nil)
        // iPad shows the card at its content size, like the Mac popover; a
        // form sheet becomes the phone's sheet in compact width.
        modalPresentationStyle = UIDevice.current.userInterfaceIdiom == .pad ? .formSheet : .pageSheet
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIColor(CaperTheme.surface); overrideUserInterfaceStyle = CaperTheme.interfaceStyle
        view.accessibilityIdentifier = "mention-card"
        let avatar = UIKitAvatar(side: 48); avatar.show(name: person.avatarName, avatarID: person.avatarId, picture: person.picture)
        let picture = UIButton(type: .custom); picture.addSubview(avatar); avatar.isUserInteractionEnabled = false
        NSLayoutConstraint.activate([
            avatar.leadingAnchor.constraint(equalTo: picture.leadingAnchor), avatar.trailingAnchor.constraint(equalTo: picture.trailingAnchor),
            avatar.topAnchor.constraint(equalTo: picture.topAnchor), avatar.bottomAnchor.constraint(equalTo: picture.bottomAnchor),
        ])
        picture.isAccessibilityElement = person.picture != nil; picture.isUserInteractionEnabled = person.picture != nil
        if person.picture != nil {
            picture.addTarget(self, action: #selector(openPicture), for: .touchUpInside)
            picture.addInteraction(UIContextMenuInteraction(delegate: self))
            picture.accessibilityLabel = "Open \(person.title)’s profile picture"; picture.accessibilityIdentifier = "mention-card-picture"
        }
        let title = UIKitMessageDialog.label(person.title, size: 15, bold: true)
        title.accessibilityIdentifier = "mention-card-title"; title.numberOfLines = 1
        let subtitle = UIKitMessageDialog.label(person.subtitle ?? "", size: 13)
        subtitle.accessibilityIdentifier = "mention-card-username"; subtitle.isHidden = person.subtitle == nil; subtitle.numberOfLines = 1
        let identity = UIStackView(arrangedSubviews: [title, subtitle]); identity.axis = .vertical; identity.spacing = 2
        more = icon("ellipsis", label: "More profile actions", id: "mention-card-more", action: {})
        more.showsMenuAsPrimaryAction = true
        message = icon("bubble.left", label: "Open conversation", id: "mention-card-message") { [weak self] in
            guard let self else { return }; Task { await self.state.openConversation() }
        }
        done = icon("xmark", label: "Close profile", id: "mention-card-close") { [weak self] in
            guard let self, !self.state.draft.preventsImplicitDismissal else { return }; self.dismiss(animated: true)
        }
        let header = UIStackView(arrangedSubviews: [picture, identity, more, message, done]); header.spacing = 6; header.alignment = .center
        let you = UIKitMessageDialog.label("You"); you.accessibilityIdentifier = "mention-card-you"; you.isHidden = !person.isSelf
        let edit = UIKitMessageDialog.button("Edit profile", primary: true) { [weak self] in self?.editProfile() }
        edit.accessibilityIdentifier = "mention-card-edit-profile"; edit.isHidden = !person.isSelf || model.account?.id != person.id
        choices.axis = .vertical; choices.spacing = 6; choices.accessibilityLabel = "Spaces you own"
        status.accessibilityIdentifier = "mention-card-status"; error.accessibilityIdentifier = "mention-card-error"
        body.axis = .vertical; body.spacing = 12
        [header, you, edit, choices, status, error].forEach { body.addArrangedSubview($0) }
        let scroll = UIScrollView(); scroll.keyboardDismissMode = .interactive
        scroll.translatesAutoresizingMaskIntoConstraints = false; body.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scroll); scroll.addSubview(body)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
            body.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor, constant: 16),
            body.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor, constant: -16),
            body.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 16),
            body.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -16),
            body.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -32),
        ])
        // The phone sheet is as tall as the card (a tap above it closes it)
        // and rides up above the keyboard at that height; a longer card
        // scrolls. Only one detent, so focusing the field never jumps it to
        // full height. The profile editor takes the full height.
        if let sheet = sheetPresentationController {
            sheet.detents = [.custom(identifier: .init("caper.profile")) { [weak self] context in
                let maximum = context.maximumDetentValue
                return MainActor.assumeIsolated {
                    guard let self, self.editor == nil else { return maximum }
                    return min(self.cardHeight(width: self.view.bounds.width > 0 ? self.view.bounds.width : self.view.window?.bounds.width ?? 390), maximum)
                }
            }]
            sheet.prefersGrabberVisible = true
        }
        // The composer is in place before the sheet first measures, so it
        // opens once at its final height instead of growing in a second step.
        render()
    }
    private var sheetHeight: CGFloat = 0
    private func cardHeight(width: CGFloat) -> CGFloat {
        ceil(body.systemLayoutSizeFitting(CGSize(width: width - 32, height: 0), withHorizontalFittingPriority: .required,
                                          verticalFittingPriority: .fittingSizeLevel).height) + 32 + view.safeAreaInsets.top
    }
    override func viewDidLayoutSubviews() { super.viewDidLayoutSubviews(); fitToContent() }
    /// The form sheet's size: 360pt wide and as tall as the card, or the
    /// profile editor's size while it is open. The phone sheet ignores it.
    private func fitToContent() {
        guard modalPresentationStyle == .formSheet else {
            // Staged files, a growing draft or the space list resize the phone sheet.
            guard let sheet = sheetPresentationController, editor == nil, view.bounds.width > 0 else { return }
            let height = cardHeight(width: view.bounds.width)
            guard abs(height - sheetHeight) > 0.5 else { return }
            let first = sheetHeight == 0; sheetHeight = height
            if first || CaperAccessibility.reduceMotion || view.window == nil { sheet.invalidateDetents() }
            else { view.layoutIfNeeded(); sheet.animateChanges { sheet.invalidateDetents() } }
            return
        }
        let width: CGFloat = 360
        let next = editor != nil ? CGSize(width: 540, height: 620) : CGSize(width: width, height: min(620, max(160, ceil(
            body.systemLayoutSizeFitting(CGSize(width: width - 32, height: 0), withHorizontalFittingPriority: .required,
                                         verticalFittingPriority: .fittingSizeLevel).height) + 32)))
        if preferredContentSize != next { preferredContentSize = next }
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated); presentationController?.delegate = self
        if !active { active = true; observe() }
    }
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if isBeingDismissed || presentingViewController == nil { active = false; CaperFocus.presentationDidClose() }
    }
    func presentationControllerShouldDismiss(_ presentationController: UIPresentationController) -> Bool {
        if let dismissAction { dismissAction.action(); return false }
        return !state.draft.preventsImplicitDismissal
    }
    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) { active = false }
    private func icon(_ symbol: String, label: String, id: String, action: @escaping () -> Void) -> UIButton {
        let button = UIButton(type: .system); button.setImage(UIImage(systemName: symbol), for: .normal)
        button.tintColor = UIColor(CaperTheme.muted); button.accessibilityLabel = label; button.accessibilityIdentifier = id
        button.widthAnchor.constraint(equalToConstant: 44).isActive = true; button.heightAnchor.constraint(equalToConstant: 44).isActive = true
        button.addAction(UIAction { _ in action() }, for: .touchUpInside); return button
    }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe() } }
    }
    private func render() {
        guard !state.draft.discarded else { dismiss(animated: false); return }
        more.isHidden = !state.canInvite && !state.canBlock; more.isEnabled = !state.busy
        message.isHidden = !state.canMessage; message.isEnabled = !state.busy
        message.accessibilityLabel = state.pending == "message" ? "Opening…" : "Open conversation"
        done.isEnabled = !state.draft.preventsImplicitDismissal
        var actions: [UIAction] = []
        if state.canInvite {
            let invite = UIAction(title: "Invite to space") { [weak self] _ in self?.state.choosingSpace.toggle() }
            invite.accessibilityIdentifier = "mention-card-invite"; actions.append(invite)
        }
        if state.canBlock {
            let block = UIAction(title: state.blocked ? "Unblock" : "Block") { [weak self] _ in self?.block() }
            block.accessibilityIdentifier = "mention-card-block"; actions.append(block)
        }
        more.menu = UIMenu(children: actions)
        status.text = state.status; status.isHidden = state.status == nil
        error.text = state.error; error.isHidden = state.error == nil
        choices.isHidden = !state.choosingSpace || !state.canInvite
        let spaces = state.ownedSpaces
        if spaceIDs != spaces.map(\.id) {
            choices.arrangedSubviews.forEach { $0.removeFromSuperview() }; choiceButtons = [:]; spaceIDs = spaces.map(\.id)
            for space in spaces {
                let button = UIKitMessageDialog.button(space.name) { [state] in Task { await state.invite(space) } }
                choices.addArrangedSubview(button); choiceButtons[space.id] = button
            }
        }
        for space in spaces {
            choiceButtons[space.id]?.configuration?.title = state.pending == space.id ? "Sending…" : space.name + (state.invited[space.id].map { " · \($0)" } ?? "")
            choiceButtons[space.id]?.isEnabled = !state.busy && state.invited[space.id] == nil
        }
        if state.canMessage {
            if composer == nil {
                let host = UIKitQuickComposer(state: NativeQuickDraft(model: model, person: person, close: { [weak self] in self?.dismiss(animated: true) }))
                addChild(host); body.addArrangedSubview(host.view); host.didMove(toParent: self); composer = host
                // A growing draft lays out only inside the scroll view, so remeasure the sheet here.
                host.heightChanged = { [weak self] in self?.view.setNeedsLayout() }
            }
            composer?.state.enabled = state.pending == nil
        } else if let composer {
            composer.stop(); composer.willMove(toParent: nil); composer.view.removeFromSuperview(); composer.removeFromParent(); self.composer = nil
        }
        fitToContent()
    }
    private func block() {
        guard !state.busy, state.canBlock else { return }
        if state.blocked { Task { await state.unblock() } }
        else { let target = state.blockTarget; dismiss(animated: true) { [requestBlock] in requestBlock(target) } }
    }
    private func editProfile() {
        guard editor == nil else { return }
        let host = UIKitProfilePresentation(model: model, close: { [weak self] in self?.dismiss(animated: true) })
        dismissAction = ProfileDismissAction(id: UUID(), action: { [weak host] in host?.requestClose() })
        editor = host; addChild(host)
        view.subviews.forEach { $0.isHidden = true }; host.view.frame = view.bounds; host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(host.view); host.didMove(toParent: self); fitToContent()
        if let sheet = sheetPresentationController { sheet.animateChanges { sheet.invalidateDetents() } }
    }
    @objc private func openPicture() {
        guard let request = profilePictureRequest(person) else { return }
        let host = UIKitMediaViewer(request: request) { [weak self] in self?.presentedViewController?.dismiss(animated: false) }
        present(host, animated: true)
    }
    func contextMenuInteraction(_ interaction: UIContextMenuInteraction, configurationForMenuAtLocation location: CGPoint) -> UIContextMenuConfiguration? {
        guard let picture = person.picture else { return nil }
        return UIContextMenuConfiguration(identifier: nil, previewProvider: nil) { [weak self] _ in
            UIMenu(children: [UIAction(title: "Open full-size picture") { _ in self?.openPicture() },
                UIAction(title: "Copy image") { _ in self?.copyPicture(picture.url) },
                UIAction(title: "Copy image link") { _ in UIPasteboard.general.url = picture.url },
                UIAction(title: "Open in browser") { _ in UIApplication.shared.open(picture.url) }])
        }
    }
    private func copyPicture(_ url: URL) {
        Task { [weak self] in
            if (try? await copyMediaImage(url)) != true, let self {
                let alert = UIAlertController(title: "Couldn’t copy image. Try opening it and saving it instead.", message: nil, preferredStyle: .alert)
                alert.addAction(UIAlertAction(title: "OK", style: .default)); self.present(alert, animated: true)
            }
        }
    }
}
#endif
