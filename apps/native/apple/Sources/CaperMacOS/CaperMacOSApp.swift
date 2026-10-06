import CaperCore
import SwiftUI

@main
struct CaperMacOSApp: App {
    private let parity = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
    private let updater = AppUpdater()
    @State private var ready = false

    var body: some Scene {
        WindowGroup {
            Group {
                if ready {
                    CaperRootView()
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
