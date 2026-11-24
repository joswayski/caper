import XCTest
@testable import CaperCore

/// The hand-off between the Share extension and the app (ShareInbox).
final class ShareInboxTests: XCTestCase {
    private var base: URL!

    override func setUpWithError() throws {
        base = FileManager.default.temporaryDirectory.appendingPathComponent("share-inbox-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDownWithError() throws { try? FileManager.default.removeItem(at: base) }

    private let general = ShareDestination(id: "chan00000001", name: "general", spaceName: "Studio", spaceId: "space0000001", direct: false)

    func testOnlyTheNewestFinishedShareIsTakenAndUnfinishedOnesWait() throws {
        let now = Date()
        let older = try ShareInbox.newShareFolder(base: base)
        try ShareInbox.complete(SharedItems(accountId: "a", destination: general, text: "older", files: [], createdAt: now.addingTimeInterval(-60)), in: older)
        let newer = try ShareInbox.newShareFolder(base: base)
        try Data("x".utf8).write(to: newer.appendingPathComponent("photo.heic"))
        try ShareInbox.complete(SharedItems(accountId: "a", destination: general, text: "newer", files: ["photo.heic"], createdAt: now), in: newer)
        // Still being copied by the extension: no items.json yet.
        let copying = try ShareInbox.newShareFolder(base: base)

        let taken = try XCTUnwrap(ShareInbox.takeNewest(base: base, now: now))
        XCTAssertEqual(taken.items.text, "newer")
        XCTAssertEqual(taken.folder, newer)
        XCTAssertFalse(FileManager.default.fileExists(atPath: older.path), "an older share is dropped")
        XCTAssertTrue(FileManager.default.fileExists(atPath: copying.path), "a share still being written is left alone")
    }

    func testExpiredSharesAreDropped() throws {
        let folder = try ShareInbox.newShareFolder(base: base)
        try ShareInbox.complete(SharedItems(accountId: "a", destination: general, text: "", files: [], createdAt: Date(timeIntervalSinceNow: -ShareInbox.lifetime - 1)), in: folder)
        XCTAssertNil(ShareInbox.takeNewest(base: base))
        XCTAssertFalse(FileManager.default.fileExists(atPath: folder.path))
    }

    func testSharedFileNamesStayInsideTheirFolderAndNeverCollide() throws {
        let folder = try ShareInbox.newShareFolder(base: base)
        let first = ShareInbox.destination(for: "../../secret.txt", in: folder)
        XCTAssertEqual(first.deletingLastPathComponent().standardizedFileURL, folder.standardizedFileURL)
        try Data().write(to: first)
        XCTAssertNotEqual(ShareInbox.destination(for: "../../secret.txt", in: folder), first)
        XCTAssertEqual(ShareInbox.destination(for: "items.json", in: folder).lastPathComponent, "file")
    }

    func testSearchMatchesEveryWordAndKeepsRecentOrder() {
        let ana = ShareDestination(id: "dm0000000001", name: "Ana Lopez", spaceName: "Direct messages", spaceId: nil, direct: true)
        let design = ShareDestination(id: "chan00000002", name: "design", spaceName: "Studio", spaceId: "space0000001", direct: false)
        let recent = [design, ana, general]
        XCTAssertEqual(ShareInbox.matching(recent, search: " "), recent)
        XCTAssertEqual(ShareInbox.matching(recent, search: "studio"), [design, general])
        XCTAssertEqual(ShareInbox.matching(recent, search: "#gen studio"), [general])
        XCTAssertEqual(ana.title, "Ana Lopez")
        XCTAssertEqual(general.title, "#general")
    }

    func testRecentsRoundTripAndClear() {
        let recents = ShareRecents(accountId: "a", destinations: [general])
        ShareInbox.saveRecents(recents, base: base)
        XCTAssertEqual(ShareInbox.recents(base: base), recents)
        ShareInbox.saveRecents(nil, base: base)
        XCTAssertNil(ShareInbox.recents(base: base))
    }
}
