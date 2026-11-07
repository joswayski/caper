import AppKit
import XCTest
@testable import CaperCore

@MainActor final class NavigationSidebarTests: XCTestCase {
    func testVoiceCardPreservesFocusAndLiveRowWhenChannelsCollapse() async throws {
        _ = NSApplication.shared
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let text = Channel(id: "text", spaceId: "space", name: "general", private: false)
        let call = Channel(id: "call", spaceId: "space", name: "design", private: false)
        model.detail = SpaceDetail(space: Space(id: "space", name: "Fixture", ownerId: "owner"), channels: [text, call], members: [])
        model.selectedChannelID = text.id
        let data = try JSONSerialization.data(withJSONObject: (0..<12).map {
            ["id": "person-\($0)", "name": "Person \($0)", "muted": false, "deafened": false, "tracks": []] as [String: Any]
        })
        model.voice.displayRosterPreview(context: VoiceContext(channelID: call.id, channelName: call.name, spaceID: "space", spaceName: "Fixture"),
            selfID: "person-3", participants: try JSONDecoder().decode([VoiceParticipant].self, from: data), speaking: ["person-11"])
        let sidebar = NavigationSidebar(model: model)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 344, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = sidebar
        defer { model.leaveVoice(); window.close() }
        sidebar.layoutSubtreeIfNeeded()
        func visible(_ view: NSView) -> [NSView] {
            view.isHiddenOrHasHiddenAncestor ? [] : [view] + view.subviews.flatMap(visible)
        }
        func button(_ id: String) -> NSButton? {
            visible(sidebar).compactMap { $0 as? NSButton }.first { $0.accessibilityIdentifier() == id }
        }
        XCTAssertEqual(visible(sidebar).filter { $0.accessibilityIdentifier().hasPrefix("voice-face-") }.count, 9)
        XCTAssertEqual(button("voice-faces-more")?.accessibilityLabel(), "3 more in the call")
        let ownFace = try XCTUnwrap(button("voice-face-person-3"))
        let firstRemote = try XCTUnwrap(button("voice-face-person-0"))
        XCTAssertLessThan(ownFace.frame.minX, firstRemote.frame.minX, "Self leads join order, not speaking order")
        let deafen = try XCTUnwrap(button("voice-call-deafen"))
        XCTAssertTrue(window.makeFirstResponder(deafen))
        deafen.performClick(nil)
        let deafened = expectation(for: NSPredicate { _, _ in
            MainActor.assumeIsolated { button("voice-call-deafen")?.accessibilityValue() as? String == "Deafened" }
        }, evaluatedWith: sidebar)
        await fulfillment(of: [deafened], timeout: 3)
        XCTAssertTrue(window.firstResponder === deafen, "A voice update must not replace the focused control")
        XCTAssertTrue(model.voice.muted, "Deafen keeps the transport's mute semantics")
        XCTAssertEqual(model.selectedChannelID, text.id, "Call controls do not move the conversation")
        XCTAssertTrue(button("voice-face-person-3") === ownFace)
        try XCTUnwrap(button("channels-toggle")).performClick(nil)
        sidebar.layoutSubtreeIfNeeded()
        XCTAssertNil(button("channel-text"))
        XCTAssertNotNil(button("channel-call"), "Collapsing Channels must not hide its live call")
        sidebar.frame.size.width = 300
        sidebar.layoutSubtreeIfNeeded()
        XCTAssertEqual(button("voice-faces-more")?.accessibilityLabel(), "5 more in the call", "Narrow cards keep two rows, not ten overflowing faces")
        try XCTUnwrap(button("voice-call-leave")).performClick(nil)
        XCTAssertEqual(model.voice.phase, .idle)
        XCTAssertEqual(model.selectedChannelID, text.id)
    }

    func testElsewhereCallStaysAboveChannelsWithoutReplacingSelectedChat() throws {
        _ = NSApplication.shared
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let channel = Channel(id: "text", spaceId: "space", name: "general", private: false)
        model.detail = SpaceDetail(space: Space(id: "space", name: "Fixture", ownerId: "owner"), channels: [channel], members: [])
        model.selectedChannelID = channel.id
        CaperRuntime.showVoiceRosterPreview(model, environment: ["CAPER_TEST_MODE": "parity", "CAPER_UI_FIXTURE": "voice-elsewhere", "CAPER_API_BASE_URL": "http://127.0.0.1:3001"])
        defer { model.leaveVoice() }
        let sidebar = NavigationSidebar(model: model)
        sidebar.frame = NSRect(x: 0, y: 0, width: 344, height: 600)
        sidebar.layoutSubtreeIfNeeded()
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let card = try XCTUnwrap(descendants(sidebar).first { $0.accessibilityIdentifier() == "voice-call-card" })
        let heading = try XCTUnwrap(descendants(sidebar).compactMap { $0 as? NSButton }.first { $0.accessibilityIdentifier() == "channels-toggle" })
        XCTAssertLessThan(card.convert(card.bounds, to: sidebar).maxY, heading.convert(heading.bounds, to: sidebar).minY)
        heading.performClick(nil); sidebar.layoutSubtreeIfNeeded()
        XCTAssertNotNil(card.superview)
        XCTAssertEqual(model.selectedChannelID, channel.id)
        XCTAssertEqual(model.voice.context?.channelID, "fixture-elsewhere-lobby")
    }

    func testRequestNavigationKeepsFocusDuringSelectionRefresh() async throws {
        _ = NSApplication.shared
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        model.directMessages = [DirectMessageConversation(id: "request", peer: DirectMessagePeer(id: "jordan", username: "jordan", displayName: "Jordan"),
            lastSeq: "1", readSeq: "0", status: .incoming)]
        model.showingMessageRequests = true
        let sidebar = NavigationSidebar(model: model)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 344, height: 600),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = sidebar
        defer { window.close() }
        sidebar.layoutSubtreeIfNeeded()
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        func request() -> NSButton? {
            descendants(sidebar).compactMap { $0 as? NSButton }.first { $0.accessibilityIdentifier() == "message-request-request" }
        }
        let button = try XCTUnwrap(request())
        XCTAssertTrue(window.makeFirstResponder(button))
        model.selectedDirectMessageID = "request"
        let selected = expectation(for: NSPredicate { _, _ in
            MainActor.assumeIsolated { request()?.accessibilityValue() as? String == "Selected" }
        }, evaluatedWith: sidebar)
        await fulfillment(of: [selected], timeout: 3)
        XCTAssertTrue(window.firstResponder === button, "Opening a request must not detach its focused navigation control")
    }

    func testCreateSpaceKeepsKeyboardFocusWhenNavigationRefreshes() async throws {
        _ = NSApplication.shared
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let sidebar = NavigationSidebar(model: model)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 344, height: 600),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = sidebar
        defer { window.close() }
        sidebar.layoutSubtreeIfNeeded()
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let button = try XCTUnwrap(descendants(sidebar).compactMap { $0 as? NSButton }
            .first { $0.accessibilityIdentifier() == "create-space" })
        XCTAssertTrue(window.makeFirstResponder(button))
        model.error = "Navigation refreshed"
        let refreshed = expectation(for: NSPredicate { _, _ in
            MainActor.assumeIsolated {
                descendants(sidebar).compactMap { $0 as? NSTextField }.contains { $0.stringValue == "Navigation refreshed" }
            }
        }, evaluatedWith: sidebar)
        await fulfillment(of: [refreshed], timeout: 3)
        XCTAssertTrue(window.firstResponder === button, "A model refresh must not remove the focused rail control")
        var opened: String?
        sidebar.show = { opened = $0.id }
        button.performClick(nil)
        XCTAssertEqual(opened, "create-space")
    }
}
