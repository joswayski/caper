import Foundation
import UserNotifications

/// Reply and Mark as read on message notifications: iPhone pushes (the server
/// sets `aps.category`) and the Mac's local notifications. Both work with the
/// saved session alone, so iOS can run them with the app in the background
/// or not running at all.
enum NotificationActions {
    static let category = "caper.message"
    static let reply = "caper.reply"
    static let read = "caper.read"

    static func register(on center: UNUserNotificationCenter) {
        let reply = UNTextInputNotificationAction(identifier: reply, title: "Reply", options: [],
                                                  textInputButtonTitle: "Send", textInputPlaceholder: "Reply")
        let read = UNNotificationAction(identifier: read, title: "Mark as read", options: [])
        center.setNotificationCategories([UNNotificationCategory(identifier: category, actions: [reply, read],
                                                                 intentIdentifiers: [], options: [])])
    }

    /// Messages only: never invitations, message requests or voice.
    static func actionable(_ userInfo: [AnyHashable: Any]) -> Bool {
        guard let id = userInfo["messageId"] as? String, !id.isEmpty, !(userInfo["notificationId"] is String) else { return false }
        return !["space.invitation", "channel.invitation", "direct.request", "voice.started", "message.deleted"]
            .contains(userInfo["kind"] as? String ?? "")
    }

    /// Runs Reply or Mark as read and returns the account's badge count, or
    /// nil for any other response (a tap, a dismissal) or a failure. A reply
    /// that could not be sent says so in a notification of its own.
    static func handle(_ response: UNNotificationResponse, api: APIClient) async -> Int? {
        let content = response.notification.request.content
        guard [reply, read].contains(response.actionIdentifier), actionable(content.userInfo),
              let messageID = content.userInfo["messageId"] as? String else { return nil }
        if response.actionIdentifier == read {
            guard let result = try? await retrying({ try await api.readFromNotification(messageID: messageID) }) else { return nil }
            await removeRead(through: response.notification)
            return result.badgeCount
        }
        let text = (response as? UNTextInputNotificationResponse)?.userText.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !text.isEmpty else { return nil }
        // One ID for every attempt: a retry after a lost response never sends twice.
        let clientMessageID = UUID().uuidString.lowercased()
        do {
            return try await retrying {
                try await api.replyFromNotification(messageID: messageID, clientMessageID: clientMessageID, text: text)
            }.badgeCount
        } catch {
            let failed = UNMutableNotificationContent()
            failed.title = content.title
            failed.body = "Couldn’t send “\(text)”. Open Caper to try again."
            failed.threadIdentifier = content.threadIdentifier
            failed.userInfo = content.userInfo.filter { ["spaceId", "channelId", "conversationId", "kind", "threadRootId", "messageId"].contains($0.key as? String) }
            try? await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "reply-failed:\(messageID)", content: failed, trigger: nil))
            return nil
        }
    }

    private static func retrying<T>(_ work: () async throws -> T) async throws -> T {
        do { return try await work() } catch is URLError { return try await work() }
    }

    /// Reading a message also reads what came before it in the same
    /// conversation (or thread), so those notifications go too.
    private static func removeRead(through notification: UNNotification) async {
        let center = UNUserNotificationCenter.current()
        let content = notification.request.content
        let root = content.userInfo["threadRootId"] as? String
        let read = await center.deliveredNotifications().filter {
            $0.request.content.threadIdentifier == content.threadIdentifier && $0.date <= notification.date
                && $0.request.content.userInfo["threadRootId"] as? String == root && actionable($0.request.content.userInfo)
        }.map(\.request.identifier)
        center.removeDeliveredNotifications(withIdentifiers: read + [notification.request.identifier])
    }
}

/// A notification's Reply or Mark as read: the account's new badge count.
struct NotificationActionResult: Decodable, Sendable {
    let badgeCount: Int
    let messageId: String?
}
