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
             "tracks": [["id": "private-track", "kind": "microphone"]]
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
