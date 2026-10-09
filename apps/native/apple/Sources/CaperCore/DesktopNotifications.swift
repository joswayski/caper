#if os(macOS)
import AppKit
import UserNotifications

/// Local notifications while the Mac app runs; this is not APNs after quit.
@MainActor
final class DesktopNotifications: NSObject, UNUserNotificationCenterDelegate {
    private weak var model: AppModel?
    private var gateway: Gateway?
    private var accountID: String?
    private var epoch = 0

    init(model: AppModel) { self.model = model }

    func start() {
        guard ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] != "parity",
              let model, let account = model.account, accountID != account.id else { return }
        stop()
        accountID = account.id
        let epoch = self.epoch
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        let api = model.api
        let gateway = Gateway(baseURL: api.baseURL, token: { await api.authorizationToken() }) { _, _ in }
        self.gateway = gateway
        model.removeDeliveredNotifications = { id in
            Task {
                let notifications = await center.deliveredNotifications()
                center.removeDeliveredNotifications(withIdentifiers: notifications.filter { $0.request.content.threadIdentifier == id }.map(\.request.identifier))
            }
        }
        Task { [weak self] in
            _ = try? await center.requestAuthorization(options: [.alert, .sound, .badge])
            guard self?.accountID == account.id, self?.epoch == epoch else { return }
            await gateway.subscribeNotifications { [weak self] event in
                guard self?.accountID == account.id, self?.epoch == epoch else { return }
                self?.receive(event)
            }
        }
    }

    func stop() {
        epoch += 1
        accountID = nil
        let previous = gateway; gateway = nil
        Task { await previous?.stop() }
        // Logout reaches this in unit tests, whose xctest host is not an app
        // bundle; there the notification center throws.
        guard Bundle.main.bundleURL.pathExtension == "app" else { return }
        UNUserNotificationCenter.current().removeAllDeliveredNotifications()
        UNUserNotificationCenter.current().removeAllPendingNotificationRequests()
    }

    private func receive(_ event: [String: Any]) {
        guard event["type"] as? String == "notification.created", let model, model.account?.id == accountID,
              let route = NotificationRoute(userInfo: event),
              let messageID = event["messageId"] as? String,
              let title = event["title"] as? String, let body = event["body"] as? String,
              let timestamp = event["createdAt"] as? String else { return }
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let date = formatter.date(from: timestamp) ?? ISO8601DateFormatter().date(from: timestamp)
        guard let date, (-30...120).contains(Date().timeIntervalSince(date)) else { return }
        if NSApp.isActive && !model.navigationOpen && model.isShowing(route) { return }
        let content = UNMutableNotificationContent()
        content.title = title; content.body = body; content.sound = .default; content.userInfo = event
        content.threadIdentifier = event["conversationId"] as? String ?? event["channelId"] as? String ?? ""
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
        let request = UNNotificationRequest(identifier: "\(accountID ?? ""):\(epoch):\(messageID)", content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request) { _ in
            if let temporary { try? FileManager.default.removeItem(at: temporary) }
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        guard let model, model.account?.id == accountID,
              notification.request.identifier.hasPrefix("\(accountID ?? ""):\(epoch):") else { return [] }
        if let route = NotificationRoute(userInfo: notification.request.content.userInfo), NSApp.isActive && !model.navigationOpen && model.isShowing(route) { return [] }
        return [.banner, .list, .sound]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let model, model.account?.id == accountID,
              response.notification.request.identifier.hasPrefix("\(accountID ?? ""):\(epoch):"),
              let route = NotificationRoute(userInfo: response.notification.request.content.userInfo) else { return }
        NSApp.activate(ignoringOtherApps: true)
        await model.open(route)
    }
}
#endif
