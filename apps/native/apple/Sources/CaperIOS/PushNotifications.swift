import CaperCore
import UIKit
import UserNotifications

@MainActor
final class PushNotifications: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    weak var model: AppModel? { didSet { bindModel() } }
    private var token: String?
    private var platform: String?
    private var accountID: String?
    /// A notification tapped before the model was attached (a cold launch).
    private var pendingRoute: NotificationRoute?
    private var optInGeneration = 0
    /// The bundle ID, which the server uses as the APNs topic for this device.
    private var appID: String? { Bundle.main.bundleIdentifier }

    func application(_ application: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    private func bindModel() {
        model?.setPushEnabled = { [weak self] enabled in await self?.setEnabled(enabled) }
        model?.disablePushLocally = { [weak self] in self?.disableLocally(remember: false) }
        if let model, let route = pendingRoute {
            pendingRoute = nil
            // The model holds it until the account's spaces load.
            Task { await model.open(route) }
        }
        Task { await refreshConfiguration() }
    }

    func refreshConfiguration() async {
        guard let model, let account = model.account else {
            accountID = nil; platform = nil
            return
        }
        accountID = account.id
        do {
            let config = try await model.api.pushConfiguration()
            guard model.account?.id == account.id else { return }
            guard let wanted = apnsPlatform else {
                model.configurePush(available: false, enabled: false)
                return
            }
            let available = config.platforms.contains(wanted)
            let choice = Self.choice(for: account.id)
            model.configurePush(available: available, enabled: available && choice == true)
            platform = available ? wanted : nil
            // On by default: until the account turns push on or off here, each
            // launch asks iOS. It prompts only once; after that it answers with
            // the person's decision, so allowing later in Settings turns push on.
            if model.pushEnabled { UIApplication.shared.registerForRemoteNotifications() }
            else if available && choice == nil { await setEnabled(true) }
        } catch { if model.account?.id == account.id { model.configurePush(available: false, enabled: false) } }
    }

    // iOS has no public SecTask entitlement reader. This value is generated from
    // the same build setting as the APS entitlement; simulators never register.
    private var apnsPlatform: String? {
        #if targetEnvironment(simulator)
        return nil
        #else
        guard let value = Bundle.main.object(forInfoDictionaryKey: "CaperAPNsEnvironment") as? String else { return nil }
        if value == "development" { return "apnsSandbox" }
        if value == "production" { return "apns" }
        return nil
        #endif
    }

    /// This account's choice on this phone: nil until it turns push on or off
    /// here. (A new key: older builds also wrote `false` on logout.)
    private static func choice(for accountID: String) -> Bool? {
        UserDefaults.standard.object(forKey: "caper.push.choice.\(accountID)") as? Bool
    }

    private static func setChoice(_ on: Bool, for accountID: String) {
        UserDefaults.standard.set(on, forKey: "caper.push.choice.\(accountID)")
    }

    /// Logout stops push without recording a choice, so signing in again keeps the default.
    private func disableLocally(remember: Bool) {
        optInGeneration += 1
        model?.configurePush(available: platform != nil, enabled: false)
        if remember, let accountID { Self.setChoice(false, for: accountID) }
        pendingRoute = nil
        UIApplication.shared.unregisterForRemoteNotifications()
    }

    func setEnabled(_ enabled: Bool) async {
        guard let model, let accountID, model.account?.id == accountID, let platform else { return }
        if enabled {
            optInGeneration += 1
            let attempt = optInGeneration
            let granted = (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound])) == true
            guard model.account?.id == accountID, optInGeneration == attempt else { return }
            guard granted else { model.configurePush(available: true, enabled: false); return }
            Self.setChoice(true, for: accountID)
            model.configurePush(available: true, enabled: true)
            UIApplication.shared.registerForRemoteNotifications()
        } else {
            disableLocally(remember: true)
            if let token { try? await model.api.unregisterPushDevice(platform: platform, token: token, appID: appID) }
        }
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        let value = deviceToken.map { String(format: "%02x", $0) }.joined()
        token = value
        guard let model, model.pushEnabled, let platform, let accountID, model.account?.id == accountID else { return }
        let bundleID = appID
        Task {
            guard model.pushEnabled, model.account?.id == accountID else { return }
            try? await model.api.registerPushDevice(platform: platform, token: value, appID: bundleID)
        }
    }

    /// A tap opens the push's channel (`spaceId` + `channelId`) or DM (`conversationId`).
    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let route = NotificationRoute(userInfo: response.notification.request.content.userInfo) else { return }
        guard let model else { pendingRoute = route; return }
        await model.open(route)
    }

    /// The server's alert, except for the conversation that is already open.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        guard let model, model.account != nil, model.pushEnabled else { return [] }
        if let route = NotificationRoute(userInfo: notification.request.content.userInfo), model.isShowing(route) { return [] }
        return [.banner, .list, .sound]
    }
}
