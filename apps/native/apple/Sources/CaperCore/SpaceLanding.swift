import Foundation

/// The space and channel the account last opened, kept across launches like
/// web's `caper:space-landing:<account>`. IDs only, per account and device;
/// direct messages, visits and unjoined previews are never stored.
public struct SpaceLanding: Codable, Equatable, Sendable {
    public var spaceID: String
    public var channelID: String?

    public init(spaceID: String, channelID: String?) {
        self.spaceID = spaceID
        self.channelID = channelID
    }

    static func key(accountID: String) -> String { "caper.space-landing.\(accountID)" }

    public static func load(accountID: String, defaults: UserDefaults = .standard) -> SpaceLanding? {
        guard !accountID.isEmpty, let data = defaults.data(forKey: key(accountID: accountID)) else { return nil }
        // Missing or corrupt storage falls back to the first joined channel.
        guard let landing = try? JSONDecoder().decode(SpaceLanding.self, from: data), !landing.spaceID.isEmpty else { return nil }
        return landing
    }

    public static func save(_ landing: SpaceLanding, accountID: String, defaults: UserDefaults = .standard) {
        guard !accountID.isEmpty, !landing.spaceID.isEmpty, let data = try? JSONEncoder().encode(landing) else { return }
        defaults.set(data, forKey: key(accountID: accountID))
    }

    /// The space launch opens: the one already selected, then a test override,
    /// then the saved landing while it is still in the list, else the first.
    public static func initialSpaceID(spaceIDs: [String], selected: String?, preferred: String?, saved: SpaceLanding?) -> String? {
        for candidate in [selected, preferred, saved?.spaceID] {
            if let candidate, spaceIDs.contains(candidate) { return candidate }
        }
        return spaceIDs.first
    }
}
