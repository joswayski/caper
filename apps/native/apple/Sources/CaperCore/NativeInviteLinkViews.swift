#if os(macOS)
import AppKit

@MainActor final class AppKitInviteLinks: NSStackView, NSTextFieldDelegate {
    private let state: NativeInviteLinks
    private let observation = AppKitObservation()
    private let primary = AppKitKit.stack([], spacing: 8), custom = AppKitKit.stack([], spacing: 10), links = AppKitKit.stack([], spacing: 8)
    private let expiry = NSTextField(), uses = NSTextField(), unit = NSPopUpButton(), channel = NSPopUpButton()
    private let expiryError = AppKitKit.label("", size: 12, color: AppKitKit.error), usesError = AppKitKit.label("", size: 12, color: AppKitKit.error), error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var create: AppKitKitButton!
    private var started = false
    private var lastLinks: [InviteLink]?, lastDefault: InviteLink?
    private var lastBusy: Bool?, lastCopied: String?, lastDeactivated: Bool?
    private var timer: Timer?
    init(state: NativeInviteLinks) {
        self.state = state; super.init(frame: .zero); orientation = .vertical; spacing = 12; alignment = .leading
        if state.showsDefault {
            add(AppKitKit.label("Invite link", size: 14, weight: .bold))
            add(AppKitKit.label("Anyone with this link can join. Up to 100 total joins, valid for 30 days from creation.", size: 12, color: AppKitKit.muted)); add(primary)
        }
        let toggle = AppKitKitButton("Custom links") { [state] in state.expanded.toggle() }; toggle.setAccessibilityIdentifier("space-custom-links"); add(toggle)
        add(custom)
        for (field, value, placeholder, label, id) in [(expiry, state.expiryNumber, "30", "Expire after", "invite-link-expiry-number"), (uses, state.maxUses, "No limit", "Max uses", "invite-link-max-uses")] {
            field.stringValue = value; field.placeholderString = placeholder; field.font = AppKitKit.font(13); field.delegate = self
            field.heightAnchor.constraint(greaterThanOrEqualToConstant: 36).isActive = true; field.setAccessibilityLabel(label); field.setAccessibilityIdentifier(id)
        }
        unit.addItems(withTitles: InviteExpiryUnit.allCases.map(\.label)); unit.selectItem(at: InviteExpiryUnit.allCases.firstIndex(of: state.expiryUnit) ?? 0)
        unit.target = self; unit.action = #selector(unitChanged); unit.setAccessibilityIdentifier("invite-link-expiry-unit"); unit.setAccessibilityLabel("Expiry unit")
        channel.target = self; channel.action = #selector(channelChanged); channel.setAccessibilityLabel("Opens in")
        create = AppKitKitButton("Create link") { [state] in state.create() }; create.setAccessibilityIdentifier("invite-link-create")
        expiryError.setAccessibilityIdentifier("invite-link-expiry-error"); usesError.setAccessibilityIdentifier("invite-link-max-uses-error")
        for row in [AppKitKit.label("Create a separate link", size: 14, weight: .bold), AppKitKit.label("Choose different limits or a landing channel. Existing links keep their settings.", size: 12, color: AppKitKit.muted),
            AppKitKit.label("Expire after", size: 12, weight: .bold), expiry, unit, expiryError, AppKitKit.label("Max uses", size: 12, weight: .bold), uses, usesError,
            AppKitKit.label("Opens in", size: 12, weight: .bold), channel, create!, links] as [NSView] { custom.addArrangedSubview(row); AppKitKit.fillWidth(row, of: custom) }
        add(error)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { timer?.invalidate() }
    private func add(_ view: NSView) { addArrangedSubview(view); AppKitKit.fillWidth(view, of: self) }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil {
            observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
            if !started { started = true; Task { [state] in await state.load() } }
            timer?.invalidate(); timer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.lastLinks = nil; self?.render() } }
        } else { observation.stop(); timer?.invalidate(); timer = nil }
    }
    func controlTextDidChange(_ notification: Notification) {
        if notification.object as? NSTextField === expiry { state.expiryNumber = expiry.stringValue; state.expiryError = nil }
        else { state.maxUses = uses.stringValue; state.maxUsesError = nil }
    }
    @objc private func unitChanged() { state.expiryUnit = InviteExpiryUnit.allCases[unit.indexOfSelectedItem]; state.expiryError = nil }
    @objc private func channelChanged() { state.channelID = state.publicChannels.indices.contains(channel.indexOfSelectedItem) ? state.publicChannels[channel.indexOfSelectedItem].id : nil }
    private func render() {
        custom.isHidden = !state.expanded; expiry.isHidden = state.expiryUnit == .never
        [expiry, uses, unit, channel].forEach { $0.isEnabled = !state.busy }
        let channels = state.publicChannels, names = channels.isEmpty ? ["No public channels"] : channels.map { "#\($0.name)" }
        if channel.itemTitles != names { channel.removeAllItems(); channel.addItems(withTitles: names) }
        channel.selectItem(at: channels.firstIndex { $0.id == state.channelID } ?? 0); channel.isEnabled = !state.busy && !channels.isEmpty
        create.title = state.pending == "create" ? "Creating…" : "Create link"; create.isEnabled = !state.busy
        expiryError.stringValue = state.expiryError ?? ""; expiryError.isHidden = state.expiryError == nil
        usesError.stringValue = state.maxUsesError ?? ""; usesError.isHidden = state.maxUsesError == nil
        error.stringValue = state.error ?? ""; error.isHidden = state.error == nil
        guard lastLinks != state.links || lastDefault != state.defaultLink || lastBusy != state.busy || lastCopied != state.copied || lastDeactivated != state.deactivated else { return }
        lastLinks = state.links; lastDefault = state.defaultLink; lastBusy = state.busy; lastCopied = state.copied; lastDeactivated = state.deactivated
        for stack in [primary, links] { stack.arrangedSubviews.forEach { $0.removeFromSuperview() } }
        if let link = state.defaultLink { append(row(link, main: true), to: primary) }
        else if state.busy { append(AppKitKit.label("Preparing invite link…", size: 12), to: primary) }
        else {
            if state.deactivated { append(AppKitKit.label("This link is deactivated. Opening Invite people again prepares a new link, or you can create another now.", size: 12, color: AppKitKit.muted), to: primary) }
            let prepare = AppKitKitButton(state.deactivated ? "Create another link" : "Prepare link", kind: .primary) { [state] in state.prepare() }
            prepare.setAccessibilityIdentifier("default-invite-link-create"); append(prepare, to: primary)
        }
        if state.loading, state.links.isEmpty { append(AppKitKit.label("Loading invite links…", size: 12), to: links) }
        for link in state.links { append(row(link, main: false), to: links) }
    }
    private func append(_ row: NSView, to stack: NSStackView) { stack.addArrangedSubview(row); AppKitKit.fillWidth(row, of: stack) }
    private func row(_ link: InviteLink, main: Bool) -> NSView {
        let url = AppKitKit.label(state.model.inviteLinkURL(link.code), size: 12); url.isSelectable = true
        let meta = AppKitKit.label(InviteLinks.meta(for: link, now: Date()), size: 11, color: AppKitKit.muted)
        if main { meta.setAccessibilityIdentifier("default-invite-link-summary") }
        let copied = state.copied == link.code
        let copy = AppKitKitButton(copied ? "Copied" : main ? "Copy link" : "Copy", kind: main ? .primary : .secondary) { [state] in state.copy(link, briefly: !main) }
        copy.setAccessibilityLabel(copied ? "Copied" : main ? "Copy link" : "Copy invite link")
        if main { copy.setAccessibilityIdentifier("default-invite-link-copy") }
        copy.isEnabled = !main || !state.busy
        let actions = NSStackView(views: [copy]); actions.spacing = 8
        if main {
            let options = AppKitKitButton("…") { [weak self] in
                guard let self else { return }
                let menu = NSMenu()
                for (title, identifier, action) in [
                    ("Regenerate link", "default-invite-link-regenerate", { [weak self] in self?.confirm(link, regenerate: true) }),
                    ("Custom links", "", { [state] in state.expanded = true }),
                    ("Deactivate link", "default-invite-link-deactivate", { [weak self] in self?.confirm(link, regenerate: false) })
                ] as [(String, String, () -> Void)] {
                    let item = AppKitMenuAction(title, action: action); item.setAccessibilityIdentifier(identifier); menu.addItem(item)
                }
                menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
            }
            options.setAccessibilityLabel("More invite options"); options.setAccessibilityIdentifier("default-invite-link-options"); options.isEnabled = !state.busy; actions.addArrangedSubview(options)
        } else {
            let revoke = AppKitKitButton(state.pending == link.code ? "Revoking…" : "Revoke", kind: .destructive) { [weak self] in self?.confirm(link, regenerate: false) }; revoke.isEnabled = !state.busy; actions.addArrangedSubview(revoke)
        }
        let row = AppKitKit.stack([url, meta, actions], spacing: 6); row.setAccessibilityIdentifier("invite-link-\(link.code)"); return row
    }
    private func confirm(_ link: InviteLink, regenerate: Bool) {
        guard !state.busy, let window else { return }
        let action = regenerate ? "Regenerate link" : state.defaultLink?.code == link.code ? "Deactivate link" : "Revoke"
        let alert = NSAlert(); alert.messageText = action + "?"; alert.informativeText = "The previous URL stops admitting new members." + (regenerate ? " The new link resets to 30 days and 100 joins." : "")
        alert.addButton(withTitle: "Cancel"); alert.addButton(withTitle: action)
        alert.beginSheetModal(for: window) { [state] response in if response == .alertSecondButtonReturn { if regenerate { state.regenerate(link) } else { state.revoke(link) } } }
    }
}

@MainActor private final class AppKitMenuAction: NSMenuItem {
    private let handler: () -> Void
    init(_ title: String, action: @escaping () -> Void) {
        handler = action; super.init(title: title, action: #selector(fire), keyEquivalent: ""); target = self
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func fire() { handler() }
}
#elseif os(iOS)
import UIKit
import Observation

@MainActor final class UIKitInviteLinks: UIStackView {
    private let state: NativeInviteLinks
    private let present: (UIViewController) -> Void
    private let primary = UIStackView(), custom = UIStackView(), links = UIStackView()
    private let expiry = UIKitMessageDialog.workspaceField("30", label: "Expire after"), uses = UIKitMessageDialog.workspaceField("No limit", label: "Max uses")
    private let expiryError = UIKitMessageDialog.label("", error: true), usesError = UIKitMessageDialog.label("", error: true), error = UIKitMessageDialog.label("", error: true)
    private var unit: UIButton!, channel: UIButton!, create: UIButton!
    private var generation = 0
    private var started = false
    private var lastLinks: [InviteLink]?, lastDefault: InviteLink?, lastBusy: Bool?, lastCopied: String?, lastDeactivated: Bool?
    private var timer: Timer?
    init(state: NativeInviteLinks, present: @escaping (UIViewController) -> Void) {
        self.state = state; self.present = present; super.init(frame: .zero); axis = .vertical; spacing = 12
        for stack in [primary, custom, links] { stack.axis = .vertical; stack.spacing = 10 }
        if state.showsDefault {
            [UIKitMessageDialog.label("Invite link", size: 14, bold: true), UIKitMessageDialog.label("Anyone with this link can join. Up to 100 total joins, valid for 30 days from creation."), primary].forEach(addArrangedSubview)
        }
        let toggle = UIKitMessageDialog.button("Custom links") { [state] in state.expanded.toggle() }; toggle.accessibilityIdentifier = "space-custom-links"; addArrangedSubview(toggle); addArrangedSubview(custom)
        expiry.text = state.expiryNumber; uses.text = state.maxUses
        expiry.keyboardType = .numberPad; uses.keyboardType = .numberPad
        expiry.accessibilityIdentifier = "invite-link-expiry-number"; uses.accessibilityIdentifier = "invite-link-max-uses"
        expiry.addAction(UIAction { [weak self] _ in guard let self else { return }; self.state.expiryNumber = self.expiry.text ?? ""; self.state.expiryError = nil }, for: .editingChanged)
        uses.addAction(UIAction { [weak self] _ in guard let self else { return }; self.state.maxUses = self.uses.text ?? ""; self.state.maxUsesError = nil }, for: .editingChanged)
        unit = UIKitMessageDialog.button("Expiry unit") {}; unit.showsMenuAsPrimaryAction = true; unit.accessibilityIdentifier = "invite-link-expiry-unit"
        channel = UIKitMessageDialog.button("Opens in") {}; channel.showsMenuAsPrimaryAction = true
        create = UIKitMessageDialog.button("Create link") { [state] in state.create() }; create.accessibilityIdentifier = "invite-link-create"
        expiryError.accessibilityIdentifier = "invite-link-expiry-error"; usesError.accessibilityIdentifier = "invite-link-max-uses-error"
        [UIKitMessageDialog.label("Create a separate link", size: 14, bold: true), UIKitMessageDialog.label("Choose different limits or a landing channel. Existing links keep their settings."),
            UIKitMessageDialog.label("Expire after", bold: true), expiry, unit!, expiryError, UIKitMessageDialog.label("Max uses", bold: true), uses, usesError,
            UIKitMessageDialog.label("Opens in", bold: true), channel!, create!, links].forEach(custom.addArrangedSubview)
        addArrangedSubview(error)
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { timer?.invalidate() }
    override func didMoveToWindow() {
        super.didMoveToWindow(); generation += 1; timer?.invalidate(); timer = nil
        if window != nil {
            observe(generation); if !started { started = true; Task { [state] in await state.load() } }
            timer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.lastLinks = nil; self?.render() } }
        }
    }
    private func observe(_ token: Int) {
        guard token == generation, window != nil else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
    }
    private func render() {
        custom.isHidden = !state.expanded; expiry.isHidden = state.expiryUnit == .never
        // Disabling the focused field would resign it and drop the keyboard mid-create.
        expiry.isEnabled = !state.busy || expiry.isFirstResponder; uses.isEnabled = !state.busy || uses.isFirstResponder; unit.isEnabled = !state.busy
        unit.configuration?.title = state.expiryUnit.label; unit.accessibilityLabel = "Expiry unit"; unit.accessibilityValue = state.expiryUnit.label
        unit.menu = UIMenu(children: InviteExpiryUnit.allCases.map { value in UIAction(title: value.label, state: value == state.expiryUnit ? .on : .off) { [state] _ in state.expiryUnit = value; state.expiryError = nil } })
        let channels = state.publicChannels
        channel.configuration?.title = channels.first { $0.id == state.channelID }.map { "#\($0.name)" } ?? channels.first.map { "#\($0.name)" } ?? "No public channels"
        channel.isEnabled = !state.busy && !channels.isEmpty; channel.accessibilityLabel = "Opens in"; channel.accessibilityValue = channel.configuration?.title
        channel.menu = UIMenu(children: channels.map { value in UIAction(title: "#\(value.name)", state: value.id == state.channelID ? .on : .off) { [state] _ in state.channelID = value.id } })
        create.configuration?.title = state.pending == "create" ? "Creating…" : "Create link"; create.isEnabled = !state.busy
        expiryError.text = state.expiryError; expiryError.isHidden = state.expiryError == nil; usesError.text = state.maxUsesError; usesError.isHidden = state.maxUsesError == nil
        error.text = state.error; error.isHidden = state.error == nil
        if lastLinks != state.links || lastDefault != state.defaultLink || lastBusy != state.busy || lastCopied != state.copied || lastDeactivated != state.deactivated {
            lastLinks = state.links; lastDefault = state.defaultLink; lastBusy = state.busy; lastCopied = state.copied; lastDeactivated = state.deactivated
            for stack in [primary, links] { stack.arrangedSubviews.forEach { stack.removeArrangedSubview($0); $0.removeFromSuperview() } }
            if let link = state.defaultLink { primary.addArrangedSubview(row(link, main: true)) }
            else if state.busy { primary.addArrangedSubview(UIKitMessageDialog.label("Preparing invite link…")) }
            else {
                if state.deactivated { primary.addArrangedSubview(UIKitMessageDialog.label("This link is deactivated. Opening Invite people again prepares a new link, or you can create another now.")) }
                let prepare = UIKitMessageDialog.button(state.deactivated ? "Create another link" : "Prepare link", primary: true) { [state] in state.prepare() }
                prepare.accessibilityIdentifier = "default-invite-link-create"; primary.addArrangedSubview(prepare)
            }
            if state.loading, state.links.isEmpty { links.addArrangedSubview(UIKitMessageDialog.label("Loading invite links…")) }
            for link in state.links { links.addArrangedSubview(row(link, main: false)) }
        }
        var controller = next
        while let next = controller { if let frame = next as? UIKitWorkspaceFrame { frame.view.setNeedsLayout(); break }; controller = next.next }
    }
    private func row(_ link: InviteLink, main: Bool) -> UIView {
        let url = UITextView(); url.isEditable = false; url.isScrollEnabled = false; url.text = state.model.inviteLinkURL(link.code)
        url.font = .monospacedSystemFont(ofSize: 12, weight: .regular); url.textColor = UIColor(CaperTheme.text); url.backgroundColor = .clear; url.textContainerInset = .zero
        let meta = UIKitMessageDialog.label(InviteLinks.meta(for: link, now: Date()), size: 11); if main { meta.accessibilityIdentifier = "default-invite-link-summary" }
        let copied = state.copied == link.code
        let copy = UIKitMessageDialog.button(copied ? "Copied" : main ? "Copy link" : "Copy", primary: main) { [state] in state.copy(link, briefly: !main) }
        copy.accessibilityLabel = copied ? "Copied" : main ? "Copy link" : "Copy invite link"; if main { copy.accessibilityIdentifier = "default-invite-link-copy" }
        copy.isEnabled = !main || !state.busy
        let actions = UIStackView(arrangedSubviews: [copy]); actions.spacing = 8
        if main {
            let options = UIKitMessageDialog.button("…") {}; options.accessibilityLabel = "More invite options"; options.accessibilityIdentifier = "default-invite-link-options"
            options.showsMenuAsPrimaryAction = true; options.isEnabled = !state.busy
            options.menu = UIMenu(children: [UIAction(title: "Regenerate link") { [weak self] _ in self?.confirm(link, regenerate: true) },
                UIAction(title: "Custom links") { [state] _ in state.expanded = true }, UIAction(title: "Deactivate link", attributes: .destructive) { [weak self] _ in self?.confirm(link, regenerate: false) }])
            actions.addArrangedSubview(options)
        } else {
            let revoke = UIKitMessageDialog.button(state.pending == link.code ? "Revoking…" : "Revoke") { [weak self] in self?.confirm(link, regenerate: false) }; revoke.isEnabled = !state.busy; actions.addArrangedSubview(revoke)
        }
        let row = UIStackView(arrangedSubviews: [url, meta, actions]); row.axis = .vertical; row.spacing = 6; row.accessibilityIdentifier = "invite-link-\(link.code)"; return row
    }
    private func confirm(_ link: InviteLink, regenerate: Bool) {
        guard !state.busy else { return }
        let action = regenerate ? "Regenerate link" : state.defaultLink?.code == link.code ? "Deactivate link" : "Revoke"
        let alert = UIAlertController(title: action + "?", message: "The previous URL stops admitting new members." + (regenerate ? " The new link resets to 30 days and 100 joins." : ""), preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel)); alert.addAction(UIAlertAction(title: action, style: .destructive) { [state] _ in if regenerate { state.regenerate(link) } else { state.revoke(link) } }); present(alert)
    }
}
#endif
