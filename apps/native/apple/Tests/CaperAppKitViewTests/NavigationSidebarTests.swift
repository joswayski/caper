import AppKit
import XCTest
@testable import CaperCore

@MainActor final class NavigationSidebarTests: XCTestCase {
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
