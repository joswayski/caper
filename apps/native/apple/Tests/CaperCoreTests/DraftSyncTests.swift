import XCTest
@testable import CaperCore

/// The cross-device draft rules (docs/media.md, "Draft sync") without timers or storage.
final class DraftSyncTests: XCTestCase {
    private let general = DraftKey(channelID: "chan00000001", threadRootID: nil)
    private let reply = DraftKey(channelID: "chan00000001", threadRootID: "Message00000001")

    private func server(_ key: DraftKey, _ text: String, at updatedAt: String) -> ServerDraft {
        ServerDraft(channelId: key.channelID, threadRootId: key.threadRootID, text: text, updatedAt: updatedAt)
    }

    func testServerDraftsApplyOnlyWhenCleanAndNewer() {
        var sync = DraftSync()
        XCTAssertTrue(sync.accept(server(general, "from web", at: "2026-11-27T18:04:05.000002Z")))
        XCTAssertFalse(sync.accept(server(general, "same", at: "2026-11-27T18:04:05.000002Z")), "not newer")
        XCTAssertFalse(sync.accept(server(general, "older", at: "2026-11-27T18:04:05.000001Z")), "older")
        XCTAssertTrue(sync.accept(server(reply, "a reply", at: "2026-11-27T18:04:05.000001Z")), "each draft keeps its own updatedAt")

        sync.dirty.insert(general)
        XCTAssertFalse(sync.accept(server(general, "elsewhere", at: "2026-11-27T19:00:00.000000Z")), "a dirty draft keeps its local edit")
        XCTAssertEqual(sync.applied[general], "2026-11-27T18:04:05.000002Z", "a refused draft records nothing")
    }

    func testASavesResponseCountsAsAppliedSoItsEchoChangesNothing() {
        var sync = DraftSync()
        sync.dirty = [general]
        let saved = server(general, "typed here", at: "2026-11-27T18:04:06.000000Z")
        sync.saved(saved, unchanged: true)
        XCTAssertTrue(sync.dirty.isEmpty)
        XCTAssertFalse(sync.accept(saved), "the save's own draft.updated echo")

        sync.dirty = [reply]
        sync.saved(server(reply, "first", at: "2026-11-27T18:04:07.000000Z"), unchanged: false)
        XCTAssertEqual(sync.dirty, [reply], "an edit made while the save was in flight stays dirty for its own save")
        sync.saved(server(general, "late", at: "2026-11-27T18:04:01.000000Z"), unchanged: true)
        XCTAssertEqual(sync.applied[general], "2026-11-27T18:04:06.000000Z", "an older response never moves applied back")
    }

    func testDeviceOnlyNonEmptyDraftsAreUploadedAfterLoad() {
        let other = DraftKey(channelID: "dm0000000001", threadRootID: nil)
        let device = [general: "typed offline", reply: "known", other: ""]
        let missing = DraftSync.deviceOnly(device, server: [server(reply, "", at: "2026-11-27T18:00:00.000000Z")])
        XCTAssertEqual(missing, [general], "a server entry (even a cleared one) or empty text is not uploaded")
    }

    func testDraftKeysRoundTripAsWebStoresThem() {
        XCTAssertEqual(general.id, "chan00000001")
        XCTAssertEqual(reply.id, "chan00000001 Message00000001")
        XCTAssertEqual(DraftKey(id: general.id), general)
        XCTAssertEqual(DraftKey(id: reply.id), reply)
    }

    func testSavedDraftsKeepDirtyMarksAndReadOlderStorage() throws {
        let suite = "DraftSync.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set(Data(#"{"conversations":{"chan00000001":"old build"},"threads":{}}"#.utf8), forKey: SavedDrafts.key(accountID: "alice"))
        let legacy = SavedDrafts.load(accountID: "alice", defaults: defaults)
        XCTAssertEqual(legacy.conversations, ["chan00000001": "old build"])
        XCTAssertNil(legacy.dirty)

        // A cleared, unsaved draft keeps only its mark so the clear still reaches the server.
        SavedDrafts.save(SavedDrafts(conversations: ["chan00000001": ""], threads: ["Message00000001": "reply", "Message00000002": " "],
                                     threadChannels: ["Message00000001": "chan00000001", "Message00000002": "chan00000001",
                                                      "Message00000003": "chan00000002"],
                                     dirty: [general.id]),
                         accountID: "alice", defaults: defaults)
        let saved = SavedDrafts.load(accountID: "alice", defaults: defaults)
        XCTAssertTrue(saved.conversations.isEmpty)
        XCTAssertEqual(saved.threads, ["Message00000001": "reply"])
        XCTAssertEqual(saved.threadChannels, ["Message00000001": "chan00000001"])
        XCTAssertEqual(saved.dirty, [general.id])

        SavedDrafts.save(SavedDrafts(), accountID: "alice", defaults: defaults)
        XCTAssertNil(defaults.data(forKey: SavedDrafts.key(accountID: "alice")), "nothing left removes the entry")
    }

    func testDraftUpdatedEventsParse() {
        let event = ProfileFeedEvent.parse(["type": "draft.updated", "draft": [
            "channelId": "chan00000001", "threadRootId": NSNull(), "text": "hello", "updatedAt": "2026-11-27T18:04:05.123456Z",
        ] as [String: Any]])
        XCTAssertEqual(event, .draftUpdated(server(general, "hello", at: "2026-11-27T18:04:05.123456Z")))
        XCTAssertNil(ProfileFeedEvent.parse(["type": "draft.updated", "draft": ["channelId": "", "text": "x", "updatedAt": "now"]]))
        XCTAssertNil(ProfileFeedEvent.parse(["type": "draft.updated", "draft": ["channelId": "chan00000001", "text": "x"]]))
        XCTAssertEqual(ProfileFeedEvent.parse(["type": "ready"]), .ready)
    }
}
