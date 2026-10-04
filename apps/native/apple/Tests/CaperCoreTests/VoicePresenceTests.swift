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

    private func snapshot(_ revision: Int, name: String) -> [String: Any] {
        ["type": "snapshot", "revision": revision, "participants": [
            ["id": "guest", "name": name, "muted": true, "deafened": false,
             "tracks": [["id": "private-track", "kind": "microphone"]]]
        ]]
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
