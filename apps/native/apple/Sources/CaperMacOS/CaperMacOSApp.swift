import CaperCore
import SwiftUI

@main
struct CaperMacOSApp: App {
    var body: some Scene {
        WindowGroup { CaperRootView().frame(minWidth: 840, minHeight: 600) }
        .windowStyle(.hiddenTitleBar)
    }
}
