#if os(macOS)
import AppKit
import SwiftUI

/// One Settings window shares the shipping preferences and release history.
/// The development host never registers a login item or installs an update.
@MainActor public final class AppKitSettingsPresenter: NSObject, NSWindowDelegate {
    private let model: AppModel
    private var window: NSWindow?
    private weak var previousWindow: NSWindow?
    private weak var previousResponder: NSResponder?
    private var previousContext: String?
    private var focusContext: String { "\(model.chat.editingContext):\(model.chat.threadRootID ?? "channel")" }

    public init(model: AppModel) { self.model = model; super.init() }

    public var canOpenWhatsNew: Bool { !model.whatsNew.workspaceModalOpen || model.whatsNew.settingsOpen }

    public func show(from parent: NSWindow?) {
        // Do not transfer or acknowledge an already-presented main-window sheet.
        if model.whatsNew.presentation != nil && !model.whatsNew.settingsOpen {
            (parent?.attachedSheet ?? parent)?.makeKeyAndOrderFront(nil)
            return
        }
        if window?.isVisible != true {
            previousWindow = parent
            previousResponder = parent?.firstResponder
            previousContext = focusContext
        }
        if window == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 380),
                styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.title = "Caper Settings — AppKit development"
            window.identifier = NSUserInterfaceItemIdentifier("caper-appkit-settings")
            window.isReleasedWhenClosed = false
            window.delegate = self
            let loginItem = CaperLoginItem(available: false, status: { .notRegistered }, register: {}, unregister: {})
            window.contentViewController = NSHostingController(rootView: CaperSettingsView(
                whatsNew: model.whatsNew, loginItem: loginItem, managesPresentation: false))
            window.center()
            self.window = window
        }
        // NSWindow.close() retains this host for reuse. Its visibility, not
        // SwiftUI onAppear/onDisappear, owns the release-note presentation gate.
        model.whatsNew.settingsOpen = true
        window?.makeKeyAndOrderFront(nil)
    }

    public func openWhatsNew(from parent: NSWindow?) {
        guard canOpenWhatsNew else { return }
        let owner = model.whatsNew.settingsOpen ? window : parent
        (owner?.attachedSheet ?? owner)?.makeKeyAndOrderFront(nil)
        guard model.whatsNew.presentation == nil else { return }
        model.whatsNew.open()
    }

    public func windowShouldClose(_ sender: NSWindow) -> Bool {
        // Escape/Done dismisses the nested sheet first, preserving its explicit
        // acknowledgment. Command-W must not reparent it onto the workspace.
        sender.attachedSheet == nil && model.whatsNew.presentation == nil
    }

    public func windowWillClose(_ notification: Notification) {
        let restore = window?.isKeyWindow == true
        model.whatsNew.settingsOpen = false
        let context = previousContext
        let responder = previousResponder as? NSView
        let parent = previousWindow
        previousResponder = nil; previousWindow = nil; previousContext = nil
        DispatchQueue.main.async { [weak self, weak responder, weak parent] in
            guard let self, restore, NSApp.isActive, let parent, parent.isVisible,
                  self.window?.isVisible != true, context == self.focusContext,
                  !self.model.whatsNew.workspaceModalOpen, parent.attachedSheet == nil,
                  NSApp.keyWindow == nil || NSApp.keyWindow === parent,
                  let responder, responder.window === parent, !responder.isHiddenOrHasHiddenAncestor else { return }
            parent.makeKeyAndOrderFront(nil)
            parent.makeFirstResponder(responder)
        }
    }
}

/// Like UIKit's root presenter, only release notes use a zero-size SwiftUI
/// host. Keep it mounted while native sign-in/workspace controllers change.
@MainActor public enum AppKitWhatsNew {
    public static func presenter(model: AppModel) -> NSViewController {
        NSHostingController(rootView: AppKitWhatsNewHost(model: model))
    }
}

@MainActor private struct AppKitWhatsNewHost: View {
    @Bindable var model: AppModel
    @State private var active = NSApp.isActive

    var body: some View {
        Color.clear.frame(width: 0, height: 0)
            .modifier(WhatsNewPresentation(model: model, applicationActive: active))
            .preferredColorScheme(.dark)
            .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in active = true }
            .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in active = false }
            .task(id: model.phase == .ready) {
                if model.phase == .ready { CaperRuntime.showWhatsNewPreview(model) }
            }
    }
}
#endif
