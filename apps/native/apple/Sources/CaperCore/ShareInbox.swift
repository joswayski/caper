import Foundation
#if os(macOS)
import Security
#endif

// Sharing into Caper from other apps (docs/media.md, "Sharing into Caper").
// The app and its Share extension exchange files here, in their App Group
// container, and nothing else: the extension never holds the account session.
// Foundation only, so the extensions compile this file on its own.

/// A channel or DM a share can go to, as `GET /api/chat/forward-destinations` lists it.
struct ShareDestination: Codable, Equatable, Sendable {
    let id: String
    let name: String
    let spaceName: String
    let spaceId: String?
    let direct: Bool

    /// What a picker row shows: `#general` or a person's name.
    var title: String { direct ? name : "#\(name)" }
}

/// The signed-in account's recent conversations, newest first, which the app
/// writes so the extension can offer them.
struct ShareRecents: Codable, Equatable, Sendable {
    let accountId: String
    let destinations: [ShareDestination]
}

/// One share the extension handed over: its files sit beside this record.
struct SharedItems: Codable, Equatable, Sendable {
    let accountId: String
    let destination: ShareDestination
    let text: String
    let files: [String]
    let createdAt: Date
}

enum ShareInbox {
    static let urlScheme = "caper"
    static let urlHost = "share"
    /// Shares older than this are dropped unopened.
    static let lifetime: TimeInterval = 24 * 60 * 60

    /// The App Group container's sharing folder, or nil when this build has no
    /// App Group (development builds): sharing is then unavailable.
    static var root: URL? {
        guard let group = groupIdentifier,
              let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group) else { return nil }
        return container.appendingPathComponent("Sharing", isDirectory: true)
    }

    /// macOS reads the group from the signature (Developer ID groups are
    /// team-prefixed, added at signing); iOS from Info.plist's `CaperAppGroup`.
    static var groupIdentifier: String? {
        #if os(macOS)
        guard let task = SecTaskCreateFromSelf(nil),
              let groups = SecTaskCopyValueForEntitlement(task, "com.apple.security.application-groups" as CFString, nil) as? [String] else { return nil }
        return groups.first
        #else
        return (Bundle.main.object(forInfoDictionaryKey: "CaperAppGroup") as? String).flatMap { $0.isEmpty ? nil : $0 }
        #endif
    }

    // MARK: Recent conversations

    static func saveRecents(_ recents: ShareRecents?, base: URL? = ShareInbox.root) {
        guard let root = base else { return }
        let file = root.appendingPathComponent("recents.json")
        guard let recents else { try? FileManager.default.removeItem(at: file); return }
        try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try? JSONEncoder().encode(recents).write(to: file, options: .atomic)
    }

    static func recents(base: URL? = ShareInbox.root) -> ShareRecents? {
        guard let root = base, let data = try? Data(contentsOf: root.appendingPathComponent("recents.json")) else { return nil }
        return try? JSONDecoder().decode(ShareRecents.self, from: data)
    }

    /// Destinations whose name or space contains every search word (a leading
    /// `#` is ignored), in their order: most recent first.
    static func matching(_ destinations: [ShareDestination], search: String) -> [ShareDestination] {
        let terms = search.split(whereSeparator: \.isWhitespace).map { $0.hasPrefix("#") ? String($0.dropFirst()) : String($0) }.filter { !$0.isEmpty }
        return destinations.filter { destination in
            terms.allSatisfy { "\(destination.spaceName) \(destination.name)".localizedCaseInsensitiveContains($0) }
        }
    }

    // MARK: Shares

    /// A new share's private folder, where the extension copies files as it reads them.
    static func newShareFolder(base: URL? = ShareInbox.root) throws -> URL {
        guard let root = base else { throw CocoaError(.fileNoSuchFile) }
        let folder = root.appendingPathComponent("Inbox", isDirectory: true).appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder
    }

    /// A safe, unused file name in [folder] for a shared file called [name].
    static func destination(for name: String?, in folder: URL) -> URL {
        let cleaned = (name ?? "").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: ":", with: "_")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let base = cleaned.isEmpty || cleaned == "." || cleaned == ".." || cleaned == "items.json" ? "file" : String(cleaned.prefix(200))
        var candidate = folder.appendingPathComponent(base)
        var index = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            let stem = (base as NSString).deletingPathExtension, ext = (base as NSString).pathExtension
            candidate = folder.appendingPathComponent(ext.isEmpty ? "\(stem) \(index)" : "\(stem) \(index).\(ext)")
            index += 1
        }
        return candidate
    }

    /// Finishes a share: written last, so the app never reads a half-copied one.
    static func complete(_ items: SharedItems, in folder: URL) throws {
        try JSONEncoder().encode(items).write(to: folder.appendingPathComponent("items.json"), options: .atomic)
    }

    /// The newest finished share; older and expired ones are removed.
    static func takeNewest(base: URL? = ShareInbox.root, now: Date = Date()) -> (items: SharedItems, folder: URL)? {
        guard let root = base else { return nil }
        let inbox = root.appendingPathComponent("Inbox", isDirectory: true)
        let folders = (try? FileManager.default.contentsOfDirectory(at: inbox, includingPropertiesForKeys: [.creationDateKey])) ?? []
        var shares: [(items: SharedItems, folder: URL)] = []
        for folder in folders {
            if let data = try? Data(contentsOf: folder.appendingPathComponent("items.json")),
               let items = try? JSONDecoder().decode(SharedItems.self, from: data) {
                if now.timeIntervalSince(items.createdAt) < lifetime { shares.append((items, folder)) } else { remove(folder) }
            } else if let created = try? folder.resourceValues(forKeys: [.creationDateKey]).creationDate,
                      now.timeIntervalSince(created) > 60 * 60 {
                // Abandoned while the extension was still copying.
                remove(folder)
            }
        }
        shares.sort { $0.items.createdAt > $1.items.createdAt }
        shares.dropFirst().forEach { try? FileManager.default.removeItem(at: $0.folder) }
        return shares.first
    }

    static func remove(_ folder: URL) { try? FileManager.default.removeItem(at: folder) }
}
