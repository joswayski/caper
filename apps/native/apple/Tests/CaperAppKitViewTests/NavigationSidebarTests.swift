import AppKit
import XCTest
@testable import CaperCore

@MainActor final class NavigationSidebarTests: XCTestCase {
    func testActivityEntryRetainsFocusAndUpdatesUnreadWithoutAcknowledging() async throws {
        _ = NSApplication.shared
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        model.account = Account(id: "owner", username: "owner", displayName: "Owner")
        model.activityUnread = 17
        let sidebar = NavigationSidebar(model: model)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 340, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = sidebar
        defer { window.close() }
        var opens = 0
        sidebar.showActivity = { opens += 1 }
        sidebar.layoutSubtreeIfNeeded()
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        func entry() -> NSButton? {
            descendants(sidebar).compactMap { $0 as? NSButton }.first { $0.accessibilityIdentifier() == "activity-inbox" }
        }
        let button = try XCTUnwrap(entry())
        XCTAssertEqual(button.accessibilityLabel(), "Notifications")
        XCTAssertEqual(button.accessibilityValue() as? String, "17 unread")
        XCTAssertTrue(window.makeFirstResponder(button))
        button.performClick(nil)
        XCTAssertEqual(opens, 1)
        XCTAssertEqual(model.activityUnread, 17, "Opening the inbox is not a mark-read action")
        for unread in [0, 2] {
            model.activityUnread = unread
            let updated = expectation(for: NSPredicate { _, _ in
                MainActor.assumeIsolated { entry()?.accessibilityValue() as? String == "\(unread) unread" }
            }, evaluatedWith: sidebar)
            await fulfillment(of: [updated], timeout: 3)
            sidebar.frame.size.width = 280
            sidebar.layoutSubtreeIfNeeded()
            XCTAssertTrue(entry() === button)
            XCTAssertTrue(window.firstResponder === button, "An unread refresh must not replace the keyboard-focused entry")
            XCTAssertEqual(button.accessibilityLabel(), "Notifications")
            let visibleCounts = try XCTUnwrap(button.superview).subviews.compactMap { $0 as? NSTextField }
            XCTAssertEqual(visibleCounts.map(\.stringValue), [unread > 0 ? String(unread) : ""])
            XCTAssertTrue(visibleCounts.allSatisfy { !$0.isAccessibilityElement() })
            XCTAssertTrue(visibleCounts.allSatisfy { $0.frame.minX >= 0 && $0.frame.maxX <= button.frame.maxX })
        }
        button.performClick(nil)
        XCTAssertEqual(opens, 2)
        XCTAssertEqual(model.activityUnread, 2)
    }

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
        for width in [CGFloat(220), 370, 440] {
            sidebar.frame.size.width = 60 + width
            sidebar.layoutSubtreeIfNeeded()
            XCTAssertTrue(window.firstResponder === deafen, "Resizing must preserve the focused call control")
            XCTAssertTrue(button("voice-face-person-3") === ownFace)
            XCTAssertEqual(model.selectedChannelID, text.id)
            for control in visible(sidebar).compactMap({ $0 as? NSButton }) {
                let frame = control.convert(control.bounds, to: sidebar)
                XCTAssertGreaterThanOrEqual(frame.minX, 0, control.accessibilityIdentifier())
                XCTAssertLessThanOrEqual(frame.maxX, sidebar.bounds.width, control.accessibilityIdentifier())
                XCTAssertGreaterThan(frame.width, 0, control.accessibilityIdentifier())
            }
            let railButton = try XCTUnwrap(button("create-space"))
            XCTAssertEqual(railButton.convert(railButton.bounds, to: sidebar).midX, 30, "Resizing does not stretch the 60-point space rail")
        }
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

    func testChannelNamesIncludePrivacyAndBrowseStatusAcrossRefreshes() async throws {
        _ = NSApplication.shared
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [NavigationSidebarURLProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!, session: session))
        model.selectedSpaceID = "space"
        model.selectedChannelID = "planning"
        model.account = Account(id: "owner", username: "owner", displayName: "Owner", debugEnabled: false)
        await model.loadNotificationSettings()
        await model.refreshUnreadSpaces()
        XCTAssertTrue(model.channelUnread("planning"))
        XCTAssertTrue(model.notificationsMuted(.channel(spaceID: "space", channelID: "planning")))
        let space = Space(id: "space", name: "Example", ownerId: "owner")
        func channels(planningPrivate: Bool) -> [Channel] {
            [Channel(id: "general", spaceId: "space", name: "general", private: !planningPrivate),
             Channel(id: "planning", spaceId: "space", name: "planning", private: planningPrivate),
             Channel(id: "ideas", spaceId: "space", name: "ideas", private: !planningPrivate, joined: false),
             Channel(id: "drafts", spaceId: "space", name: "drafts", private: planningPrivate, joined: false)]
        }
        model.detail = SpaceDetail(space: space, channels: channels(planningPrivate: true), members: [])
        let sidebar = NavigationSidebar(model: model)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 344, height: 600),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = sidebar
        defer { window.close() }
        sidebar.layoutSubtreeIfNeeded()
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        func button(_ identifier: String) -> NSButton? {
            descendants(sidebar).compactMap { $0 as? NSButton }.first { $0.accessibilityIdentifier() == identifier }
        }
        func assertRows(planningPrivate: Bool, browsing: Bool) throws {
            for channel in channels(planningPrivate: planningPrivate) {
                let selected = channel.id == "planning"
                let name = channel.name + (channel.private ? ", private" : "")
                for prefix in browsing ? ["channel", "browse-channel"] : ["channel"] {
                    if prefix == "channel", !channel.joined {
                        XCTAssertNil(button("\(prefix)-\(channel.id)"), "Previews must not appear in joined navigation")
                        continue
                    }
                    let row = try XCTUnwrap(button("\(prefix)-\(channel.id)"))
                    let status = channel.joined ? "Joined" : "Read-only preview"
                    XCTAssertEqual(row.accessibilityLabel(), prefix == "browse-channel" && channel.private ? "\(name), \(status)" : name)
                    XCTAssertEqual(row.title, channel.name + (selected ? "  •" : ""), "Privacy belongs in the accessible name, not the visible title")
                    XCTAssertEqual(row.accessibilityValue() as? String, selected ? "Selected, Unread activity, Muted" : "")
                    let image = try XCTUnwrap(row.image)
                    let symbol = try XCTUnwrap(NSImage(systemSymbolName: channel.private ? "lock" : "number", accessibilityDescription: nil))
                    XCTAssertEqual(image.tiffRepresentation, symbol.tiffRepresentation)
                    if prefix == "browse-channel" {
                        XCTAssertEqual(row.accessibilityHelp(), status)
                        XCTAssertTrue(try XCTUnwrap(row.superview).subviews.compactMap { $0 as? NSTextField }.contains { $0.stringValue == status })
                    }
                }
            }
        }
        try assertRows(planningPrivate: true, browsing: false)

        // Enter Browse through the real space menu, without reaching into the
        // sidebar's private browsing flag or NavigationRow implementation.
        var openedMenu: NSMenu?
        let observer = NotificationCenter.default.addObserver(forName: NSMenu.didBeginTrackingNotification, object: nil, queue: .main) { notification in
            MainActor.assumeIsolated {
                guard let menu = notification.object as? NSMenu else { return }
                openedMenu = menu
                RunLoop.main.perform(inModes: [.eventTracking, .default]) {
                    MainActor.assumeIsolated { menu.cancelTracking() }
                }
            }
        }
        defer { NotificationCenter.default.removeObserver(observer) }
        try XCTUnwrap(button("selected-space-name")).performClick(nil)
        let menu = try XCTUnwrap(openedMenu)
        let browse = menu.indexOfItem(withTitle: "Browse channels")
        XCTAssertGreaterThanOrEqual(browse, 0)
        guard browse >= 0 else { return }
        menu.performActionForItem(at: browse)
        sidebar.layoutSubtreeIfNeeded()
        XCTAssertFalse(try XCTUnwrap(descendants(sidebar).compactMap { $0 as? NSSearchField }.first).isHidden)
        try assertRows(planningPrivate: true, browsing: true)

        // Same IDs, both directions: this must also refresh any retained rows,
        // remove stale privacy suffixes, and never accumulate duplicate suffixes.
        for planningPrivate in [false, true] {
            model.detail = SpaceDetail(space: space, channels: channels(planningPrivate: planningPrivate), members: [])
            let expected = planningPrivate ? "planning, private, Joined" : "planning"
            let refreshed = expectation(for: NSPredicate { _, _ in
                MainActor.assumeIsolated { button("browse-channel-planning")?.accessibilityLabel() == expected }
            }, evaluatedWith: sidebar)
            await fulfillment(of: [refreshed], timeout: 3)
            try assertRows(planningPrivate: planningPrivate, browsing: true)
        }
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

private final class NavigationSidebarURLProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let body: String
        switch request.url?.path {
        case "/api/notifications/settings":
            body = #"{"level":"all","mobile":"whenInactive","overrides":[{"spaceId":"space","channelId":"planning","mutedUntil":"forever"}]}"#
        case "/api/spaces/unread":
            body = #"{"spaces":[{"id":"space","channels":[{"id":"planning","lastSeq":"2","readSeq":"0"}]}]}"#
        default:
            client?.urlProtocol(self, didFailWithError: URLError(.unsupportedURL))
            return
        }
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil,
            headerFields: ["content-type": "application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
