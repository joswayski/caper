import AppKit
import XCTest
@testable import CaperCore

@MainActor final class MemberPanelTests: XCTestCase {
    func testPanelResizeKeyboardAndAccessibilityRespectDirectionAndLiveBounds() throws {
        for widensToLeft in [true, false] {
            var committed: [CGFloat] = []
            let handle = NativePanelResizeHandle("Panel width", identifier: "panel-resize", defaultWidth: 280, widensToLeft: widensToLeft) { width, commit in
                if commit { committed.append(width) }
            }
            func key(_ code: UInt16) throws {
                let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
                    windowNumber: 0, context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: code))
                handle.keyDown(with: event)
            }
            XCTAssertEqual(handle.accessibilityRole(), .slider)
            handle.update(width: 333, maximum: 440)
            XCTAssertTrue(committed.isEmpty, "Window layout must not save the temporarily clamped width")
            try key(123)
            XCTAssertEqual(committed.last, widensToLeft ? 343 : 323)
            try key(124)
            XCTAssertEqual(committed.last, 333)
            XCTAssertTrue(handle.accessibilityPerformIncrement())
            XCTAssertEqual(committed.last, 343, "AX increase always widens, regardless of which edge is dragged")
            XCTAssertTrue(handle.accessibilityPerformDecrement())
            XCTAssertEqual(committed.last, 333)
            try key(115)
            XCTAssertEqual(committed.last, 220)
            try key(119)
            XCTAssertEqual(committed.last, 440)
            handle.update(width: 440, maximum: 225.5)
            XCTAssertEqual(committed.last, 440, "Clamping is a display change, not a new preference")
            XCTAssertEqual(handle.accessibilityValue() as? String, "225 pixels")
            XCTAssertTrue(handle.accessibilityPerformDecrement())
            XCTAssertEqual(committed.last, 220)
            XCTAssertTrue(handle.accessibilityPerformIncrement())
            XCTAssertEqual(committed.last, 225.5, "An interval smaller than one key step remains adjustable")
            handle.update(width: 440, maximum: 220)
            try key(119)
            XCTAssertTrue(handle.accessibilityPerformIncrement())
            XCTAssertEqual(committed.last, 220)
            XCTAssertEqual(handle.accessibilityValue() as? String, "220 pixels")
        }
    }

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
        for width in [CGFloat(370), 440, 220] {
            panel.frame.size.width = width
            panel.layoutSubtreeIfNeeded()
            XCTAssertEqual(scroll.frame.width, width)
            XCTAssertLessThan(rows().count, 12, "Resizing must not realize the full roster")
            XCTAssertEqual(rows().first?.accessibilityLabel(), "Member 0, Online")
            XCTAssertTrue(rows().allSatisfy { $0.frame.minX >= 0 && $0.frame.maxX <= document.bounds.width })
        }
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
