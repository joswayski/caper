import CaperCore
import SwiftUI

@main
struct CaperMacOSApp: App {
    private let parity = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
    var body: some Scene {
        WindowGroup { CaperRootView().frame(minWidth: parity ? 390 : 840, minHeight: 600) }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: parity ? 1440 : 1180, height: parity ? 900 : 760)
    }
}
