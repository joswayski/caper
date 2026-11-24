import Foundation

/// Unsent composer text kept on this device across launches and updates, as
/// web keeps it in `localStorage`: per account, conversation drafts by
/// channel or DM ID and thread replies by root message ID. Logout removes the
/// account's, so another account on the device never sees them. Text only;
/// staged files and reply targets stay in memory. Never sent to the server.
struct SavedDrafts: Codable, Equatable {
    var conversations: [String: String] = [:]
    var threads: [String: String] = [:]

    static func key(accountID: String) -> String { "caper.drafts.\(accountID)" }

    /// Missing or corrupt storage means no drafts.
    static func load(accountID: String, defaults: UserDefaults) -> SavedDrafts {
        guard !accountID.isEmpty, let data = defaults.data(forKey: key(accountID: accountID)),
              let saved = try? JSONDecoder().decode(SavedDrafts.self, from: data) else { return SavedDrafts() }
        return saved
    }

    /// Blank drafts are dropped; with none left the entry goes.
    static func save(_ drafts: SavedDrafts, accountID: String, defaults: UserDefaults) {
        guard !accountID.isEmpty else { return }
        let kept = SavedDrafts(conversations: drafts.conversations.filter { !$0.value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty },
                               threads: drafts.threads.filter { !$0.value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
        if kept.conversations.isEmpty && kept.threads.isEmpty {
            defaults.removeObject(forKey: key(accountID: accountID))
        } else if let data = try? JSONEncoder().encode(kept) {
            defaults.set(data, forKey: key(accountID: accountID))
        }
    }

    static func remove(accountID: String, defaults: UserDefaults) {
        guard !accountID.isEmpty else { return }
        defaults.removeObject(forKey: key(accountID: accountID))
    }
}
