import Intents
import UIKit
import UserNotifications

/// Uses only shipped artwork and the push payload. No account token, shared
/// keychain, network request, or WebRTC dependency is needed in this process.
final class NotificationService: UNNotificationServiceExtension {
    private let lock = NSLock()
    private var handler: ((UNNotificationContent) -> Void)?
    private var original: UNNotificationContent?

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        lock.lock(); handler = contentHandler; original = request.content; lock.unlock()
        guard let intent = Self.intent(for: request.content) else { finish(request.content); return }
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .incoming
        interaction.donate { [self] error in
            guard error == nil, let updated = try? request.content.updating(from: intent) else {
                finish(request.content); return
            }
            finish(updated)
        }
    }

    override func serviceExtensionTimeWillExpire() {
        lock.lock(); let fallback = original; lock.unlock()
        if let fallback { finish(fallback) }
    }

    private func finish(_ content: UNNotificationContent) {
        lock.lock(); let callback = handler; handler = nil; lock.unlock()
        callback?(content)
    }

    static func intent(for content: UNNotificationContent) -> INSendMessageIntent? {
        let data = content.userInfo
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
            intent.setImage(avatar, forParameterNamed: "speakableGroupName")
        }
        metadata.mentionsCurrentUser = data["kind"] as? String == "mention.user"
        intent.donationMetadata = metadata
        return intent
    }
}
