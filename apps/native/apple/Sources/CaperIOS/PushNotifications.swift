import CaperCore
import UIKit
import UserNotifications

@MainActor
final class PushNotifications: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    weak var model: AppModel? { didSet { bindModel() } }
    private var token: String?
    private var platform: String?
    private var accountID: String?
    private var pendingConversationID: String?
    private var optInGeneration = 0

    func application(_ application: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    private func bindModel() {
        model?.setPushEnabled = { [weak self] enabled in await self?.setEnabled(enabled) }
        model?.disablePushLocally = { [weak self] in self?.disableLocally() }
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
            model.configurePush(available: available, enabled: available && UserDefaults.standard.bool(forKey: "caper.push.enabled.\(account.id)"))
            platform = available ? wanted : nil
            if model.pushEnabled { UIApplication.shared.registerForRemoteNotifications() }
            if let pendingConversationID {
                self.pendingConversationID = nil
                await model.openDirectMessage(id: pendingConversationID)
            }
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

    private func disableLocally() {
        optInGeneration += 1
        model?.configurePush(available: platform != nil, enabled: false)
        if let accountID { UserDefaults.standard.set(false, forKey: "caper.push.enabled.\(accountID)") }
        pendingConversationID = nil
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
            UserDefaults.standard.set(true, forKey: "caper.push.enabled.\(accountID)")
            model.configurePush(available: true, enabled: true)
            UIApplication.shared.registerForRemoteNotifications()
        } else {
            disableLocally()
            if let token { try? await model.api.unregisterPushDevice(platform: platform, token: token) }
        }
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        let value = deviceToken.map { String(format: "%02x", $0) }.joined()
        token = value
        guard let model, model.pushEnabled, let platform, let accountID, model.account?.id == accountID else { return }
        Task {
            guard model.pushEnabled, model.account?.id == accountID else { return }
            try? await model.api.registerPushDevice(platform: platform, token: value)
            guard model.account?.id == accountID else { return }
            if let pendingConversationID { self.pendingConversationID = nil; await model.openDirectMessage(id: pendingConversationID) }
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let id = response.notification.request.content.userInfo["conversationId"] as? String else { return }
        guard let model, model.account != nil else { pendingConversationID = id; return }
        await model.openDirectMessage(id: id)
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        guard let model, model.account != nil, model.pushEnabled,
              let id = notification.request.content.userInfo["conversationId"] as? String,
              model.selectedDirectMessageID != id else { return [] }
        return [.banner, .sound]
    }
}
