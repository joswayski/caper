import AppKit
import XCTest

@MainActor final class CaperAppKitTests: XCTestCase {
    private func launch(signedIn: Bool = true) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["CAPER_TEST_MODE"] = "parity"
        app.launchEnvironment["CAPER_API_BASE_URL"] = "http://127.0.0.1:3001"
        if signedIn {
            app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-owner-token"
            app.launchEnvironment["CAPER_TEST_SPACE_ID"] = "space0000001"
        }
        app.launchArguments = ["-ApplePersistenceIgnoreState", "YES", "-caper.space-landing.owner0000001", ""]
        app.launch()
        return app
    }

    private func capture(_ name: String, app: XCUIApplication) {
        let screenshot = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        screenshot.name = "AppKit \(name) — local fixture"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    private nonisolated static func control(_ body: [String: Any]) async throws {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (_, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
    }

    private nonisolated static func resetFixture() async throws { try await control(["reset": true]) }

    func testScrollUpHistoryRetriesFailureAndPreservesTheVisibleMessage() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        for index in 1...55 {
            try await Self.control(["incomingMessage": ["channelId": "chan00000001", "text": "History row \(index) 🔥 🚀 🎉"]])
        }
        let app = launch()
        defer { app.terminate() }
        let latest = app.textViews["message-fixture00000059"]
        XCTAssertTrue(latest.waitForExistence(timeout: 15))
        XCTAssertFalse(app.buttons["Load older messages"].exists)
        try await Self.control(["failure": ["path": "/api/chat/channels/chan00000001/messages", "method": "GET",
            "status": 503, "error": "TEST FIXTURE: older history unavailable."]])
        latest.hover()
        latest.scroll(byDeltaX: 0, deltaY: 2400)
        let retry = app.buttons["conversation-retry"]
        XCTAssertTrue(retry.waitForExistence(timeout: 10))
        let anchor = app.textViews["message-fixture00000010"]
        XCTAssertTrue(anchor.exists)
        let before = anchor.frame.minY
        capture("history error", app: app)
        retry.hover()
        app.buttons["send-message"].hover()
        retry.click()
        XCTAssertTrue(retry.waitForNonExistence(timeout: 10))
        XCTAssertTrue(anchor.exists)
        XCTAssertEqual(anchor.frame.minY, before, accuracy: 1, "Prepending a page keeps the same text at the same screen position")
        anchor.hover()
        anchor.scroll(byDeltaX: 0, deltaY: 900)
        XCTAssertTrue(app.textViews["message-chan00000001m01"].waitForExistence(timeout: 10))
        capture("older history loaded", app: app)
    }

    func testConversationSendNavigationAndSelection() async throws {
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        XCTAssertEqual(app.staticTexts["member-count"].value as? String, "3")
        app.buttons["channel-chan00000002"].hover()
        app.buttons["channel-chan00000003"].hover()
        composer.click()
        let message = "TEST FIXTURE — AppKit sends through the API and gateway."
        composer.typeText(message)
        XCTAssertEqual(composer.value as? String, message)
        // Replace a selected suffix rather than merely asserting the value.
        composer.typeKey(.leftArrow, modifierFlags: .shift)
        composer.typeText("!")
        let edited = String(message.dropLast()) + "!"
        XCTAssertEqual(composer.value as? String, edited)
        app.buttons["send-message"].click()
        let sent = app.textViews.matching(NSPredicate(format: "value == %@", edited)).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10), "The API/gateway confirms the native composer send")
        XCTAssertEqual(composer.value as? String, "")
        NSPasteboard.general.clearContents()
        sent.click()
        sent.typeKey("a", modifierFlags: .command)
        sent.typeKey("c", modifierFlags: .command)
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), edited, "Message text is selectable and copyable")
        app.buttons["channel-chan00000002"].click()
        XCTAssertTrue(app.staticTexts["# design"].waitForExistence(timeout: 10))
        app.buttons["channel-chan00000001"].click()
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        capture("conversation, wide", app: app)
    }

    func testMemberSearchPrivateGroupsAndReopeningThePanel() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let list = app.scrollViews["member-list"]
        XCTAssertTrue(list.waitForExistence(timeout: 15))
        let maya = app.groups["member-member000001"]
        XCTAssertTrue(maya.waitForExistence(timeout: 10))
        XCTAssertEqual(maya.label, "Maya, Online")
        capture("members", app: app)
        app.buttons["Search members"].hover()
        app.buttons["Close member list"].hover()
        app.buttons["Search members"].click()
        let search = app.searchFields["Name or username"]
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.typeText("FIXTURE_OWNER")
        XCTAssertTrue(maya.waitForNonExistence(timeout: 5))
        XCTAssertTrue(app.groups["member-owner0000001"].exists, "Search includes usernames, not only display names")
        capture("member search", app: app)
        search.typeKey("a", modifierFlags: .command); search.typeText("no such member")
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "value == %@", "No matching members.")).firstMatch.waitForExistence(timeout: 5))
        capture("no matching members", app: app)
        search.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        XCTAssertTrue(maya.waitForExistence(timeout: 5))
        app.buttons["channel-chan00000003"].click()
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "value == %@", "In #planning  2")).firstMatch.waitForExistence(timeout: 10))
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "value == %@", "Others in this space  1")).firstMatch.exists)
        capture("private channel members", app: app)
        app.buttons["Close member list"].click()
        XCTAssertTrue(list.waitForNonExistence(timeout: 5))
        app.buttons["toggle-members"].hover()
        app.buttons["toggle-members"].click()
        XCTAssertTrue(list.waitForExistence(timeout: 5))
        XCTAssertTrue(maya.exists)
        app.buttons["Search members"].click()
        search.typeText("maya")
        app.buttons["Close member list"].click()
        app.buttons["toggle-members"].click()
        XCTAssertTrue(app.groups["member-owner0000001"].waitForExistence(timeout: 5), "Closing the panel clears its filter")
    }

    func testSignInErrorAndSixCharacterCodeRecovery() async throws {
        try await Self.resetFixture()
        let app = launch(signedIn: false)
        defer { app.terminate() }
        let email = app.textFields["Email address"]
        XCTAssertTrue(email.waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Send code"].isEnabled)
        email.click(); email.typeText("not-an-email")
        app.buttons["Send code"].click()
        XCTAssertTrue(app.buttons["Send code"].waitForExistence(timeout: 10))
        XCTAssertTrue(email.isEnabled)
        XCTAssertEqual(app.staticTexts["sign-in-error"].value as? String, "Enter a valid email address.")
        capture("sign-in error", app: app)
        email.click(); email.typeKey("a", modifierFlags: .command)
        email.typeText("owner@example.test\n")
        let code = app.textFields["Sign-in code"]
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Sign in"].isEnabled)
        code.click(); code.typeText("ZZZZZ9")
        for attempt in 0..<3 {
            app.buttons["Sign in"].click()
            let result = attempt == 2 ? app.buttons["Email me a new code"] : app.buttons["Sign in"]
            XCTAssertTrue(result.waitForExistence(timeout: 5))
            if attempt < 2 { XCTAssertTrue(result.isEnabled) }
        }
        XCTAssertFalse(code.isEnabled)
        capture("exhausted sign-in code", app: app)
        app.buttons["Email me a new code"].click()
        XCTAssertTrue(app.buttons["Sign in"].waitForExistence(timeout: 5))
        XCTAssertTrue(code.isEnabled)
        code.click(); code.typeText("abc234")
        XCTAssertEqual(code.value as? String, "ABC234")
        app.buttons["Sign in"].click()
        XCTAssertTrue(app.textViews["message-composer"].waitForExistence(timeout: 15))
        app.buttons["Sign out"].click()
        XCTAssertTrue(email.waitForExistence(timeout: 10))
    }
}
