import CaperCore
import SwiftUI

@main
struct CaperMacOSApp: App {
    private let parity = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
    private let updater = AppUpdater()
    @State private var ready = false

    var body: some Scene {
        WindowGroup {
            CaperWindow(parity: parity, updater: updater, ready: $ready)
        }
        // Keep system window controls in their own title bar, outside the space rail.
        .windowStyle(.titleBar)
        .defaultSize(width: parity ? 1440 : 1180, height: parity ? 900 : 760)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { updater.check(manual: true) }
                    .disabled(!ready || !updater.isAvailable)
            }
        }
        Settings {
            CaperSettingsView()
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
            guard !ready else { return }
            ready = updater.prepareInstallation()
            if ready { updater.start() }
        }
        .onOpenURL { model.openInviteLink($0) }
        // An opened link goes to an open window instead of opening another;
        // with every window closed, SwiftUI opens one for it.
        .handlesExternalEvents(preferring: ["*"], allowing: ["*"])
    }
}
