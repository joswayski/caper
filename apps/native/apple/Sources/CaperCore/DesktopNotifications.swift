#if os(macOS)
import AppKit
import UserNotifications

public extension Notification.Name {
    /// Asks the app to show its main window, which may be closed while Caper
    /// keeps running (a notification was clicked).
    static let caperShowMainWindow = Notification.Name("CaperShowMainWindow")
}

/// The main window closed during a call while Caper keeps running
/// (`DesktopWindowSettings`): says so, every time. A click brings it back.
@MainActor public enum BackgroundCallNotice {
    static let identifier = "caper-background-call"

    public static func post(channelName: String) {
        guard ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] != "parity",
              Bundle.main.bundleURL.pathExtension == "app" else { return }
        let content = UNMutableNotificationContent()
        content.title = "You’re still in the call in \(channelName)"
        content.body = "Caper keeps running. Right-click Caper in the Dock to mute, deafen or leave the call."
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: identifier, content: content, trigger: nil)) { _ in }
    }
}

/// Local notifications while the Mac app runs; this is not APNs after quit.
@MainActor
final class DesktopNotifications: NSObject, UNUserNotificationCenterDelegate {
    private weak var model: AppModel?
    private var gateway: Gateway?
    private var accountID: String?
    private var epoch = 0
    /// The gateway refused the notifications subscription.
    private var refused = false
    /// Deleted messages (`notification.retracted`, from the `profiles` feed),
    /// so an original still downloading or arriving late is never shown.
    private var retracted: [String] = []
    nonisolated static let retractedLimit = 200

    init(model: AppModel) { self.model = model }

    func start() {
        // UNUserNotificationCenter throws outside an app bundle, as in the
        // bundle-less xctest host that runs CaperCore's unit tests.
        guard ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] != "parity",
              Bundle.main.bundleURL.pathExtension == "app", let model, let account = model.account, accountID != account.id else { return }
        stop()
        accountID = account.id
        let epoch = self.epoch
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        NotificationActions.register(on: center)
        let api = model.api
        // While this feed is live, the shell polls only as a backstop.
        let gateway = Gateway(baseURL: api.baseURL, token: { await api.authorizationToken() }) { [weak self] state, _ in
            guard let self, self.accountID == account.id, self.epoch == epoch else { return }
            if state != .connected { self.refused = false }
            self.model?.notificationFeedChanged(live: state == .connected && !self.refused)
        }
        self.gateway = gateway
        model.removeDeliveredNotifications = { id in
            Task {
                let notifications = await center.deliveredNotifications()
                center.removeDeliveredNotifications(withIdentifiers: notifications.filter { $0.request.content.threadIdentifier == id }.map(\.request.identifier))
            }
        }
        model.retractNotification = { [weak self] messageID in self?.retract(messageID) }
        Task { [weak self] in
            _ = try? await center.requestAuthorization(options: [.alert, .sound, .badge])
            guard self?.accountID == account.id, self?.epoch == epoch else { return }
            await gateway.subscribeNotifications { [weak self] event in
                guard let self, self.accountID == account.id, self.epoch == epoch else { return }
                if event["type"] as? String == "subscription.error" { self.refused = true; self.model?.notificationFeedChanged(live: false) }
                self.receive(event)
            }
        }
    }

    /// Input in the app: this feed's socket carries it like the chat socket.
    func reportActivity() {
        let gateway = gateway
        Task { await gateway?.reportActivity() }
    }

    func stop() {
        epoch += 1
        accountID = nil
        refused = false
        model?.notificationFeedChanged(live: false)
        retracted = []
        let previous = gateway; gateway = nil
        Task { await previous?.stop() }
        // Logout reaches this in unit tests, whose xctest host is not an app
        // bundle; there the notification center throws.
        guard Bundle.main.bundleURL.pathExtension == "app" else { return }
        UNUserNotificationCenter.current().removeAllDeliveredNotifications()
        UNUserNotificationCenter.current().removeAllPendingNotificationRequests()
    }

    /// Closes the notification shown for a deleted message and drops one
    /// that is still being prepared.
    private func retract(_ messageID: String) {
        guard accountID != nil, !messageID.isEmpty else { return }
        retracted = Self.remembering(messageID, in: retracted)
        Task { await NotificationRetraction.remove(messageID: messageID) }
    }

    /// The newest retracted ids, oldest first, at most `retractedLimit`.
    nonisolated static func remembering(_ messageID: String, in list: [String]) -> [String] {
        Array((list.filter { $0 != messageID } + [messageID]).suffix(retractedLimit))
    }

    private func isRetracted(_ event: [String: Any]) -> Bool {
        guard let id = event["messageId"] as? String else { return false }
        return retracted.contains(id)
    }

    private func receive(_ event: [String: Any]) {
        let prefix = event["kind"] as? String == "voice.started" ? "voice" : event["kind"] as? String == "direct.request" ? "request" : "invitation"
        guard event["type"] as? String == "notification.created", let model, model.account?.id == accountID,
              let route = NotificationRoute(userInfo: event),
              let messageID = (event["notificationId"] as? String).map({ "\(prefix):\($0)" }) ?? event["messageId"] as? String,
              let title = event["title"] as? String, let body = event["body"] as? String,
              let timestamp = event["createdAt"] as? String else { return }
        model.scheduleUnreadRefresh()
        // A new report for the feedback team's inbox.
        if event["kind"] as? String == NotificationRoute.feedbackKind { model.feedbackNotificationArrived() }
        // Do Not Disturb: unread state and the inbox update, but no banner.
        guard model.doNotDisturb == nil else { return }
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let date = formatter.date(from: timestamp) ?? ISO8601DateFormatter().date(from: timestamp)
        // `quiet`: another device of this account was used more recently and alerts instead.
        guard let date, (-30...120).contains(Date().timeIntervalSince(date)), !isRetracted(event),
              event["quiet"] as? Bool != true else { return }
        if model.foreground && !model.navigationOpen && model.isShowing(route) { return }
        let content = UNMutableNotificationContent()
        content.title = title; content.body = body; content.sound = .default; content.userInfo = event
        if NotificationActions.actionable(event) { content.categoryIdentifier = NotificationActions.category }
        content.threadIdentifier = (event["notificationId"] as? String).map({ "\(prefix):\($0)" }) ?? event["conversationId"] as? String ?? event["channelId"] as? String ?? ""
        var temporary: URL?
        if let image = CaperAvatar.image(for: event["senderAvatarId"] as? Int),
           let tiff = image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: tiff),
           let png = bitmap.representation(using: .png, properties: [:]) {
            let url = FileManager.default.temporaryDirectory.appendingPathComponent("caper-notification-\(UUID().uuidString).png")
            if (try? png.write(to: url, options: .atomic)) != nil {
                temporary = url
                if let attachment = try? UNNotificationAttachment(identifier: "sender", url: url, options: nil) { content.attachments = [attachment] }
            }
        }
        let account = accountID
        let generation = epoch
        let senderTemporary = temporary
        Task { [weak self] in
            let media = await NotificationAttachment.load(event)
            guard let self, self.accountID == account, self.epoch == generation, !self.isRetracted(event), model.doNotDisturb == nil,
                  !(model.foreground && !model.navigationOpen && model.isShowing(route)) else {
                if let media { try? FileManager.default.removeItem(at: media.url) }
                if let senderTemporary { try? FileManager.default.removeItem(at: senderTemporary) }
                return
            }
            if let media { content.attachments = [media] }
            let request = UNNotificationRequest(identifier: "\(account ?? ""):\(generation):\(messageID)", content: content, trigger: nil)
            UNUserNotificationCenter.current().add(request) { error in
                if let senderTemporary { try? FileManager.default.removeItem(at: senderTemporary) }
                // Successful attachments belong to the system until notification removal.
                if error != nil, let media { try? FileManager.default.removeItem(at: media.url) }
            }
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        if notification.request.identifier == BackgroundCallNotice.identifier { return [.banner, .list] }
        guard let model, model.account?.id == accountID,
              notification.request.identifier.hasPrefix("\(accountID ?? ""):\(epoch):") else { return [] }
        model.scheduleUnreadRefresh()
        if model.doNotDisturb != nil { return [] }
        if let route = NotificationRoute(userInfo: notification.request.content.userInfo), model.foreground && !model.navigationOpen && model.isShowing(route) { return [] }
        return [.banner, .list, .sound]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        if response.notification.request.identifier == BackgroundCallNotice.identifier {
            NSApp.activate(ignoringOtherApps: true)
            NotificationCenter.default.post(name: .caperShowMainWindow, object: nil)
            return
        }
        guard let model, model.account?.id == accountID,
              response.notification.request.identifier.hasPrefix("\(accountID ?? ""):\(epoch):"),
              let route = NotificationRoute(userInfo: response.notification.request.content.userInfo) else { return }
        // Reply and Mark as read stay in the notification; the Dock badge follows the unread refresh.
        if [NotificationActions.reply, NotificationActions.read].contains(response.actionIdentifier) {
            if await NotificationActions.handle(response, api: model.api) != nil { model.scheduleUnreadRefresh() }
            return
        }
        NSApp.activate(ignoringOtherApps: true)
        NotificationCenter.default.post(name: .caperShowMainWindow, object: nil)
        await model.open(route)
    }
}
#endif
