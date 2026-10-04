import CaperCore
import AppKit
import SwiftUI

@MainActor
private final class DailyDockIcon {
    private let defaults = UserDefaults.standard
    private var timer: Timer?
    private var currentIndex: Int?

    func start(enabled: Bool) {
        guard enabled, timer == nil else { return }
        refresh()
        NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification,
            object: nil, queue: .main) { [weak self] _ in Task { @MainActor in self?.refresh() } }
        timer = Timer.scheduledTimer(withTimeInterval: 15 * 60, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    private func refresh(now: Date = Date()) {
        let day = CaperDailyIcon.utcDay(containing: now)
        let savedDay = defaults.string(forKey: "daily-dock-icon-day-v1")
        let savedIndex = defaults.object(forKey: "daily-dock-icon-index-v1") as? Int
        let index = CaperDailyIcon.select(day: day, savedDay: savedDay, savedIndex: savedIndex,
            random: UInt64.random(in: UInt64.min...UInt64.max))
        if day != savedDay || index != savedIndex {
            defaults.set(day, forKey: "daily-dock-icon-day-v1")
            defaults.set(index, forKey: "daily-dock-icon-index-v1")
        }
        if currentIndex != index {
            NSApplication.shared.applicationIconImage = CaperAvatar.image(for: index)
            currentIndex = index
        }
    }
}

@main
struct CaperMacOSApp: App {
    private let parity = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
    private let updater = AppUpdater()
    private let dailyDockIcon = DailyDockIcon()

    var body: some Scene {
        WindowGroup {
            CaperRootView()
                .frame(minWidth: parity ? 390 : 840, minHeight: 600)
                .onAppear {
                    updater.start()
                    dailyDockIcon.start(enabled: !parity)
                }
        }
        // Keep system window controls in their own title bar, outside the space rail.
        .windowStyle(.titleBar)
        .defaultSize(width: parity ? 1440 : 1180, height: parity ? 900 : 760)
        .commands {
            CommandGroup(after: .appInfo) {
                CaperLaunchAtLoginControls()
                Divider()
                Button("Check for Updates…") { updater.check(manual: true) }
                    .disabled(!updater.isAvailable)
            }
        }
    }
}
