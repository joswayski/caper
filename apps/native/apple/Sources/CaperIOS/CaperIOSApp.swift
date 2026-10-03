import CaperCore
import SwiftUI

@main
struct CaperIOSApp: App {
    @UIApplicationDelegateAdaptor(PushNotifications.self) private var push
    @State private var model = CaperRuntime.makeModel()

    var body: some Scene {
        WindowGroup {
            CaperRootView(model: model)
                .onAppear { push.model = model }
                .onChange(of: model.account?.id) { _, _ in Task { await push.refreshConfiguration() } }
        }
    }
}
