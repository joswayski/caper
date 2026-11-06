import AppKit
import XCTest
@testable import CaperCore

@MainActor final class MemberPanelTests: XCTestCase {
    func testLargeRosterRealizesOnlyVisibleMembersAndSearchWorksAfterScrolling() async throws {
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let members = (0..<3000).map { Member(id: "member-\($0)", username: "person\($0)", displayName: "Member \($0)", owner: false) }
        model.selectedSpaceID = "space"
        model.detail = SpaceDetail(space: Space(id: "space", name: "Example", ownerId: "owner"), channels: [], members: members)
        await model.presence.watch(spaceID: "space", members: members)
        model.presence.receive(["type": "snapshot", "members": members.map { ["userId": $0.id, "status": "online"] }], generation: 1)
        let panel = MemberPanel(model: model)
        panel.frame = NSRect(x: 0, y: 0, width: 220, height: 400)
        panel.layoutSubtreeIfNeeded()
        let scroll = try XCTUnwrap(panel.subviews.compactMap { $0 as? NSScrollView }.first)
        let document = try XCTUnwrap(scroll.documentView)
        func rows() -> [NSView] { document.subviews.filter { $0.accessibilityIdentifier().hasPrefix("member-") }.sorted { $0.frame.minY < $1.frame.minY } }
        XCTAssertLessThan(rows().count, 12, "The accessibility tree and avatar lifetime follow only the visible roster")
        XCTAssertEqual(rows().first?.accessibilityLabel(), "Member 0, Online")
        scroll.contentView.scroll(to: NSPoint(x: 0, y: document.bounds.height - scroll.contentSize.height))
        panel.layoutSubtreeIfNeeded()
        XCTAssertLessThan(rows().count, 12)
        XCTAssertEqual(rows().last?.accessibilityLabel(), "Member 2999, Online")
        model.presence.query = "person17"
        let searched = expectation(for: NSPredicate { _, _ in
            MainActor.assumeIsolated { rows().contains { $0.accessibilityLabel() == "Member 17, Online" } }
        }, evaluatedWith: panel)
        await fulfillment(of: [searched], timeout: 3)
        XCTAssertLessThan(rows().count, 12)
        panel.setVisible(false)
        XCTAssertTrue(document.subviews.isEmpty, "Closing the panel releases offscreen avatars and accessibility rows")
        XCTAssertEqual(model.presence.query, "")
        await model.presence.stop()
    }

    func testPresenceGroupsKeepIdleOnlineAndUnknownSeparateFromOffline() async throws {
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let members = (0..<4).map { Member(id: "member-\($0)", username: "person\($0)", displayName: "Member \($0)", owner: false) }
        model.selectedSpaceID = "space"
        model.detail = SpaceDetail(space: Space(id: "space", name: "Example", ownerId: "owner"), channels: [], members: members)
        await model.presence.watch(spaceID: "space", members: members)
        model.presence.receive(["type": "snapshot", "members": [
            ["userId": "member-0", "status": "offline"], ["userId": "member-1", "status": "idle"],
            ["userId": "member-2", "status": "online"]]], generation: 1)
        let panel = MemberPanel(model: model)
        panel.frame = NSRect(x: 0, y: 0, width: 220, height: 400)
        panel.layoutSubtreeIfNeeded()
        let scroll = try XCTUnwrap(panel.subviews.compactMap { $0 as? NSScrollView }.first)
        let children = try XCTUnwrap(scroll.documentView).subviews.sorted { $0.frame.minY < $1.frame.minY }
        XCTAssertEqual(children.map { ($0 as? NSTextField)?.stringValue ?? $0.accessibilityLabel() ?? "" }, [
            "Online  2", "Member 1, Idle", "Member 2, Online", "Offline  1", "Member 0, Offline",
            "Status unavailable  1", "Member 3, Status unavailable"])
        await model.presence.stop()
    }
}
