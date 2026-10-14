import CaperCore
import SwiftUI

@main
struct CaperMacOSApp: App {
    private let parity = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
    @State private var updater = AppUpdater()
    @State private var ready = false
    @State private var whatsNew: WhatsNewModel?

    var body: some Scene {
        WindowGroup {
            CaperWindow(parity: parity, updater: updater, ready: $ready, whatsNew: $whatsNew)
                .environment(\.desktopUpdates, updater.controls)
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
            }
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
        Group {
            if ready {
                CaperRootView(model: model)
            } else {
                Color.clear
            }
        }
        .frame(minWidth: parity ? 390 : 840, minHeight: 600)
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
