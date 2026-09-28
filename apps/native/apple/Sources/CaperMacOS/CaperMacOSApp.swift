import CaperCore
import SwiftUI

@main
struct CaperMacOSApp: App {
    private let parity = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
    @StateObject private var updater = AppUpdater()

    var body: some Scene {
        WindowGroup {
            CaperRootView()
                .frame(minWidth: parity ? 390 : 840, minHeight: 600)
                .modifier(UpdatePrompts(updater: updater))
                .onAppear { updater.start() }
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: parity ? 1440 : 1180, height: parity ? 900 : 760)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { updater.check(manual: true) }
                    .disabled(!updater.isAvailable)
            }
        }
    }
}

/// Native alerts for a found update, a manual check with nothing new, or a failure.
private struct UpdatePrompts: ViewModifier {
    @ObservedObject var updater: AppUpdater

    func body(content: Content) -> some View {
        content
            .alert(
                "Caper \(updater.prompt?.version ?? "") is available",
                isPresented: Binding(get: { updater.prompt != nil }, set: { if !$0 { updater.prompt = nil } }),
                presenting: updater.prompt
            ) { update in
                if update.canApply {
                    Button("Restart to Update") { updater.restart() }
                } else {
                    Button("Download") { NSWorkspace.shared.open(AppUpdater.downloadPage) }
                }
                Button("Later", role: .cancel) { updater.later() }
            } message: { update in
                let restart = update.canApply
                    ? "Caper restarts to install it and leaves any voice call."
                    : "Caper can't update this copy in place. Download the new version, or move Caper to Applications so updates install automatically."
                Text(update.notes.isEmpty ? restart : "\(update.notes)\n\n\(restart)")
            }
            .alert("Caper is up to date", isPresented: $updater.upToDate) {
                Button("OK", role: .cancel) {}
            }
            .alert(
                "Update failed",
                isPresented: Binding(get: { updater.failure != nil }, set: { if !$0 { updater.failure = nil } })
            ) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(updater.failure ?? "")
            }
    }
}
