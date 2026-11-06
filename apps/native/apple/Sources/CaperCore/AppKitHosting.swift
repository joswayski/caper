#if os(macOS)
import AppKit
import SwiftUI

/// On-demand profile surfaces shared with the shipping Mac app. The workspace,
/// timeline and main composer remain AppKit; no hosting view exists until open.
@MainActor public final class AppKitProfilePresenter: NSObject, NSPopoverDelegate {
    private let model: AppModel
    private var popover: NSPopover?
    private var draft: ProfileMessageDraft?
    private var dismissAction: ProfileDismissAction?

    public init(model: AppModel) { self.model = model }

    public func show(author: ChatAuthor, relativeTo rect: NSRect, of view: NSView) {
        guard !author.isGuest else { return }
        show(MentionCard.resolve(author, members: model.detail?.members ?? [], people: model.people,
            peers: model.directMessages.map(\.peer), account: model.account, viewerID: model.chat.viewerID), rect: rect, view: view)
    }

    public func show(member: Member, relativeTo rect: NSRect, of view: NSView) {
        show(mention: .user(id: member.id, username: member.username), relativeTo: rect, of: view)
    }

    public func show(mention: MessageMention, relativeTo rect: NSRect, of view: NSView) {
        guard case let .user(id, username) = mention else { return }
        show(MentionCard.resolve(MentionPill(id: id, username: username), members: model.detail?.members ?? [],
            people: model.people, peers: model.directMessages.map(\.peer), account: model.account,
            viewerID: model.chat.viewerID), rect: rect, view: view)
    }

    private func show(_ person: MentionCardPerson, rect: NSRect, view: NSView) {
        guard let window = view.window else { return }
        if let popover, popover.isShown {
            popover.animates = false
            popover.performClose(nil)
            guard !popover.isShown else { return }
        }
        let popover = NSPopover()
        self.popover = popover
        draft = model.profileMessageDraft(person: person)
        dismissAction = nil
        popover.behavior = .transient
        popover.animates = !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        popover.delegate = self
        let host = NSHostingController(rootView: MentionCardView(model: model, person: person,
            requestBlock: { [weak self, weak window] target in
                guard let self, let window else { return }
                self.confirmBlock(target, window: window)
            }, close: { [weak popover] in
                // Entrances animate, closing is instant on every motion setting.
                popover?.animates = false
                popover?.close()
            })
            .onPreferenceChange(ProfileDismissPreference.self) { [weak self] action in self?.dismissAction = action }
            .preferredColorScheme(.dark)
            .tint(CaperTheme.terracottaBright))
        host.sizingOptions = [.preferredContentSize]
        popover.contentViewController = host
        popover.show(relativeTo: rect, of: view, preferredEdge: .maxY)
    }

    public func popoverShouldClose(_ popover: NSPopover) -> Bool {
        if let dismissAction { dismissAction.action(); return false }
        guard draft?.preventsImplicitDismissal != true else { return false }
        popover.animates = false
        return true
    }

    public func popoverDidClose(_ notification: Notification) {
        // Releasing the host runs the card's onDisappear (including a requested
        // block) and leaves its account-scoped draft in the model for reopening.
        guard let closed = notification.object as? NSPopover else { return }
        closed.contentViewController = nil
        // A delayed old notification must never tear down its replacement.
        guard popover === closed else { return }
        popover = nil; draft = nil; dismissAction = nil
    }

    private func confirmBlock(_ target: BlockTarget, window: NSWindow) {
        let alert = NSAlert()
        alert.messageText = target.confirmationTitle
        alert.informativeText = BlockTarget.confirmationMessage
        alert.addButton(withTitle: "Block"); alert.addButton(withTitle: "Cancel")
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn, let self else { return }
            Task { await self.model.chat.block(target) }
        }
    }
}
#endif
