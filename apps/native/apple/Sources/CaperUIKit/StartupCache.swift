import CaperCore
import Foundation

/// The last session's workspace, so launch paints a real conversation before
/// the network answers. It is a display snapshot only: the live AppModel
/// replaces it as soon as it is ready, and logout deletes it.
struct StartupSnapshot: Codable {
    static let version = 1
    var version = Self.version
    let origin: String
    let accountID: String
    let spaces: [Space]
    let detail: SpaceDetail?
    let channelID: String?
    let channelName: String
    /// The newest channel messages, oldest first.
    let messages: [ChatMessage]
}

enum StartupCache {
    /// Enough to fill a tall phone; older rows load from the server.
    static let messageLimit = 60

    private static var url: URL? {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?
            .appendingPathComponent("caper-uikit-startup.json")
    }

    static func load(origin: URL) -> StartupSnapshot? {
        guard let url, let data = try? Data(contentsOf: url),
              let snapshot = try? JSONDecoder().decode(StartupSnapshot.self, from: data),
              snapshot.version == StartupSnapshot.version, snapshot.origin == origin.absoluteString else { return nil }
        return snapshot
    }

    @MainActor static func save(_ model: AppModel) {
        guard model.phase == .ready, let account = model.account, let url else { return }
        let chat = model.chat
        let channelID = model.selectedDirectMessageID == nil && model.visit == nil && model.openingChannelID == nil
            ? model.selectedChannelID : nil
        // Only the open channel's latest page: never a jumped-to middle window.
        let messages = channelID != nil && !chat.hasNewer && !chat.loading
            ? chat.displayedChannelMessages.filter { $0.channelId == channelID }.suffix(messageLimit) : []
        let snapshot = StartupSnapshot(
            origin: CaperUIKitLaunch.origin.absoluteString, accountID: account.id, spaces: model.spaces, detail: model.detail,
            channelID: channelID, channelName: chat.channelName, messages: Array(messages))
        guard let data = try? JSONEncoder().encode(snapshot) else { return }
        // Readable after first unlock only, like the Keychain session.
        try? data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    static func clear() {
        guard let url else { return }
        try? FileManager.default.removeItem(at: url)
    }
}
