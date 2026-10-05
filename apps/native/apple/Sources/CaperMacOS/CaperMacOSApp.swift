import CaperCore
import SwiftUI

@main
struct CaperMacOSApp: App {
    private let parity = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
    private let updater = AppUpdater()

    var body: some Scene {
        WindowGroup {
            CaperRootView()
                .frame(minWidth: parity ? 390 : 840, minHeight: 600)
                .onAppear { updater.start() }
        }
        // Keep system window controls in their own title bar, outside the space rail.
        .windowStyle(.titleBar)
        .defaultSize(width: parity ? 1440 : 1180, height: parity ? 900 : 760)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { updater.check(manual: true) }
                    .disabled(!updater.isAvailable)
            }
        }
        Settings {
            CaperSettingsView()
        }
    }
}
