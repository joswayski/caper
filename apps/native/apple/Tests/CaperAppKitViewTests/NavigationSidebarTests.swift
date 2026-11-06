import AppKit
import XCTest
@testable import CaperCore

@MainActor final class NavigationSidebarTests: XCTestCase {
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
