import AppKit
import CaperCore
import SwiftUI

@main
struct CaperMacOSApp: App {
    private let parity = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
    @State private var updater = AppUpdater()
    @State private var ready = false
    @State private var whatsNew: WhatsNewModel?
    @Environment(\.openWindow) private var openWindow

    var body: some Scene {
        WindowGroup(id: "caper-main") {
            CaperWindow(parity: parity, updater: updater, ready: $ready, whatsNew: $whatsNew)
                .environment(\.desktopUpdates, updater.controls)
                .onChange(of: updater.reviewRequest) { _, _ in
                    bringReviewForward()
                }
        }
        // Keep system window controls in their own title bar, outside the space rail.
        .windowStyle(.titleBar)
        .defaultSize(width: parity ? 1440 : 1180, height: parity ? 900 : 760)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { updater.check(manual: true) }
                    .disabled(!ready || !updater.isAvailable || updater.checking)
            }
        }
        Settings {
            if let whatsNew {
                CaperSettingsView(whatsNew: whatsNew)
                    .environment(\.desktopUpdates, updater.controls)
                    // Settings can outlive the main window. Bring back the real
                    // account window so the review uses its actual voice state.
                    .onChange(of: updater.reviewRequest) { _, _ in
                        bringReviewForward()
                    }
            }
        }
    }

    private func bringReviewForward() {
        if let window = NSApp.windows.first(where: {
            $0.identifier?.rawValue == "caper-main" && ($0.isVisible || $0.isMiniaturized)
        }) {
            window.deminiaturize(nil)
            window.makeKeyAndOrderFront(nil)
        } else {
            openWindow(id: "caper-main")
        }
    }
}

/// One Caper window, with its own model as before. It owns the model so a
/// `caper://invite/<code>` link that arrives before the window is ready still
/// reaches it.
private struct CaperWindow: View {
    let parity: Bool
    let updater: AppUpdater
    @Binding var ready: Bool
    @Binding var whatsNew: WhatsNewModel?
    @Environment(\.controlActiveState) private var windowActivity
    @State private var model = CaperRuntime.makeModel()

    var body: some View {
        ZStack {
            Group {
                if ready {
                    CaperRootView(model: model)
                } else {
                    Color.clear
                }
            }
            .disabled(updater.offeredUpdate != nil)
            .accessibilityElement(children: .contain)
            .accessibilityHidden(updater.offeredUpdate != nil)
            if let update = updater.offeredUpdate {
                DesktopUpdateReview(update: update,
                    inCall: model.voice.phase != .idle && model.voice.phase != .failed,
                    close: updater.dismissReview, restart: updater.applyReviewedUpdate,
                    download: updater.download,
                    report: model.canSendUpdateReport ? { [installed = updater.installedVersion] error in
                        await model.sendUpdateReport(error: error,
                                                     installed: String(installed.dropFirst("Installed: ".count)),
                                                     target: update.version)
                    } : nil)
            }
        }
        .frame(minWidth: parity ? 390 : 840, minHeight: 600)
        .background(UpdateWindowRegistration())
        .onAppear {
            whatsNew = model.whatsNew
            guard !ready else { return }
            ready = updater.prepareInstallation()
            if ready { updater.start() }
        }
        .onChange(of: windowActivity) { _, activity in
            if activity == .key { whatsNew = model.whatsNew }
        }
        .onOpenURL { model.openInviteLink($0) }
        // An opened link goes to an open window instead of opening another;
        // with every window closed, SwiftUI opens one for it.
        .handlesExternalEvents(preferring: ["*"], allowing: ["*"])
    }
}

/// Identifies account windows without depending on SwiftUI's private window IDs.
private struct UpdateWindowRegistration: NSViewRepresentable {
    final class RegistrationView: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            window?.identifier = NSUserInterfaceItemIdentifier("caper-main")
        }
    }
    func makeNSView(context: Context) -> RegistrationView { RegistrationView() }
    func updateNSView(_ nsView: RegistrationView, context: Context) { }
}
