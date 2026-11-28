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

    // MARK: Files (rule 4)

    private func asset(_ id: String, spoiler: Bool = false) -> ChatAttachment {
        ChatAttachment(id: id, kind: .image, contentType: "image/webp", name: "\(id).webp", size: 2048, hasPreview: true,
                       url: "https://cdn.caper.chat/assets/\(id)?exp=2&sig=s", previewUrl: "https://cdn.caper.chat/assets/\(id)/preview?exp=2&sig=s",
                       spoiler: spoiler)
    }

    /// A file picked here: uploading until `uploaded` gives it its server copy.
    private func picked(_ name: String, uploaded: ChatAttachment? = nil, spoiler: Bool = false) -> AttachmentDraft {
        var draft = AttachmentDraft(id: "local-\(name)", name: name, kind: .image,
                                    localURL: URL(fileURLWithPath: "/tmp/\(name).webp"), sourceSize: 4096)
        draft.preparing = false; draft.spoiler = spoiler
        draft.original = LocalAttachmentFile(url: URL(fileURLWithPath: "/tmp/\(name).heic"), name: name, contentType: "image/heic", size: 4096)
        if let uploaded { draft.attachment = uploaded; draft.uploadedAt = Date(); draft.progress = 1 }
        return draft
    }

    func testASaveSendsCompletedUploadsInTileOrderWithTheirSpoilers() {
        let tiles = [picked("a", uploaded: asset("Asset0000001"), spoiler: true), picked("uploading", spoiler: true),
                     picked("b", uploaded: asset("Asset0000002"))]
        let files = DraftSync.fileIDs(tiles)
        XCTAssertEqual(files.ids, ["Asset0000001", "Asset0000002"], "a file still uploading isn't sent")
        XCTAssertEqual(files.spoilers, ["Asset0000001"])
        XCTAssertTrue(DraftSync.fileIDs([]).ids.isEmpty, "no files sends an empty list, which clears them")
    }

    func testServerFilesArriveAsReadySyncedTilesAndUploadsHereSurvive() {
        let own = picked("own", uploaded: asset("Asset0000001")), uploading = picked("uploading")
        var ids = ["new-1"]
        let merged = DraftSync.mergeFiles(local: [uploading, own], server: [asset("Asset0000002", spoiler: true), asset("Asset0000001", spoiler: true)],
                                          newID: { ids.removeFirst() })
        XCTAssertEqual(merged.files.map(\.id), ["new-1", "local-own", "local-uploading"], "server order, then files still uploading here")
        XCTAssertTrue(merged.dropped.isEmpty)
        let synced = merged.files[0]
        XCTAssertTrue(synced.synced)
        XCTAssertEqual(synced.attachment?.id, "Asset0000002")
        XCTAssertFalse(synced.preparing)
        XCTAssertNil(synced.original, "no local original")
        XCTAssertFalse(synced.needsUpload, "never uploaded here, nor renewed after 23 hours")
        XCTAssertTrue(synced.spoiler)
        XCTAssertEqual(synced.localURL.absoluteString, "https://cdn.caper.chat/assets/Asset0000002?exp=2&sig=s")
        XCTAssertEqual(synced.editPreviewURL?.absoluteString, "https://cdn.caper.chat/assets/Asset0000002/preview?exp=2&sig=s", "the server preview is its thumbnail")
        XCTAssertTrue(synced.editable, "Edit downloads it first")
        XCTAssertEqual(DraftSync.fileIDs(merged.files).ids, ["Asset0000002", "Asset0000001"], "a synced tile sends by its ID")
        XCTAssertFalse(merged.files[1].synced, "this device's own upload keeps its tile and original")
        XCTAssertNotNil(merged.files[1].original)
        XCTAssertTrue(merged.files[1].spoiler, "with the server's spoiler mark")
        XCTAssertEqual(merged.files[2], uploading)
    }

    func testAFileTheServerNoLongerListsIsDroppedAndAnEchoChangesNothing() {
        let own = picked("own", uploaded: asset("Asset0000001"))
        let synced = AttachmentDraft(synced: asset("Asset0000002"), id: "synced-1")!
        let removed = DraftSync.mergeFiles(local: [own, synced, picked("uploading")], server: [])
        XCTAssertEqual(removed.files.map(\.id), ["local-uploading"])
        XCTAssertEqual(removed.dropped.map(\.id), ["local-own", "synced-1"], "released as Remove does")

        let tiles = [own, synced]
        let echo = DraftSync.mergeFiles(local: tiles, server: tiles.compactMap(\.attachment), newID: { XCTFail("no new tile"); return "x" })
        XCTAssertEqual(echo.files, tiles, "the save's own echo changes nothing")
        XCTAssertTrue(echo.dropped.isEmpty)
        XCTAssertNil(AttachmentDraft(synced: ChatAttachment(id: "Asset0000003", kind: .file, contentType: "application/pdf", name: "a.pdf", size: 1), id: "x"),
                     "a file without a URL can't be shown")
    }

    func testFilesAreKnownOnceAServerDraftListsThemOrASaveReturnsThem() {
        var sync = DraftSync()
        XCTAssertTrue(sync.accept(server(general, "", at: "2026-11-27T18:00:00.000001Z")))
        XCTAssertFalse(sync.filesKnown.contains(general), "a server without file sync says nothing about files")
        var listed = server(reply, "", at: "2026-11-27T18:00:00.000001Z"); listed.attachments = [asset("Asset0000001")]
        XCTAssertTrue(sync.accept(listed))
        XCTAssertTrue(sync.filesKnown.contains(reply))

        var saved = server(general, "typed", at: "2026-11-27T18:00:01.000000Z"); saved.attachments = [asset("Asset0000002")]
        XCTAssertTrue(sync.saved(saved, unchanged: true), "a save that left files out shows the ones it returns")
        XCTAssertFalse(sync.saved(saved, unchanged: true), "once")
    }

    func testDraftFilesDecodeSkippingMalformedOnes() throws {
        let json = #"""
        {"channelId":"chan00000001","threadRootId":null,"text":"look","updatedAt":"2026-11-27T18:04:05.123456Z","attachments":[
          {"id":"Asset0000001","kind":"image","contentType":"image/webp","name":"a.webp","size":10,"url":"https://cdn.caper.chat/a","spoiler":true},
          {"id":"","kind":"image","contentType":"image/webp","name":"bad.webp","size":10}
        ]}
        """#
        let draft = try JSONDecoder().decode(ServerDraft.self, from: Data(json.utf8))
        XCTAssertEqual(draft.attachments?.map(\.id), ["Asset0000001"])
        XCTAssertEqual(draft.attachments?.first?.spoiler, true)
        let older = try JSONDecoder().decode(ServerDraft.self, from: Data(#"{"channelId":"c","text":"","updatedAt":"x"}"#.utf8))
        XCTAssertNil(older.attachments)
        XCTAssertNil(older.threadRootId)
    }
}
