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
        let firstDay = app.staticTexts["date-chan00000001m01"]
        XCTAssertTrue(firstDay.waitForExistence(timeout: 5))
        XCTAssertFalse((firstDay.value as? String ?? "").isEmpty, "The local date is readable through accessibility")
        XCTAssertFalse(app.staticTexts["date-chan00000001m02"].exists, "Same-day followups do not repeat the divider")
        let mentioned = app.textViews["message-chan00000001m04"]
        XCTAssertTrue(mentioned.waitForExistence(timeout: 5))
        capture("dates and mentions", app: app)
        mentioned.click()
        mentioned.typeKey("a", modifierFlags: .command)
        mentioned.typeKey("c", modifierFlags: .command)
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), mentioned.value as? String,
            "Mention styling must not add decoration or markup to copied text")
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
        let sentDay = app.staticTexts["date-\(sent.identifier.dropFirst("message-".count))"]
        XCTAssertTrue(sentDay.waitForExistence(timeout: 5), "A new local day gets a divider")
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

    func testAutocompleteKeyboardPointerUndoAndSelection() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        let message = app.textViews["message-chan00000001m04"]
        let originalFrame = message.frame
        composer.click(); composer.typeText("@")
        let feedback = app.buttons["mention-suggestion-feedback"]
        XCTAssertTrue(feedback.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["mention-suggestion-fixture_owner"].exists)
        XCTAssertTrue(app.buttons["mention-suggestion-everyone"].exists)
        XCTAssertEqual(message.frame, originalFrame, "The popup overlays rather than reflows message history")
        capture("mention suggestions", app: app)
        // Up wraps to the last row below the fold; Down wraps back to the first.
        composer.typeKey(.upArrow, modifierFlags: [])
        XCTAssertTrue(app.buttons["mention-suggestion-here"].isHittable)
        composer.typeKey(.downArrow, modifierFlags: [])
        composer.typeKey(.downArrow, modifierFlags: [])
        composer.typeKey(.return, modifierFlags: [])
        XCTAssertEqual(composer.value as? String, "@alex ", "Return accepts the selected row, not a send")
        composer.typeKey("z", modifierFlags: .command)
        XCTAssertEqual(composer.value as? String, "@", "Autocomplete is one native undoable edit")
        // AppKit selects the restored replacement on undo. Collapsing that
        // selection resumes suggestions; a selected token must not open them.
        composer.typeKey("c", modifierFlags: .command)
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), "@")
        XCTAssertFalse(feedback.exists)
        composer.typeKey(.rightArrow, modifierFlags: [])
        XCTAssertTrue(feedback.waitForExistence(timeout: 5))
        composer.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(feedback.waitForNonExistence(timeout: 5))
        composer.typeKey(.return, modifierFlags: .shift)
        XCTAssertEqual(composer.value as? String, "@\n")

        composer.typeKey("a", modifierFlags: .command)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString("👩‍💻 @ma tail", forType: .string)
        composer.typeKey("v", modifierFlags: .command)
        for _ in 0..<5 { composer.typeKey(.leftArrow, modifierFlags: []) }
        let maya = app.buttons["mention-suggestion-maya"]
        XCTAssertTrue(maya.waitForExistence(timeout: 5))
        app.buttons["send-message"].hover(); maya.hover(); maya.click()
        XCTAssertEqual(composer.value as? String, "👩‍💻 @maya  tail", "UTF-16 replacement preserves the Unicode prefix and suffix")
        composer.typeText("next")
        XCTAssertEqual(composer.value as? String, "👩‍💻 @maya next tail", "The caret remains just after the inserted separator")
        composer.typeKey("a", modifierFlags: .command); composer.typeText("@ma")
        XCTAssertTrue(maya.waitForExistence(timeout: 5))
        composer.typeKey(.leftArrow, modifierFlags: .shift)
        XCTAssertTrue(maya.waitForNonExistence(timeout: 5), "A noncollapsed selection suppresses suggestions")
        composer.typeKey(.rightArrow, modifierFlags: [])
        XCTAssertTrue(maya.waitForExistence(timeout: 5))
        composer.typeKey(.return, modifierFlags: .shift)
        XCTAssertEqual(composer.value as? String, "@ma\n", "Shift-Return adds a newline even with suggestions open")
    }

    func testAutocompleteEmojiAndDirectMessageScope() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        composer.click(); composer.typeText(":rocket")
        let rocket = app.buttons["emoji-suggestion-1f680"]
        XCTAssertTrue(rocket.waitForExistence(timeout: 5))
        capture("emoji suggestions", app: app)
        composer.typeKey(.tab, modifierFlags: [])
        XCTAssertEqual(composer.value as? String, "🚀")
        XCTAssertTrue(rocket.waitForNonExistence(timeout: 5))
        composer.typeKey("a", modifierFlags: .command); composer.typeText("@ma")
        XCTAssertTrue(app.buttons["mention-suggestion-maya"].waitForExistence(timeout: 5))
        app.buttons["profile-author-chan00000001m02"].click()
        XCTAssertTrue(app.buttons["mention-card-message"].waitForExistence(timeout: 5))
        app.buttons["mention-card-message"].click()
        XCTAssertTrue(app.buttons["mention-card-close"].waitForNonExistence(timeout: 5))
        composer.click(); composer.typeText("@")
        XCTAssertTrue(app.buttons["mention-suggestion-feedback"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["mention-suggestion-maya"].exists)
        XCTAssertFalse(app.buttons["mention-suggestion-everyone"].exists, "Channel broadcasts never leak into DMs")
        XCTAssertFalse(app.buttons["mention-suggestion-here"].exists)
        XCTAssertFalse(app.buttons["mention-suggestion-fixture_owner"].exists)
        capture("DM mention suggestions", app: app)
        composer.typeKey(.escape, modifierFlags: [])
        composer.typeKey("a", modifierFlags: .command); composer.typeText("@maya hello from autocomplete")
        composer.typeKey(.return, modifierFlags: [])
        let sent = app.textViews.matching(NSPredicate(format: "value == %@", "@maya hello from autocomplete")).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        XCTAssertEqual(composer.value as? String, "")
    }

    func testMentionClickPreservesDraggingAndProtectsUnsavedProfileEdits() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let message = app.textViews["message-chan00000001m04"]
        XCTAssertTrue(message.waitForExistence(timeout: 15))
        // The resolved pill follows "Agreed, " on the first line at either width.
        let pill = message.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 90, dy: 10))
        pill.press(forDuration: 0.05, thenDragTo: pill.withOffset(CGVector(dx: 65, dy: 0)))
        app.typeKey("c", modifierFlags: .command)
        let copied = try XCTUnwrap(NSPasteboard.general.string(forType: .string))
        XCTAssertFalse(copied.isEmpty)
        XCTAssertLessThan(copied.count, (message.value as? String ?? "").count)
        XCTAssertFalse(app.buttons["mention-card-close"].exists, "Dragging through a mention selects text, not a profile")
        pill.click()
        XCTAssertTrue(app.buttons["mention-card-edit-profile"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any)["mention-card-you"].exists)
        XCTAssertFalse(app.buttons["mention-card-message"].exists)
        capture("own mention profile", app: app)
        app.buttons["mention-card-edit-profile"].click()
        let name = app.textFields["Display name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.click(); name.typeKey("a", modifierFlags: .command); name.typeText("Unsaved name")
        app.typeKey(.escape, modifierFlags: [])
        let confirmation = app.sheets.containing(.staticText, identifier: "Discard profile changes?").firstMatch
        let keep = confirmation.buttons["Keep editing"]
        XCTAssertTrue(keep.waitForExistence(timeout: 5), "Escape must not silently discard profile edits")
        capture("unsaved profile confirmation", app: app)
        keep.click()
        XCTAssertEqual(name.value as? String, "Unsaved name")
        app.buttons["Close profile"].click()
        XCTAssertTrue(confirmation.buttons["Discard"].waitForExistence(timeout: 5))
        confirmation.buttons["Discard"].click()
        XCTAssertTrue(app.buttons["profile-save"].waitForNonExistence(timeout: 5))
        app.buttons["profile-avatar-chan00000001m01"].click()
        XCTAssertTrue(app.buttons["mention-card-edit-profile"].waitForExistence(timeout: 5))
        app.buttons["mention-card-close"].click()
    }

    func testAuthorAndMemberProfilesShareDraftAndSendThroughDirectMessages() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let author = app.buttons["profile-author-chan00000001m02"]
        XCTAssertTrue(author.waitForExistence(timeout: 15))
        author.hover(); app.buttons["profile-author-chan00000001m04"].hover(); author.click()
        let composer = app.textViews["mention-card-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        composer.click(); composer.typeText("TEST FIXTURE — independent profile draft")
        capture("profile inline composer", app: app)
        app.buttons["mention-card-close"].click()
        app.buttons["profile-member-member000001"].click()
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — independent profile draft")
        app.descendants(matching: .any)["mention-card-more"].click()
        app.descendants(matching: .any)["mention-card-block"].click()
        let confirmation = app.sheets.containing(.staticText, identifier: "Block Maya?").firstMatch
        XCTAssertTrue(confirmation.buttons["Block"].waitForExistence(timeout: 5))
        capture("profile block confirmation", app: app)
        confirmation.buttons["Cancel"].click()
        author.click()
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — independent profile draft")
        app.buttons["mention-card-send"].click()
        let sent = app.textViews.matching(NSPredicate(format: "value == %@", "TEST FIXTURE — independent profile draft")).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10), "Profile Send opens the confirmed DM without losing the draft")
        XCTAssertTrue(app.buttons["mention-card-close"].waitForNonExistence(timeout: 5))
        XCTAssertEqual(app.textViews["message-composer"].value as? String, "")
        capture("profile message delivered", app: app)
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
