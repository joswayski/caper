import Intents
import UIKit
import UserNotifications

/// Bundled sender artwork and optional signed original downloads. No account
/// token, shared keychain, provider credential or WebRTC dependency.
final class NotificationService: UNNotificationServiceExtension {
    private let lock = NSLock()
    private var handler: ((UNNotificationContent) -> Void)?
    private var original: UNNotificationContent?
    private var download: Task<Void, Never>?

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        lock.lock(); handler = contentHandler; original = request.content; lock.unlock()
        // A deleted message's retraction replaces the original by its collapse
        // id; originals delivered before collapse ids are removed by
        // `messageId`. Its passive "This message was deleted." goes out plain.
        if let deleted = NotificationRetraction.deletedMessageID(request.content.userInfo) {
            download = Task { [self] in
                await NotificationRetraction.remove(messageID: deleted)
                finish(request.content)
            }
            return
        }
        download = Task { [self] in
            let attachment = await NotificationAttachment.load(request.content.userInfo)
            guard !Task.isCancelled, let content = request.content.mutableCopy() as? UNMutableNotificationContent else {
                if let attachment { try? FileManager.default.removeItem(at: attachment.url) }
                return
            }
            if let attachment { content.attachments = [attachment] }
            decorate(content)
        }
    }

    private func decorate(_ content: UNNotificationContent) {
        guard let intent = Self.intent(for: content) else { finish(content); return }
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .incoming
        interaction.donate { [self] error in
            guard error == nil, let updated = try? content.updating(from: intent).mutableCopy() as? UNMutableNotificationContent else {
                finish(content); return
            }
            updated.attachments = content.attachments
            // Communication decoration must not replace the server's account badge.
            updated.badge = content.badge
            finish(updated)
        }
    }

    override func serviceExtensionTimeWillExpire() {
        download?.cancel()
        lock.lock(); let fallback = original; lock.unlock()
        if let fallback { finish(fallback) }
    }

    private func finish(_ content: UNNotificationContent) {
        lock.lock(); let callback = handler; handler = nil; lock.unlock()
        if let callback { callback(content) }
        else { for attachment in content.attachments { try? FileManager.default.removeItem(at: attachment.url) } }
    }

    static func intent(for content: UNNotificationContent) -> INSendMessageIntent? {
        let data = content.userInfo
        if let kind = data["kind"] as? String, kind == "space.invitation" || kind == "channel.invitation" || kind == "direct.request" { return nil }
        guard let senderID = data["senderId"] as? String, !senderID.isEmpty,
              let name = data["sender"] as? String, !name.isEmpty,
              let index = data["senderAvatarId"] as? Int, (0..<800).contains(index),
              let image = UIImage(named: "caper-avatar-\(index)", in: Bundle(for: NotificationService.self), compatibleWith: nil),
              let png = image.pngData() else { return nil }
        let direct = data["conversationId"] as? String
        let channel = data["channelId"] as? String
        guard let conversation = direct ?? channel, !conversation.isEmpty else { return nil }
        let avatar = INImage(imageData: png)
        let sender = INPerson(personHandle: INPersonHandle(value: senderID, type: .unknown),
            nameComponents: nil, displayName: name, image: avatar, contactIdentifier: nil, customIdentifier: senderID)
        let group = direct == nil ? data["conversationTitle"] as? String : nil
        let intent = INSendMessageIntent(recipients: nil, outgoingMessageType: .outgoingMessageText,
            content: content.body, speakableGroupName: group.map { INSpeakableString(spokenPhrase: $0) },
            conversationIdentifier: conversation, serviceName: nil, sender: sender, attachments: nil)
        let metadata = INSendMessageIntentDonationMetadata()
        metadata.recipientCount = 1
        if let group, !group.isEmpty {
            guard let count = data["recipientCount"] as? Int, count > 0 else { return nil }
            metadata.recipientCount = count
            // Caper has no separate group artwork: show the actual sender.
            intent.setImage(avatar, forParameterNamed: \.speakableGroupName)
        }
        // `@feedback` reaches the team like a direct mention.
        metadata.mentionsCurrentUser = ["mention.user", "feedback"].contains(data["kind"] as? String ?? "")
        intent.donationMetadata = metadata
        return intent
    }
}
