import CaperCore
import Darwin
import os
import UIKit
import UserNotifications

final class CaperUIKitAppDelegate: UIResponder, UIApplicationDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        // Satoshi is registered by UIAppFonts before launch; nothing to load here.
        true
    }

    func application(_ application: UIApplication, configurationForConnecting session: UISceneSession,
                     options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        let configuration = UISceneConfiguration(name: nil, sessionRole: session.role)
        configuration.delegateClass = CaperUIKitSceneDelegate.self
        return configuration
    }
}

enum CaperUIKitLaunch {
    static let log = Logger(subsystem: "chat.caper.uikit", category: "launch")
    static var parityFixture: Bool { ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity" }
    /// The server CaperRuntime.makeModel talks to (the loopback fixture in parity launches).
    static var origin: URL {
        let environment = ProcessInfo.processInfo.environment
        if parityFixture, let raw = environment["CAPER_API_BASE_URL"], let url = URL(string: raw) { return url }
        return URL(string: "https://caper.chat")!
    }

    static func mark(_ step: String) {
        guard let start = processStart else { return }
        log.notice("\(step, privacy: .public) at \(Int(Date().timeIntervalSince(start) * 1000), privacy: .public) ms")
    }

    /// When the kernel started this process, so first paint counts dyld and
    /// static initialisers too, not just our code.
    static var processStart: Date? {
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var name: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        guard sysctl(&name, u_int(name.count), &info, &size, nil, 0) == 0 else { return nil }
        let start = info.kp_proc.p_starttime
        return Date(timeIntervalSince1970: TimeInterval(start.tv_sec) + TimeInterval(start.tv_usec) / 1_000_000)
    }

    /// Parity launches use the cache only when a test asks for it.
    static var cacheEnabled: Bool {
        !parityFixture || ProcessInfo.processInfo.environment["CAPER_UIKIT_STARTUP_CACHE"] == "1"
    }

    /// The startup cache is read only for a signed-in session: the Keychain
    /// session in normal launches, the fixture bearer in parity launches.
    static func cachedSnapshot() -> StartupSnapshot? {
        let environment = ProcessInfo.processInfo.environment
        if parityFixture {
            guard environment["CAPER_TEST_BEARER"] != nil, cacheEnabled else { return nil }
        } else {
            let store = KeychainTokenStore(service: APIClient.sessionService(for: origin))
            guard (try? store.load()) != nil else { return nil }
        }
        return StartupCache.load(origin: origin)
    }
}

final class CaperUIKitSceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    private var root: RootViewController?
    private var firstFrame: CADisplayLink?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options connectionOptions: UIScene.ConnectionOptions) {
        guard let scene = scene as? UIWindowScene else { return }
        let model = CaperRuntime.makeModel()
        let cached = CaperUIKitLaunch.cachedSnapshot()
        let root = RootViewController(model: model, cached: cached)
        let window = UIWindow(windowScene: scene)
        window.rootViewController = root
        window.tintColor = UIKitStyle.terracottaBright
        window.overrideUserInterfaceStyle = .dark
        window.makeKeyAndVisible()
        self.window = window
        self.root = root
        measureFirstFrame(cached: cached != nil)
        notifications.model = model
        UNUserNotificationCenter.current().delegate = notifications
        // A link that launched the app; the model holds it until the workspace is ready.
        for context in connectionOptions.urlContexts { model.openInviteLink(context.url) }
        if let url = connectionOptions.userActivities.first(where: { $0.activityType == NSUserActivityTypeBrowsingWeb })?.webpageURL {
            model.openInviteLink(url)
        }
        if let route = connectionOptions.notificationResponse.flatMap({ NotificationRoute(userInfo: $0.notification.request.content.userInfo) }) {
            Task { await model.open(route) }
        }
        // Parity launches stand in for a link opened from another app.
        if CaperUIKitLaunch.parityFixture, let raw = ProcessInfo.processInfo.environment["CAPER_TEST_OPEN_URL"], let url = URL(string: raw) {
            model.openInviteLink(url)
        }
        Task { await model.start() }
    }

    private let notifications = NotificationTaps()

    /// Invite links (`caper://invite/<code>`, https://caper.chat/invite/<code>),
    /// as CaperIOSApp's onOpenURL and onContinueUserActivity route them. This
    /// development app registers neither, so the shipping app keeps both.
    func scene(_ scene: UIScene, openURLContexts contexts: Set<UIOpenURLContext>) {
        for context in contexts { root?.model.openInviteLink(context.url) }
    }

    func scene(_ scene: UIScene, continue userActivity: NSUserActivity) {
        if let url = userActivity.webpageURL { root?.model.openInviteLink(url) }
    }

    /// The first display refresh after the window is committed: the frame the
    /// reader sees. Logged so device runs can be read from Console.
    private func measureFirstFrame(cached: Bool) {
        let link = CADisplayLink(target: FirstFrame { [weak self] in
            self?.firstFrame?.invalidate()
            self?.firstFrame = nil
            guard let start = CaperUIKitLaunch.processStart else { return }
            let milliseconds = Int(Date().timeIntervalSince(start) * 1000)
            CaperUIKitLaunch.log.notice("first frame \(milliseconds, privacy: .public) ms cached=\(cached, privacy: .public)")
            // Parity launches expose it to the UI tests that record it.
            if CaperUIKitLaunch.parityFixture { self?.window?.accessibilityIdentifier = "first-frame-\(milliseconds)ms-cached-\(cached)" }
        }, selector: #selector(FirstFrame.tick))
        link.add(to: .main, forMode: .common)
        firstFrame = link
    }

    func sceneDidBecomeActive(_ scene: UIScene) { root?.model.applicationActivityChanged(active: true) }
    func sceneWillResignActive(_ scene: UIScene) { root?.model.applicationActivityChanged(active: false) }
    func sceneDidEnterBackground(_ scene: UIScene) { root?.saveNow() }
}

/// A tapped notification opens its channel, DM or feedback visit, as the
/// shipping app's PushNotifications does; the open conversation shows no banner.
@MainActor private final class NotificationTaps: NSObject, UNUserNotificationCenterDelegate {
    weak var model: AppModel?

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let model, let route = NotificationRoute(userInfo: response.notification.request.content.userInfo) else { return }
        await model.open(route)
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        guard let model, model.account != nil else { return [] }
        if let route = NotificationRoute(userInfo: notification.request.content.userInfo), model.isShowing(route) { return [] }
        return [.banner, .list, .sound]
    }
}

private final class FirstFrame: NSObject {
    private var done: (() -> Void)?
    init(_ done: @escaping () -> Void) { self.done = done }
    @objc func tick() {
        done?()
        done = nil
    }
}
