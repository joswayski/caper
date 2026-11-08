import Foundation
import XCTest

@MainActor final class AXMenuTests: XCTestCase {
    func testInitialTextFieldAndFullHierarchy() throws {
        continueAfterFailure = false
        let pidFile = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: pidFile) }
        let app = XCUIApplication(bundleIdentifier: "chat.caper.diagnostic.ax-menu")
        app.launchEnvironment["AX_MENU_PID_PATH"] = pidFile.path
        app.launch()
        let pid = try String(contentsOf: pidFile, encoding: .utf8)
        let start = ProcessInfo.processInfo.systemUptime
        FileHandle.standardOutput.write(Data("AX_CONTROL_QUERY_BEGIN pid=\(pid)\n".utf8))
        XCTAssertTrue(app.textFields["standalone-text-field"].waitForExistence(timeout: 15))
        let hierarchy = app.debugDescription
        let attachment = XCTAttachment(string: hierarchy)
        attachment.name = "Standalone full AX hierarchy"
        attachment.lifetime = .keepAlways
        add(attachment)
        XCTAssertTrue(hierarchy.contains("standalone-text-field"))
        let elapsed = ProcessInfo.processInfo.systemUptime - start
        FileHandle.standardOutput.write(Data("AX_CONTROL_QUERY_END elapsed=\(elapsed)\n".utf8))
    }
}
