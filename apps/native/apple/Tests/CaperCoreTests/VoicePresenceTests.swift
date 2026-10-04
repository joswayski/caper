import XCTest
@testable import CaperCore

@MainActor
final class VoicePresenceTests: XCTestCase {
    private func model() -> VoicePresenceModel {
        VoicePresenceModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
    }

    private func channel(_ number: Int) -> Channel {
        Channel(id: "channel-\(number)", spaceId: "space-a", name: "channel-\(number)", private: false)
    }

    private func snapshot(_ revision: Int, name: String, sessionStartedAt: Any? = nil) -> [String: Any] {
        var value: [String: Any] = ["type": "snapshot", "revision": revision, "participants": [
            ["id": "guest", "name": name, "muted": true, "deafened": false,
             "tracks": [["id": "private-track", "kind": "microphone"]]]
        ]]
        if let sessionStartedAt { value["sessionStartedAt"] = sessionStartedAt }
        return value
    }

    func testBoundedReadOnlyRostersRejectOldSnapshotsAndRevokedChannels() async {
        let presence = model()
        let channels = (0..<25).map { channel($0) }
        await presence.watch(spaceID: "space-a", channels: channels, demo: false)
        presence.receive(snapshot(7, name: "first"), generation: 1, channelID: channels[0].id)
        presence.receive(snapshot(6, name: "stale"), generation: 1, channelID: channels[0].id)
        XCTAssertEqual(presence.roster(for: channels[0].id).map(\.name), ["first"])
        presence.receive(snapshot(1, name: "unwatched"), generation: 1, channelID: channels[24].id)
        XCTAssertTrue(presence.roster(for: channels[24].id).isEmpty, "25th channel cannot add a 25th subscription")
        presence.revoke(channelID: channels[0].id)
        presence.receive(snapshot(8, name: "revoked"), generation: 1, channelID: channels[0].id)
        XCTAssertTrue(presence.roster(for: channels[0].id).isEmpty)
        await presence.watch(spaceID: "space-b", channels: [channel(1)], demo: false)
        presence.receive(snapshot(9, name: "old space"), generation: 1, channelID: channels[1].id)
        XCTAssertTrue(presence.roster(for: channels[1].id).isEmpty)
        await presence.stop()
        presence.receive(snapshot(10, name: "disconnected"), generation: 3, channelID: channels[1].id)
        XCTAssertTrue(presence.rosters.isEmpty)
    }

    func testDemoWatchesOneRosterAndNeverTreatsTracksAsLocalAudio() async {
        let presence = model()
        await presence.watch(spaceID: "demo", channels: [channel(0), channel(1)], demo: true)
        presence.receive(snapshot(1, name: "spectator"), generation: 1, channelID: channel(0).id)
        XCTAssertEqual(presence.roster(for: channel(0).id).first?.name, "spectator")
        XCTAssertTrue(presence.roster(for: channel(1).id).isEmpty)
        await presence.stop()
        XCTAssertTrue(presence.rosters.isEmpty)
    }

    func testSessionTimestampIsRevisionFencedAndCleared() async {
        let presence = model()
        let watched = channel(0)
        await presence.watch(spaceID: "space-a", channels: [watched], demo: false)
        presence.receive(snapshot(2, name: "current", sessionStartedAt: 1_700_000_000_123 as NSNumber), generation: 1, channelID: watched.id)
        XCTAssertEqual(presence.sessionStartedAt(for: watched.id), 1_700_000_000_123)
        presence.receive(snapshot(1, name: "stale", sessionStartedAt: 9 as NSNumber), generation: 1, channelID: watched.id)
        XCTAssertEqual(presence.sessionStartedAt(for: watched.id), 1_700_000_000_123)
        presence.receive(snapshot(3, name: "legacy"), generation: 1, channelID: watched.id)
        XCTAssertNil(presence.sessionStartedAt(for: watched.id), "absent fields from older servers clear the timer")
        presence.receive(snapshot(4, name: "current", sessionStartedAt: 12 as NSNumber), generation: 1, channelID: watched.id)
        presence.receive(snapshot(5, name: "empty", sessionStartedAt: NSNull()), generation: 1, channelID: watched.id)
        XCTAssertNil(presence.sessionStartedAt(for: watched.id), "an explicit null clears the timer")
        presence.receive(snapshot(6, name: "current", sessionStartedAt: 12 as NSNumber), generation: 1, channelID: watched.id)
        presence.revoke(channelID: watched.id)
        XCTAssertNil(presence.sessionStartedAt(for: watched.id))
        await presence.stop()
        XCTAssertTrue(presence.sessionStartedAt.isEmpty)
    }

    func testAuthenticatedSnapshotParsingIsBackwardsCompatible() throws {
        let current = try JSONDecoder().decode(VoiceSnapshot.self, from: Data(#"{"participants":[],"revision":4,"sessionStartedAt":1700000000123}"#.utf8))
        XCTAssertEqual(current.sessionStartedAt, 1_700_000_000_123)
        let legacy = try JSONDecoder().decode(VoiceSnapshot.self, from: Data(#"{"participants":[],"revision":5}"#.utf8))
        XCTAssertNil(legacy.sessionStartedAt)
        let empty = try JSONDecoder().decode(VoiceSnapshot.self, from: Data(#"{"participants":[],"revision":6,"sessionStartedAt":null}"#.utf8))
        XCTAssertNil(empty.sessionStartedAt)
    }

    func testVoiceSessionDurationBoundariesAndFutureClamp() {
        let now = Date(timeIntervalSince1970: 10_000)
        func formatted(_ seconds: Int) -> String {
            VoiceSessionDuration.format(startedAtMilliseconds: (now.timeIntervalSince1970 - Double(seconds)) * 1_000, now: now)
        }
        XCTAssertEqual(formatted(0), "00:00")
        XCTAssertEqual(formatted(59), "00:59")
        XCTAssertEqual(formatted(60), "01:00")
        XCTAssertEqual(formatted(3_599), "59:59")
        XCTAssertEqual(formatted(3_600), "1:00:00")
        XCTAssertEqual(formatted(36_061), "10:01:01")
        XCTAssertEqual(VoiceSessionDuration.format(startedAtMilliseconds: 10_001_000, now: now), "00:00")
    }
}

@MainActor
final class PresenceTests: XCTestCase {
    func testSameSpaceKeepsMemberPageAndLiveStatusesButChangedSubscriptionsReset() async {
        let presence = PresenceModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let members = (0..<27).map { Member(id: "member-\($0)", username: "user\($0)", displayName: "Member \($0)", owner: false) }
        await presence.watch(spaceID: "space-a", members: members)
        await presence.showPage(1)
        let ids: Set<String> = ["member-25", "member-26"]
        presence.receive(["type": "snapshot", "members": [
            ["userId": "member-25", "status": "online"],
            ["userId": "member-26", "status": "idle"]
        ]], generation: 2, expectedIDs: ids)
        let renamed = members.map { Member(id: $0.id, username: $0.username, displayName: "Updated \($0.displayName)", owner: $0.owner, avatarId: 31) }
        await presence.watch(spaceID: "space-a", members: renamed)
        XCTAssertEqual(presence.page, 1)
        XCTAssertTrue(presence.online)
        XCTAssertEqual(presence.statuses, ["member-25": .online, "member-26": .idle])
        XCTAssertEqual(presence.visibleMembers.first?.displayName, "Updated Member 25")
        XCTAssertEqual(presence.visibleMembers.first?.avatarId, 31)

        await presence.watch(spaceID: "space-a", members: Array(renamed.prefix(26)))
        XCTAssertEqual(presence.page, 1)
        XCTAssertFalse(presence.online)
        XCTAssertTrue(presence.statuses.isEmpty, "a changed member page needs a new snapshot")
        presence.receive(["type": "snapshot", "members": [["userId": "member-25", "status": "offline"]]], generation: 3, expectedIDs: ["member-25"])
        XCTAssertEqual(presence.statuses["member-25"], .offline)

        await presence.watch(spaceID: "space-b", members: Array(renamed.prefix(26)))
        XCTAssertEqual(presence.page, 0)
        XCTAssertFalse(presence.online)
        XCTAssertTrue(presence.statuses.isEmpty, "statuses must not carry over to another space")
        presence.receive(["type": "snapshot", "members": [["userId": "member-25", "status": "online"]]], generation: 3, expectedIDs: ["member-25"])
        XCTAssertTrue(presence.statuses.isEmpty, "late old-space snapshots must be fenced")
        await presence.stop()
    }
}
