import XCTest

@MainActor
extension XCTestCase {
    /// Fixture controls: reset, live messages from Alex.
    func control(_ body: [String: Any]) {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: body)
        let done = expectation(description: "fixture control")
        URLSession.shared.dataTask(with: request) { _, response, _ in
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
            done.fulfill()
        }.resume()
        wait(for: [done], timeout: 10)
    }
}

/// The development UIKit client against the local parity fixture in bench mode
/// (`CAPER_BENCH_MESSAGES=3000`: 3,000 #general messages in pages of 50).
/// `uikit-screenshots.sh` starts the fixture; it never reaches a real server.
@MainActor
final class CaperUIKitTests: XCTestCase {
    private static let latest = "00000000-0000-4000-9000-000000003000"
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        control(["reset": true])
    }

    private func launch(signedIn: Bool = true, cache: Bool = false, openURL: String? = nil, fixture: String? = nil) -> XCUIApplication {
        let app = XCUIApplication()
        if let openURL { app.launchEnvironment["CAPER_TEST_OPEN_URL"] = openURL }
        if let fixture { app.launchEnvironment["CAPER_UI_FIXTURE"] = fixture }
        app.launchArguments += ["-caper.space-landing.owner0000001", ""]
        // Browse's Channels/DMs split starts even in every run.
        app.launchArguments += ["-caper.channelSidebarChannelsShare", "0.5"]
        app.launchEnvironment["CAPER_TEST_MODE"] = "parity"
        app.launchEnvironment["CAPER_API_BASE_URL"] = "http://127.0.0.1:3001"
        if signedIn {
            app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-owner-token"
            app.launchEnvironment["CAPER_TEST_SPACE_ID"] = "space0000001"
        }
        if cache { app.launchEnvironment["CAPER_UIKIT_STARTUP_CACHE"] = "1" }
        app.launch()
        self.app = app
        return app
    }

    private func screenshot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func row(_ id: String) -> XCUIElement { app.cells["message-row-\(id)"] }

    func testSignInWithEmailCode() {
        let app = launch(signedIn: false)
        let email = app.textFields["login-email"]
        XCTAssertTrue(email.waitForExistence(timeout: 20))
        XCTAssertEqual(email.label, "Email address")
        let primary = app.buttons["login-primary"]
        XCTAssertFalse(primary.isEnabled)
        screenshot("uikit-login")
        email.tap()
        email.typeText("owner@example.com")
        XCTAssertTrue(primary.isEnabled)
        primary.tap()
        let code = app.textFields["login-code"]
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Check your email."].exists)
        // The unambiguous alphabet, uppercased and capped at six.
        code.typeText("abc2340")
        XCTAssertEqual(code.value as? String, "ABC234")
        screenshot("uikit-login-code")
        app.buttons["login-primary"].tap()
        XCTAssertTrue(app.textViews["message-composer"].waitForExistence(timeout: 20))
    }

    func testWrongCodeShowsError() {
        let app = launch(signedIn: false)
        let email = app.textFields["login-email"]
        XCTAssertTrue(email.waitForExistence(timeout: 20))
        email.tap()
        email.typeText("owner@example.com\n")
        let code = app.textFields["login-code"]
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        code.typeText("ZZZ999")
        app.buttons["login-primary"].tap()
        let error = app.otherElements["login-error"]
        XCTAssertTrue(error.waitForExistence(timeout: 10))
        XCTAssertTrue(error.label.contains("ncorrect"), error.label)
        screenshot("uikit-login-error")
    }

    /// A new account's profile is SwiftUI's page: Finish setup waits for a
    /// valid username, the server's refusal shows, and Log out leaves.
    func testOnboardingValidatesAndLogsOut() {
        let app = launch(fixture: "profile-validation")
        let username = app.textFields["Username"]
        let submit = app.buttons["profile-continue"]
        XCTAssertTrue(username.waitForExistence(timeout: 20))
        XCTAssertTrue(app.staticTexts["Choose how you show up."].exists)
        XCTAssertTrue(app.staticTexts["TEST FIXTURE — username already taken. Choose another username."].exists)
        XCTAssertFalse(submit.isEnabled)
        username.tap()
        username.typeText("ab\n")
        app.textFields["Display name"].typeText("Fixture Name")
        XCTAssertFalse(submit.isEnabled, "Two-letter usernames can't finish")
        screenshot("uikit-onboarding")
        username.tap()
        username.typeText("_user")
        XCTAssertTrue(submit.isEnabled)
        // Below the fields, possibly under the keyboard: scroll it clear first.
        let logOut = app.buttons["Log out"]
        if !logOut.isHittable { app.staticTexts["Choose how you show up."].swipeUp() }
        logOut.tap()
        XCTAssertTrue(app.textFields["login-email"].waitForExistence(timeout: 10), "Log out returns to sign-in")
    }

    /// Finish setup saves the profile and opens the workspace at the space's channel.
    func testFinishSetupOpensWorkspace() {
        let app = launch(fixture: "profile-validation")
        let username = app.textFields["Username"]
        XCTAssertTrue(username.waitForExistence(timeout: 20))
        username.tap()
        username.typeText("fixture_new\n")
        app.textFields["Display name"].typeText("Fixture New")
        let submit = app.buttons["profile-continue"]
        XCTAssertTrue(submit.isEnabled)
        if !submit.isHittable { app.staticTexts["Choose how you show up."].swipeUp() }
        screenshot("uikit-finish-setup")
        submit.tap()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30), "The workspace opens at the latest message")
        XCTAssertFalse(app.buttons["profile-continue"].exists)
        app.buttons["back-to-browse"].firstMatch.tap()
        let profile = app.buttons["account-profile"]
        XCTAssertTrue(profile.waitForExistence(timeout: 10))
        XCTAssertEqual(profile.label, "Edit profile for Fixture New", "The account bar shows the saved profile")
        screenshot("uikit-finish-setup-done")
    }

    func testConversationOpensAtLatestAndSends() {
        let app = launch()
        let latest = row(Self.latest)
        XCTAssertTrue(latest.waitForExistence(timeout: 30))
        XCTAssertTrue(latest.isHittable, "Opens at the newest message")
        XCTAssertEqual(app.buttons["channel-title-menu"].label, "#general options")
        screenshot("uikit-conversation")

        let composer = app.textViews["message-composer"]
        XCTAssertEqual(composer.label, "Message #general")
        XCTAssertFalse(app.buttons["send-message"].isEnabled)
        composer.tap()
        composer.typeText("TEST FIXTURE — sent from UIKit")
        XCTAssertTrue(app.buttons["send-message"].isEnabled)
        app.buttons["send-message"].tap()
        let sent = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "sent from UIKit")).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        XCTAssertTrue(sent.isHittable, "The timeline follows your own message")
        XCTAssertEqual((composer.value as? String) ?? "", "")
    }

    /// Formatted text typed through the keyboard stays exactly as
    /// typed: closing a style and typing on must not carry it onto later words.
    func testTypingFormattedTextKeepsItExactly() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        composer.tap()
        let draft = "**Release check** for _the voice beta_: see **both _builds_** and **Done** when"
        composer.typeText(draft)
        XCTAssertEqual(composer.value as? String, draft, "A closed style must not spread onto the words typed after it")
    }

    /// SwiftUI's pending row: an unknown outcome offers Retry send; a rejected
    /// send offers Edit (back to the composer) and Dismiss.
    func testFailedSendsRetryEditAndDismiss() {
        control(["admin": true])
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        let send = app.buttons["send-message"]
        let path = "/api/chat/channels/chan00000001/messages"
        control(["failure": ["path": path, "method": "POST", "status": 503]])
        composer.tap()
        composer.typeText("TEST FIXTURE unsure")
        send.tap()
        let retry = app.buttons["Retry send"]
        XCTAssertTrue(retry.waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Dismiss"].exists)
        let pending = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "pending-message-")).firstMatch
        XCTAssertTrue(pending.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "Admin")).firstMatch.exists,
                      "The unsent row shows its author's Admin badge, as SwiftUI's does")
        screenshot("uikit-pending-unknown")
        retry.tap()
        let confirmed = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "TEST FIXTURE unsure")).firstMatch
        XCTAssertTrue(retry.waitForNonExistence(timeout: 10))
        XCTAssertTrue(confirmed.exists)

        control(["failure": ["path": path, "method": "POST", "status": 422, "error": "TEST FIXTURE: refused."]])
        composer.typeText("TEST FIXTURE refused")
        send.tap()
        let edit = app.buttons["Edit"]
        XCTAssertTrue(edit.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Not sent. TEST FIXTURE: refused."].exists)
        XCTAssertFalse(retry.exists)
        screenshot("uikit-pending-rejected")
        edit.tap()
        XCTAssertTrue(edit.waitForNonExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE refused", "Edit returns the text to the composer")

        control(["failure": ["path": path, "method": "POST", "status": 422, "error": "TEST FIXTURE: refused."]])
        send.tap()
        let dismiss = app.buttons["Dismiss"]
        XCTAssertTrue(dismiss.waitForExistence(timeout: 10))
        dismiss.tap()
        XCTAssertTrue(dismiss.waitForNonExistence(timeout: 5))
        XCTAssertEqual((composer.value as? String) ?? "", "")
    }

    /// With read state, the first unread message (seq 2998) gets SwiftUI's
    /// terracotta "New messages" line, which starts a new author group.
    func testNewMessagesDivider() {
        control(["channelReads": ["chan00000001": "2997"]])
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let divider = app.cells["channel-new-messages"]
        XCTAssertTrue(divider.waitForExistence(timeout: 10))
        XCTAssertEqual(divider.label, "New messages")
        let first = row("00000000-0000-4000-9000-000000002998")
        XCTAssertTrue(first.exists)
        XCTAssertEqual(divider.frame.maxY, first.frame.minY, accuracy: 1, "The line sits directly above the first unread message")
        screenshot("uikit-new-messages")
    }

    /// The rail is SwiftUI's: an unread space gets its dot and says so.
    func testRailShowsUnreadSpace() {
        control(["channelReads": ["chan00000002": "0"]])
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.buttons["back-to-browse"].tap()
        let tile = app.buttons["space-space0000001"]
        XCTAssertTrue(tile.waitForExistence(timeout: 10))
        let unread = NSPredicate(format: "value CONTAINS %@", "Unread messages")
        XCTAssertEqual(XCTWaiter.wait(for: [expectation(for: unread, evaluatedWith: tile)], timeout: 15), .completed)
        XCTAssertTrue((tile.value as? String)?.hasPrefix("Selected") == true)
        screenshot("uikit-rail-unread")
    }

    /// A reply's context jumps to the old message; the bar then offers Load
    /// newer messages and Back to latest, which returns to the live end.
    func testJumpThenBackToLatest() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        control(["incomingMessage": ["channelId": "chan00000001", "text": "replying way back", "replyToId": "b00000000002000"]])
        let context = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "View original message from")).firstMatch
        XCTAssertTrue(context.waitForExistence(timeout: 10))
        context.tap()
        let target = row("00000000-0000-4000-9000-000000002000")
        XCTAssertTrue(target.waitForExistence(timeout: 15), "Jumps to the replied-to message")
        let latest = app.buttons["back-to-latest"]
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["Load newer messages"].exists)
        screenshot("uikit-back-to-latest")
        latest.tap()
        XCTAssertTrue(latest.waitForNonExistence(timeout: 15))
        let reply = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "replying way back")).firstMatch
        XCTAssertTrue(reply.waitForExistence(timeout: 10))
        XCTAssertTrue(reply.isHittable, "Back at the live end")
    }

    func testLiveMessageFollowsTailAndHistoryLoadsOnScrollUp() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        control(["incomingMessage": ["channelId": "chan00000001", "text": "live arrival"]])
        let live = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "live arrival")).firstMatch
        XCTAssertTrue(live.waitForExistence(timeout: 10))
        XCTAssertTrue(live.isHittable, "A live message at the tail scrolls into view")

        // The first page holds 2951–3000; scrolling up fetches 2901–2950.
        let timeline = app.collectionViews["message-timeline"]
        // Any row before the first page proves a fetch; fast swipes can fly past a specific one.
        let older = app.cells.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND identifier < %@",
                                                   "message-row-", "message-row-00000000-0000-4000-9000-000000002951")).firstMatch
        for _ in 0..<40 where !older.exists {
            timeline.swipeDown(velocity: .fast)
        }
        XCTAssertTrue(older.waitForExistence(timeout: 10), "Older history loads while scrolling up")
        screenshot("uikit-history")
    }

    func testBrowseChannelsAndSwitch() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let back = app.buttons["back-to-browse"]
        XCTAssertEqual(back.label, "Back to Browse")
        back.tap()
        let design = app.buttons["channel-chan00000002"]
        XCTAssertTrue(design.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["selected-space-name"].exists)
        XCTAssertEqual(app.buttons["channel-chan00000001"].value as? String, "Selected")
        // The lock is hidden from VoiceOver, so a private channel's name says it (SwiftUI's row, hosted).
        XCTAssertEqual(app.buttons["channel-chan00000003"].label, "planning, private")
        XCTAssertEqual(design.label, "design")
        screenshot("uikit-browse")
        design.tap()
        let title = app.buttons["channel-title-menu"]
        XCTAssertTrue(title.waitForExistence(timeout: 10))
        let switched = NSPredicate(format: "label == %@", "#design options")
        expectation(for: switched, evaluatedWith: title)
        waitForExpectations(timeout: 10)
        XCTAssertEqual(app.textViews["message-composer"].label, "Message #design")
    }

    /// Browse as SwiftUI draws it: the Channels toggle keeps only live rooms
    /// when collapsed, the divider adjusts, X returns to the conversation, and
    /// the account bar's identity opens Profile.
    func testBrowseHeaderToggleAndAccountBar() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.buttons["back-to-browse"].tap()
        let toggle = app.buttons["channels-toggle"]
        XCTAssertTrue(toggle.waitForExistence(timeout: 10))
        XCTAssertEqual(toggle.value as? String, "Expanded")
        let planning = app.buttons["channel-chan00000003"]
        XCTAssertTrue(planning.waitForExistence(timeout: 5))
        toggle.tap()
        XCTAssertTrue(planning.waitForNonExistence(timeout: 5), "Collapsed hides rooms nobody is in")
        XCTAssertTrue(app.buttons["channel-chan00000002"].exists, "The live room stays")
        XCTAssertEqual(toggle.value as? String, "Collapsed")
        screenshot("uikit-browse-collapsed")
        toggle.tap()
        XCTAssertTrue(planning.waitForExistence(timeout: 5))

        let divider = app.descendants(matching: .any)["sidebar-sections-resize"].firstMatch
        XCTAssertEqual(divider.value as? String, "50 percent channels")
        let start = divider.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.2, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -120)))
        XCTAssertNotEqual(divider.value as? String, "50 percent channels", "Dragging resizes the lists")
        divider.doubleTap()
        XCTAssertEqual(divider.value as? String, "50 percent channels", "Double-tap evens them out")

        app.buttons["account-profile"].tap()
        XCTAssertTrue(app.staticTexts["Edit profile"].firstMatch.waitForExistence(timeout: 10))
        app.buttons["Close profile"].firstMatch.tap()
        app.buttons["Close navigation"].tap()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 10))
        XCTAssertTrue(row(Self.latest).isHittable, "X returns to the conversation")
    }

    /// The title is the conversation's menu: Pins and Members.
    func testTitleMenuOpensPinsAndMembers() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let menu = app.buttons["channel-title-menu"]
        XCTAssertEqual(menu.value as? String, "48 members")
        menu.tap()
        let membersItem = app.buttons["Members · 48"]
        XCTAssertTrue(membersItem.waitForExistence(timeout: 5))
        screenshot("uikit-title-menu")
        app.buttons["Pins"].tap()
        XCTAssertTrue(app.staticTexts["No pinned messages"].waitForExistence(timeout: 5))
        screenshot("uikit-pins-empty")
        app.buttons["close-pins"].tap()
        XCTAssertTrue(menu.waitForExistence(timeout: 5))

        menu.tap()
        XCTAssertTrue(membersItem.waitForExistence(timeout: 5))
        membersItem.tap()
        let list = app.collectionViews["member-list"]
        let shown = list.waitForExistence(timeout: 5)
        sleep(1)
        screenshot("uikit-members")
        XCTAssertTrue(shown)
        let owner = list.descendants(matching: .any)["member-fixture_owner"]
        XCTAssertTrue(owner.waitForExistence(timeout: 10))
        XCTAssertEqual(owner.value as? String, "online")
        app.buttons["Close member list"].tap()
        XCTAssertFalse(list.exists)
    }

    /// Emojis, GIFs and stickers insert at the caret; the microphone is there.
    func testComposerEmojiPicker() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        composer.tap()
        composer.typeText("hi ")
        let emoji = app.buttons["composer-emoji"]
        XCTAssertEqual(emoji.label, "Emojis, GIFs and stickers")
        emoji.tap()
        let grid = app.descendants(matching: .any)["reaction-picker-grid"]
        XCTAssertTrue(grid.waitForExistence(timeout: 10))
        screenshot("uikit-emoji-picker")
        grid.buttons.firstMatch.tap()
        XCTAssertTrue(grid.waitForNonExistence(timeout: 5))
        let value = composer.value as? String ?? ""
        XCTAssertTrue(value.hasPrefix("hi ") && value.count > 3, value)
        XCTAssertTrue(app.descendants(matching: .any)["composer-voice-record"].exists)
        XCTAssertTrue(app.buttons["send-message"].isEnabled)
    }

    /// A photo chosen in the attach drawer is staged above the composer.
    /// (The fixture's relative upload URLs suit web only, so CaperCore never
    /// uploads to it; `testIncomingAttachmentRenders` covers sent media.)
    func testAttachDrawerStagesAPhoto() {
        control(["reset": true, "uploadsEnabled": true])
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let attach = app.buttons["attach-files-button"]
        expectation(for: NSPredicate(format: "isEnabled == true"), evaluatedWith: attach)
        waitForExpectations(timeout: 15)
        attach.tap()
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'Full Access' OR label BEGINSWITH[c] 'Allow'")).firstMatch
        let tiles = app.buttons.matching(identifier: "attach-drawer-tile")
        // A hosted simulator can take 10 s to show the prompt: wait for it or the tiles.
        let asked = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in allow.exists || tiles.firstMatch.exists }, object: nil)
        _ = XCTWaiter.wait(for: [asked], timeout: 20)
        if allow.exists { allow.tap() }
        XCTAssertTrue(tiles.firstMatch.waitForExistence(timeout: 15), "The simulator's sample photos must appear")
        tiles.firstMatch.tap()
        XCTAssertTrue(app.buttons["attach-drawer-send"].waitForExistence(timeout: 10))
        screenshot("uikit-attach-drawer")
        // A real drag: XCUITest's swipeDown travels too little for the close threshold.
        let drawer = app.otherElements["attach-drawer"]
        let handle = app.buttons["attach-drawer-resize"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        handle.press(forDuration: 0.05, thenDragTo: handle.withOffset(CGVector(dx: 0, dy: 300)), withVelocity: .fast, thenHoldForDuration: 0)
        _ = drawer.waitForNonExistence(timeout: 3)
        XCTAssertFalse(drawer.exists, "Swiping the handle down closes the drawer")
        XCTAssertTrue(app.descendants(matching: .any)["Files to send"].waitForExistence(timeout: 10), "The photo waits above the composer")
        XCTAssertTrue(app.buttons["send-message"].isEnabled)
        screenshot("uikit-attachment-staged")
    }

    /// A message's image arrives by edit and renders inside the row.
    func testIncomingAttachmentRenders() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        control(["incomingEdit": ["channelId": "chan00000001", "messageId": "b00000000003000", "text": "Image test",
                                  "attachments": [["id": "fixtureimage001", "kind": "image", "status": "ready",
                                                   "name": "TEST FIXTURE image.png", "contentType": "image/png",
                                                   "size": 1024, "width": 512, "height": 512,
                                                   "url": "http://127.0.0.1:3001/__fixture/embed.svg"]]]])
        let media = row(Self.latest).descendants(matching: .any)["attachment-fixtureimage001"]
        XCTAssertTrue(media.waitForExistence(timeout: 15))
        screenshot("uikit-attachment-row")
    }

    /// Reaction chips toggle yours; holding a row opens message actions,
    /// whose Pin shows up in Pins with Go to message.
    func testReactionsAndMessageActionsPin() {
        let app = launch()
        let latest = row(Self.latest)
        XCTAssertTrue(latest.waitForExistence(timeout: 30))

        // The latest message gets a 👍 from the actions sheet's quick reactions.
        app.textViews["message-text-\(Self.latest)"].press(forDuration: 0.8)
        let sheet = app.descendants(matching: .any)["message-actions-sheet"]
        XCTAssertTrue(sheet.waitForExistence(timeout: 10), "Holding a row opens its actions")
        screenshot("uikit-message-actions")
        app.buttons["quick-reaction-👍"].tap()
        XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
        let chip = app.buttons["reaction-chip-b00000000003000-👍"]
        XCTAssertTrue(chip.waitForExistence(timeout: 10), "The reaction appears as a chip")
        XCTAssertTrue(chip.label.contains("selected by you"), chip.label)
        screenshot("uikit-reaction-chip")
        // Tapping your own chip takes it back.
        chip.tap()
        XCTAssertTrue(chip.waitForNonExistence(timeout: 10))

        app.textViews["message-text-\(Self.latest)"].press(forDuration: 0.8)
        XCTAssertTrue(sheet.waitForExistence(timeout: 10))
        app.buttons["Pin message"].tap()
        XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
        let badge = latest.buttons["Pinned by you"]
        XCTAssertTrue(badge.waitForExistence(timeout: 10), "Pinned rows say who pinned them")
        screenshot("uikit-pinned-row")
        // "Pinned by" opens the pinner's card.
        badge.tap()
        let profile = app.descendants(matching: .any)["mention-card"]
        XCTAssertTrue(profile.waitForExistence(timeout: 10), "Pinned by opens the pinner's profile")
        app.swipeDown(velocity: .fast)
        XCTAssertTrue(profile.waitForNonExistence(timeout: 10))

        app.buttons["channel-title-menu"].tap()
        app.buttons["Pins"].tap()
        let card = app.descendants(matching: .any)["pinned-message-b00000000003000"]
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        screenshot("uikit-pins")
        // Holding a card opens its actions over Pins.
        card.textViews.firstMatch.press(forDuration: 0.8)
        XCTAssertTrue(sheet.waitForExistence(timeout: 10), "Holding a pinned card opens message actions")
        app.swipeDown(velocity: .fast)
        XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
        XCTAssertTrue(card.exists, "Pins stays open under the actions")
        // Tapping the card's body goes to the message, as SwiftUI's.
        card.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1)).tap()
        XCTAssertTrue(card.waitForNonExistence(timeout: 10), "Tapping a card goes to its message")
        XCTAssertTrue(latest.waitForExistence(timeout: 10))

        app.buttons["channel-title-menu"].tap()
        app.buttons["Pins"].tap()
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        app.buttons["go-to-message-b00000000003000"].tap()
        XCTAssertTrue(card.waitForNonExistence(timeout: 10), "Go to message closes Pins")
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
    }

    /// "(edited)" opens the message's history; swipes open Members and
    /// Browse, and return from Browse, as SwiftUI's drawers.
    func testEditedHistoryAndSwipes() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        composer.tap()
        composer.typeText("TEST FIXTURE edit me")
        app.buttons["send-message"].tap()
        let sent = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "edit me")).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        sent.press(forDuration: 0.8)
        XCTAssertTrue(app.descendants(matching: .any)["message-actions-sheet"].waitForExistence(timeout: 10))
        app.buttons["Edit message"].tap()
        let field = app.descendants(matching: .any)["message-edit-text"]
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        field.tap()
        field.typeText(" again")
        app.buttons["Save changes"].tap()
        let edited = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "message-edited-")).firstMatch
        XCTAssertTrue(edited.waitForExistence(timeout: 10), "An edited row shows (edited) as a button")
        XCTAssertEqual(edited.label, "Edited, version 2")
        edited.tap()
        XCTAssertTrue(app.staticTexts["Message history"].waitForExistence(timeout: 10), "(edited) opens the history")
        screenshot("uikit-message-history")
        app.buttons["Close"].tap()
        XCTAssertTrue(app.staticTexts["Message history"].waitForNonExistence(timeout: 10))

        let timeline = app.collectionViews["message-timeline"]
        timeline.swipeLeft()
        let members = app.descendants(matching: .any)["members-panel"]
        XCTAssertTrue(members.waitForExistence(timeout: 10), "A swipe left on the timeline opens Members")
        app.buttons["Close member list"].tap()
        XCTAssertTrue(members.waitForNonExistence(timeout: 10))

        timeline.swipeRight()
        let account = app.buttons["account-profile"]
        XCTAssertTrue(account.waitForExistence(timeout: 10))
        sleep(1)
        XCTAssertTrue(account.isHittable, "A swipe right from mid-timeline opens Browse")
        app.collectionViews["direct-message-list"].swipeLeft()
        sleep(1)
        XCTAssertTrue(composer.isHittable, "A swipe left on Browse returns to the conversation")
    }

    /// Reply in thread opens the thread; a reply there shows under the root
    /// as "1 reply · View thread", which reopens it.
    func testReplyInThreadAndViewThread() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.textViews["message-text-\(Self.latest)"].press(forDuration: 0.8)
        let sheet = app.descendants(matching: .any)["message-actions-sheet"]
        XCTAssertTrue(sheet.waitForExistence(timeout: 10))
        app.buttons["Reply in thread"].tap()
        let thread = app.descendants(matching: .any)["thread-view"]
        XCTAssertTrue(thread.waitForExistence(timeout: 10), "Reply in thread opens the thread")
        XCTAssertTrue(app.staticTexts["in #general"].exists)
        let composer = app.textViews["thread-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.label, "Reply to thread…")
        composer.tap()
        composer.typeText("TEST FIXTURE — a thread reply")
        app.buttons["send-message"].tap()
        let reply = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "a thread reply")).firstMatch
        XCTAssertTrue(reply.waitForExistence(timeout: 10))
        screenshot("uikit-thread")
        // A swipe right from mid-thread goes back, as SwiftUI's ThreadBackSwipe.
        app.collectionViews["thread-timeline"].swipeRight()
        XCTAssertTrue(thread.waitForNonExistence(timeout: 10), "A swipe right closes the thread")
        let link = app.buttons["view-thread-b00000000003000"]
        XCTAssertTrue(link.waitForExistence(timeout: 10), "The root shows its thread link")
        XCTAssertEqual(link.value as? String, "1 reply")
        screenshot("uikit-thread-link")
        link.tap()
        XCTAssertTrue(thread.waitForExistence(timeout: 10))
        XCTAssertTrue(reply.waitForExistence(timeout: 10))
    }

    /// A channel row's menu is SwiftUI inside a UIKit cell; its Leave channel
    /// confirmation must still present from there.
    func testLeaveChannelConfirmationFromHostedRow() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.buttons["back-to-browse"].tap()
        let options = app.buttons["channel-options-chan00000001"]
        XCTAssertTrue(options.waitForExistence(timeout: 10))
        options.tap()
        app.buttons["Leave channel"].tap()
        let title = app.staticTexts["Leave #general?"]
        XCTAssertTrue(title.waitForExistence(timeout: 10), "Leave channel's confirmation presents from the hosted row")
        screenshot("uikit-leave-channel")
        app.buttons["Cancel"].tap()
        XCTAssertTrue(title.waitForNonExistence(timeout: 10))
    }

    /// A thread that fails to load says why, with Retry, as SwiftUI's.
    func testThreadLoadErrorRetries() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        control(["failure": ["path": "/api/chat/channels/chan00000001/messages/b00000000003000/thread", "method": "GET",
                             "status": 503, "error": "TEST FIXTURE: thread unavailable."]])
        app.textViews["message-text-\(Self.latest)"].press(forDuration: 0.8)
        XCTAssertTrue(app.descendants(matching: .any)["message-actions-sheet"].waitForExistence(timeout: 10))
        app.buttons["Reply in thread"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["thread-view"].waitForExistence(timeout: 10))
        let error = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "thread unavailable")).firstMatch
        XCTAssertTrue(error.waitForExistence(timeout: 10), "The thread shows its load error")
        screenshot("uikit-thread-error")
        app.descendants(matching: .any)["timeline-edge"].buttons["Retry"].tap()
        XCTAssertTrue(error.waitForNonExistence(timeout: 10), "Retry loads the thread")
    }

    /// Reply, from message actions, raises the keyboard in the channel's
    /// composer; Reply in thread does the same for the thread's.
    func testReplyFocusesTheComposer() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        let focused = NSPredicate(format: "hasKeyboardFocus == true")
        app.textViews["message-text-\(Self.latest)"].press(forDuration: 0.8)
        XCTAssertTrue(app.descendants(matching: .any)["message-actions-sheet"].waitForExistence(timeout: 10))
        app.buttons.matching(NSPredicate(format: "label == %@", "Reply")).firstMatch.tap()
        XCTAssertEqual(XCTWaiter.wait(for: [expectation(for: focused, evaluatedWith: composer)], timeout: 10), .completed,
                       "Reply focuses the channel composer")
        screenshot("uikit-reply-focus")

        app.textViews["message-text-\(Self.latest)"].press(forDuration: 0.8)
        XCTAssertTrue(app.descendants(matching: .any)["message-actions-sheet"].waitForExistence(timeout: 10))
        app.buttons["Reply in thread"].tap()
        let thread = app.textViews["thread-composer"]
        XCTAssertTrue(thread.waitForExistence(timeout: 10))
        XCTAssertEqual(XCTWaiter.wait(for: [expectation(for: focused, evaluatedWith: thread)], timeout: 10), .completed,
                       "Reply in thread focuses the thread composer")
    }

    /// A link in the draft gets SwiftUI's card inside the box, with ×; a long
    /// draft shows the counter from 3,000 characters.
    func testComposerLinkPreviewAndCounter() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        composer.tap()
        composer.typeText("see https://example.com/fixture ")
        let remove = app.buttons["Remove link preview"]
        XCTAssertTrue(remove.waitForExistence(timeout: 5), "The draft's link gets a card")
        XCTAssertTrue(remove.isHittable, "The box grows to show it")
        screenshot("uikit-composer-link-preview")
        remove.tap()
        XCTAssertTrue(remove.waitForNonExistence(timeout: 5))

        UIPasteboard.general.string = String(repeating: "a", count: 3100)
        composer.press(forDuration: 1.0)
        let paste = app.menuItems["Paste"].exists ? app.menuItems["Paste"] : app.buttons["Paste"]
        XCTAssertTrue(paste.waitForExistence(timeout: 5))
        paste.tap()
        let counter = app.staticTexts.matching(NSPredicate(format: "label ENDSWITH %@", "of 4,000 characters")).firstMatch
        XCTAssertTrue(counter.waitForExistence(timeout: 10), "The counter shows from 3,000")
        screenshot("uikit-composer-counter")
    }

    /// Someone typing shows above the composer and clears when they stop.
    func testTypingIndicator() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        let composerFrame = composer.frame
        let messageFrame = row(Self.latest).frame
        control(["typing": ["channelId": "chan00000001"]])
        let typing = app.otherElements["typing-indicator"]
        XCTAssertTrue(typing.waitForExistence(timeout: 10))
        XCTAssertEqual(typing.label, "Maya is typing…")
        XCTAssertEqual(composer.frame.minY, composerFrame.minY, accuracy: 1)
        XCTAssertEqual(row(Self.latest).frame.minY, messageFrame.minY, accuracy: 1)
        screenshot("uikit-typing")
        control(["typing": ["channelId": "chan00000001", "active": false]])
        XCTAssertTrue(typing.waitForNonExistence(timeout: 10), "Stopping clears it")
        XCTAssertEqual(composer.frame.minY, composerFrame.minY, accuracy: 1)
        XCTAssertEqual(row(Self.latest).frame.minY, messageFrame.minY, accuracy: 1)
    }

    /// A person pill opens their card, as do the author's avatar and the members list.
    func testMentionAndAuthorCards() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        composer.tap()
        composer.typeText("TEST FIXTURE ping @maya about it")
        app.buttons["send-message"].tap()
        let sent = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "ping @maya")).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        let pill = sent.links.firstMatch
        XCTAssertTrue(pill.waitForExistence(timeout: 10), "The resolved mention is a link VoiceOver can reach")
        pill.tap()
        let card = app.descendants(matching: .any)["mention-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 10), "Tapping a pill opens the card")
        XCTAssertTrue(app.buttons["mention-card-message"].waitForExistence(timeout: 5), "Maya's card offers Message")
        screenshot("uikit-mention-card")
        app.swipeDown(velocity: .fast)
        XCTAssertTrue(card.waitForNonExistence(timeout: 10))

        // The author's avatar on the newest unbroken row.
        let latest = row(Self.latest)
        latest.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 34, dy: 26)).tap()
        XCTAssertTrue(card.waitForExistence(timeout: 10), "The author's avatar opens their card")
        app.swipeDown(velocity: .fast)
        XCTAssertTrue(card.waitForNonExistence(timeout: 10))

        app.buttons["channel-title-menu"].tap()
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Members")).firstMatch.tap()
        let member = app.cells.matching(NSPredicate(format: "label == %@", "Open profile for Maya")).firstMatch
        XCTAssertTrue(member.waitForExistence(timeout: 10))
        member.tap()
        XCTAssertTrue(card.waitForExistence(timeout: 10), "A member opens their card")
    }

    /// `@` and `:` open the shared autocomplete above the composer; a picked
    /// row replaces the typed token, and the sent mention resolves to a pill.
    func testComposerAutocomplete() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        composer.tap()
        composer.typeText("TEST FIXTURE autocomplete @ma")
        let mentions = app.descendants(matching: .any)["mention-suggestions"]
        XCTAssertTrue(mentions.waitForExistence(timeout: 10))
        let maya = app.buttons["mention-suggestion-maya"]
        XCTAssertTrue(maya.exists)
        XCTAssertTrue(maya.label.hasPrefix("Maya"), maya.label)
        screenshot("uikit-mention-autocomplete")
        maya.tap()
        XCTAssertTrue(mentions.waitForNonExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE autocomplete @maya ")

        composer.typeText("party :tad")
        let emoji = app.descendants(matching: .any)["emoji-suggestions"]
        XCTAssertTrue(emoji.waitForExistence(timeout: 10))
        let first = emoji.buttons.firstMatch
        XCTAssertTrue(first.label.hasSuffix(", emoji"), first.label)
        screenshot("uikit-emoji-autocomplete")
        first.tap()
        XCTAssertTrue(emoji.waitForNonExistence(timeout: 5))
        let value = composer.value as? String ?? ""
        XCTAssertTrue(value.hasPrefix("TEST FIXTURE autocomplete @maya party ") && !value.contains(":tad"), value)

        app.buttons["send-message"].tap()
        let sent = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "autocomplete @maya party")).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        XCTAssertTrue(sent.links.firstMatch.waitForExistence(timeout: 10), "The picked mention resolves to a pill")
    }

    /// The draft renders like the message will: Markdown delimiters stay in
    /// the source but draw nothing, and a resolved mention is highlighted.
    /// (Typing on after a closing delimiter is covered by the shared
    /// formatting rules, not here.)
    func testComposerDraftFormatting() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        let composer = app.textViews["message-composer"]
        composer.tap()
        composer.typeText("TEST FIXTURE for @maya **bold**")
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE for @maya **bold**", "VoiceOver reads the draft as it will be sent")
        screenshot("uikit-draft-formatting")
        app.buttons["send-message"].tap()
        let sent = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "for @maya bold")).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        XCTAssertTrue(sent.links.firstMatch.exists, "The mention resolved")
        screenshot("uikit-draft-formatting-sent")
    }

    /// Notifications sits atop Direct messages and opens the activity inbox.
    func testActivityInbox() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.buttons["back-to-browse"].tap()
        let entry = app.cells["activity-inbox"]
        XCTAssertTrue(entry.waitForExistence(timeout: 10))
        XCTAssertEqual(entry.label, "Notifications")
        entry.tap()
        let mention = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "check the narrow layout")).firstMatch
        XCTAssertTrue(mention.waitForExistence(timeout: 10), "The fixture's mention is listed")
        screenshot("uikit-activity-inbox")
        app.buttons["Done"].firstMatch.tap()
        XCTAssertTrue(mention.waitForNonExistence(timeout: 10))
        XCTAssertTrue(entry.exists)
    }

    /// The feedback team's inbox: Feedback under Notifications pushes the
    /// list; a report visits a space the owner is not in, at the report; Back
    /// returns to the inbox.
    func testFeedbackInboxVisitsAReport() {
        control(["feedbackTeam": true])
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.buttons["back-to-browse"].tap()
        let inbox = app.cells["feedback-inbox"]
        XCTAssertTrue(inbox.waitForExistence(timeout: 10))
        let waiting = NSPredicate(format: "value == %@", "1 feedback conversations waiting for a reply")
        XCTAssertEqual(XCTWaiter.wait(for: [expectation(for: waiting, evaluatedWith: inbox)], timeout: 10), .completed,
                       "The row counts reports waiting for a reply")
        screenshot("uikit-browse-feedback-rows")
        inbox.tap()
        let report = app.buttons["feedback-report-chan00000009m02"]
        XCTAssertTrue(report.waitForExistence(timeout: 10))
        screenshot("uikit-feedback-inbox")
        report.tap()
        let title = app.buttons["channel-title-menu"]
        XCTAssertTrue(row("00000000-0000-4000-8000-000000000092").waitForExistence(timeout: 10), "The visit opens at the report")
        XCTAssertEqual(title.label, "#plans · Maya's Workshop options")
        XCTAssertEqual(app.textViews["message-composer"].label, "Message #plans")
        screenshot("uikit-feedback-visit")
        title.tap()
        XCTAssertTrue(app.buttons["Pins"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Members")).firstMatch.exists,
                       "A visit has no member list")
        app.buttons["Pins"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["No pinned messages"].waitForExistence(timeout: 5))
        app.buttons["close-pins"].tap()
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        app.buttons["back-to-browse"].tap()
        XCTAssertTrue(report.waitForExistence(timeout: 10), "Back returns to the inbox")
        app.buttons["feedback-back"].firstMatch.tap()
        XCTAssertTrue(inbox.waitForExistence(timeout: 10))
    }

    /// The account menu opens SwiftUI's release notes sheet.
    func testWhatsNew() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.buttons["back-to-browse"].tap()
        app.buttons["account-settings"].tap()
        let item = app.buttons["open-whats-new"].firstMatch
        XCTAssertTrue(item.waitForExistence(timeout: 5))
        item.tap()
        let sheet = app.descendants(matching: .any)["whats-new"].firstMatch
        XCTAssertTrue(sheet.waitForExistence(timeout: 10))
        screenshot("uikit-whats-new")
        app.buttons["Done"].firstMatch.tap()
        XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
    }

    /// Owners reach Invite people and Space settings from the space name,
    /// Create channel from Channels, and channel settings from a held channel.
    func testOwnerWorkflows() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.buttons["back-to-browse"].tap()
        let space = app.buttons["selected-space-name"]
        XCTAssertTrue(space.waitForExistence(timeout: 10))
        space.tap()
        app.buttons["Invite people…"].firstMatch.tap()
        XCTAssertTrue(app.buttons["default-invite-link-copy"].waitForExistence(timeout: 10), "The primary invite link is ready to copy")
        screenshot("uikit-invite")
        app.buttons["Close"].firstMatch.tap()
        XCTAssertTrue(app.buttons["default-invite-link-copy"].waitForNonExistence(timeout: 5))

        space.tap()
        app.buttons["Space settings"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Manage space"].firstMatch.waitForExistence(timeout: 10))
        screenshot("uikit-space-settings")
        app.buttons["Close"].firstMatch.tap()

        app.buttons["create-channel"].tap()
        XCTAssertTrue(app.staticTexts["Create a channel"].firstMatch.waitForExistence(timeout: 10))
        screenshot("uikit-create-channel")
        app.buttons["Close"].firstMatch.tap()

        app.buttons["channel-options-chan00000002"].tap()
        app.buttons["Channel settings"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Channel settings"].firstMatch.waitForExistence(timeout: 10))
        screenshot("uikit-channel-settings")
        app.buttons["Close"].firstMatch.tap()

        // Browse channels: search the space's channels, joined or not.
        space.tap()
        app.buttons["browse-channels"].firstMatch.tap()
        let search = app.descendants(matching: .any)["browse-channels-search"].firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.tap()
        search.typeText("des")
        XCTAssertTrue(app.cells["browse-channel-design"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.cells["browse-channel-general"].exists, "The search filters channels")
        screenshot("uikit-browse-channels")
        search.typeText("\n")

        app.buttons["account-settings"].tap()
        XCTAssertTrue(app.buttons["sound-effects"].firstMatch.waitForExistence(timeout: 5), "Audio settings are in the account menu")
        XCTAssertTrue(app.buttons["voice-messages-send-on-stop"].firstMatch.exists)
        app.buttons["open-privacy"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Privacy"].firstMatch.waitForExistence(timeout: 10))
        screenshot("uikit-privacy")
    }

    /// Every field of Invite people's custom link form and its Create link stay
    /// reachable while the keyboard is up.
    func testInviteFormReachableWithKeyboard() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        if app.buttons["back-to-browse"].isHittable { app.buttons["back-to-browse"].tap() }
        let space = app.buttons["selected-space-name"]
        XCTAssertTrue(space.waitForExistence(timeout: 10))
        space.tap()
        app.buttons["Invite people…"].firstMatch.tap()
        XCTAssertTrue(app.buttons["default-invite-link-copy"].waitForExistence(timeout: 10))
        app.buttons["default-invite-link-options"].firstMatch.tap()
        app.buttons["Custom links"].firstMatch.tap()
        let maxUses = app.textFields["invite-link-max-uses"]
        XCTAssertTrue(maxUses.waitForExistence(timeout: 5))
        let scroll = app.scrollViews["space-settings-scroll"]
        for _ in 0..<4 where !maxUses.isHittable { scroll.swipeUp() }
        maxUses.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 10))
        maxUses.typeText("5")
        sleep(1)
        screenshot("uikit-invite-keyboard")
        let dialog = scroll.frame
        XCTAssertLessThanOrEqual(dialog.maxY, keyboard.frame.minY + 1, "The dialog ends above the keyboard")
        // The keyboard shrinks the dialog: the field being edited scrolls fully into view by itself.
        XCTAssertLessThanOrEqual(maxUses.frame.maxY, dialog.maxY + 1, "The focused field is not clipped by the dialog's edge")
        XCTAssertGreaterThanOrEqual(maxUses.frame.minY, dialog.minY - 1)
        // Each control can be scrolled clear of the keyboard and used. A short
        // drag that settles before lifting scrolls about as far as a finger
        // does; a swipe can fling past the 265pt an iPhone SE leaves visible.
        let create = app.buttons["invite-link-create"]
        let username = app.textFields["Exact username"]
        func visible(_ element: XCUIElement) -> Bool {
            element.isHittable && element.frame.minY >= dialog.minY - 1 && element.frame.maxY <= keyboard.frame.minY
        }
        func reveal(_ element: XCUIElement) {
            for _ in 0..<20 where !visible(element) {
                let start = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
                let step: CGFloat = element.frame.minY < dialog.minY ? 100 : -100
                start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: step)),
                            withVelocity: .slow, thenHoldForDuration: 0.3)
            }
            XCTAssertTrue(visible(element), "\(element) is reachable between the dialog's header and the keyboard")
        }
        for element in [app.textFields["invite-link-expiry-number"], maxUses, create, username] { reveal(element) }
        // Create works while Max uses still has the number pad up: scroll to it and tap it.
        if UIDevice.current.userInterfaceIdiom == .phone {
            XCTAssertTrue(keyboard.keys["0"].exists && !keyboard.keys["a"].exists, "The number pad is still open")
        }
        reveal(create)
        XCTAssertTrue(keyboard.exists, "The keyboard is still open")
        screenshot("uikit-invite-create-keyboard")
        let copies = app.buttons.matching(NSPredicate(format: "label == %@", "Copy invite link"))
        let before = copies.count
        create.tap()
        let made = NSPredicate { _, _ in copies.count > before }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: made, object: nil)], timeout: 10), .completed,
                       "Create link made a link")
        // iPad simulators may collapse the keyboard to its shortcut bar; typing proves the focus.
        reveal(username)
        username.tap()
        username.typeText("sa")
        XCTAssertEqual(username.value as? String, "sa", "The username field takes typing")
    }

    /// iPad: Browse beside the conversation, as SwiftUI above 760 points, in
    /// portrait and landscape. The member list tiles once there is room.
    func testWideLayoutFollowsRotation() throws {
        let device = XCUIDevice.shared
        device.orientation = .portrait
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        guard app.windows.firstMatch.frame.width > 760 else {
            throw XCTSkip("Run on an iPad simulator (CAPER_IOS_SIMULATOR_DESTINATION) for the wide layout.")
        }
        defer { device.orientation = .portrait }
        let channels = app.collectionViews["channel-list"]
        let timeline = app.collectionViews["message-timeline"]
        let members = app.collectionViews["member-list"]
        func assertWide(_ name: String, tiled: Bool) {
            sleep(1)
            XCTAssertTrue(channels.isHittable, "\(name): Browse shows")
            XCTAssertTrue(timeline.isHittable, "\(name): the conversation shows")
            XCTAssertFalse(app.buttons["back-to-browse"].isHittable, "\(name): no Back to Browse")
            XCTAssertLessThan(channels.frame.maxX, timeline.frame.minX + 1, "\(name): side by side")
            XCTAssertTrue(app.descendants(matching: .any)["channel-sidebar-resize"].exists)
            XCTAssertEqual(members.exists, tiled, "\(name): the member list tiles only when it fits")
            if tiled { XCTAssertLessThan(timeline.frame.maxX, members.frame.minX + 1, "\(name): members beside the timeline") }
        }
        assertWide("portrait", tiled: app.windows.firstMatch.frame.width - 340 >= 540)
        screenshot("uikit-ipad-portrait")
        device.orientation = .landscapeLeft
        assertWide("landscape", tiled: true)
        screenshot("uikit-ipad-landscape")
        // Rows rewrapped to the new width keep one-line headers beside their avatars.
        for header in timeline.otherElements.matching(NSPredicate(format: "label CONTAINS %@", ", 10:")).allElementsBoundByIndex {
            XCTAssertLessThan(header.frame.height, 30, "\(header.label): the header stays one line after rotating")
        }
        // Members toggles from the header, and a chosen channel opens beside Browse.
        app.buttons["Hide member list"].firstMatch.tap()
        XCTAssertFalse(members.waitForExistence(timeout: 2))
        app.buttons["Show member list"].firstMatch.tap()
        XCTAssertTrue(members.waitForExistence(timeout: 5))
        app.buttons["channel-chan00000002"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["selected-channel-name"].firstMatch.waitForExistence(timeout: 10))
        XCTAssertTrue(channels.isHittable, "Opening a channel keeps Browse")
        // As SwiftUI: a list the reader chose to show stays shown, over the conversation if it no longer fits.
        device.orientation = .portrait
        sleep(1)
        XCTAssertTrue(channels.isHittable && timeline.isHittable, "portrait again: still side by side")
        XCTAssertTrue(members.exists, "portrait again: the shown member list stays")
    }

    /// People in a channel's voice: Browse grows its row into a live card, and
    /// the open channel's strip offers Join under the header.
    func testVoiceStripAndLiveRoom() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        XCTAssertFalse(app.descendants(matching: .any)["voice-strip"].exists, "Nobody is in #general's voice")
        app.buttons["back-to-browse"].tap()
        let count = app.descendants(matching: .any)["voice-count-chan00000002"]
        XCTAssertTrue(count.waitForExistence(timeout: 10), "#design's row shows who is in voice")
        screenshot("uikit-browse-live")
        app.buttons["channel-chan00000002"].tap()
        let strip = app.descendants(matching: .any)["voice-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 10), "The open channel's strip offers its voice")
        XCTAssertTrue(strip.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Join")).firstMatch.exists)
        screenshot("uikit-voice-strip")
    }

    /// A caper:// invite link opened from another app shows Join with invite link
    /// once the workspace is ready.
    func testOpenedInviteLinkShowsJoinDialog() {
        let app = launch(openURL: "caper://invite/ABCDEFGHJK")
        XCTAssertTrue(app.staticTexts["Join with invite link"].firstMatch.waitForExistence(timeout: 30))
        screenshot("uikit-join-invite")
    }

    /// An account in no spaces names its first one, which opens at #general.
    func testFirstSpaceCreatesAndOpens() {
        control(["noSpaces": true])
        let app = launch()
        XCTAssertTrue(app.staticTexts["Name your space"].waitForExistence(timeout: 30))
        let create = app.buttons["first-space-create"]
        XCTAssertFalse(create.isEnabled, "An empty name can't be created")
        screenshot("uikit-first-space")
        app.textFields.firstMatch.tap()
        app.textFields.firstMatch.typeText("Gaming nights")
        create.tap()
        let title = app.buttons["channel-title-menu"]
        XCTAssertTrue(title.waitForExistence(timeout: 15))
        XCTAssertTrue(title.label.hasPrefix("#general"), title.label)
    }

    /// Spaces failing to load: Try again recovers; Direct messages opens Browse.
    func testSpacesUnavailableRetriesAndOpensDirectMessages() {
        control(["noSpaces": true])
        control(["failure": ["path": "/api/spaces", "method": "GET", "status": 503, "persistent": true]])
        let app = launch()
        XCTAssertTrue(app.staticTexts["Spaces are unavailable."].waitForExistence(timeout: 30))
        screenshot("uikit-spaces-unavailable")
        app.buttons["Try again"].tap()
        XCTAssertTrue(app.staticTexts["Spaces are unavailable."].waitForExistence(timeout: 5), "Still failing")
        control(["clearFailures": true])
        app.buttons["Try again"].tap()
        XCTAssertTrue(app.staticTexts["Name your space"].waitForExistence(timeout: 15))
        control(["failure": ["path": "/api/spaces", "method": "GET", "status": 503, "persistent": true]])
        app.terminate()
        _ = launch()
        XCTAssertTrue(self.app.staticTexts["Spaces are unavailable."].waitForExistence(timeout: 30))
        self.app.buttons["Direct messages"].tap()
        XCTAssertTrue(self.app.buttons["new-direct-message"].waitForExistence(timeout: 10), "Browse lists direct messages")
        screenshot("uikit-spaces-unavailable-browse")
    }

    /// A space that fails to open shows SwiftUI's banner; Retry opening opens it.
    func testNavigationErrorRetriesOpening() {
        control(["failure": ["path": "/api/spaces/space0000001", "method": "GET", "status": 503]])
        let app = launch()
        let banner = app.otherElements["navigation-error"]
        XCTAssertTrue(banner.waitForExistence(timeout: 30))
        screenshot("uikit-navigation-error")
        app.buttons["Retry opening"].tap()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 15))
        XCTAssertFalse(banner.exists)
    }

    /// An empty conversation says so; the note-to-self DM says what it is for.
    func testEmptyConversation() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.buttons["back-to-browse"].tap()
        app.cells["dm-self"].tap()
        XCTAssertTrue(app.staticTexts["You can message yourself here to keep notes, reminders, and ideas."].waitForExistence(timeout: 15))
        screenshot("uikit-conversation-empty")
    }

    /// Browse lists DMs under the channels; New message opens a DM by exact
    /// username, and the conversation sends like a channel.
    func testNewDirectMessageOpensAndSends() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        app.buttons["back-to-browse"].tap()
        let newMessage = app.cells["new-message"]
        XCTAssertTrue(newMessage.waitForExistence(timeout: 10), "Browse lists DMs with New message")
        XCTAssertTrue(app.cells["dm-self"].exists, "Your own notes are listed")
        newMessage.tap()
        let username = app.textFields["dm-username"]
        XCTAssertTrue(username.waitForExistence(timeout: 10))
        username.typeText("maya")
        let submit = app.buttons["dm-create-submit"]
        // A known person opens at once; anyone else is found first.
        if submit.label == "Find person" {
            submit.tap()
            expectation(for: NSPredicate(format: "label == %@", "Open conversation"), evaluatedWith: submit)
            waitForExpectations(timeout: 10)
        }
        XCTAssertTrue(app.staticTexts["Maya (@maya)"].exists, "The sheet names who you'll message")
        submit.tap()
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 10))
        let opened = NSPredicate(format: "label == %@", "Message Maya")
        expectation(for: opened, evaluatedWith: composer)
        waitForExpectations(timeout: 10)
        XCTAssertEqual(app.buttons["channel-title-menu"].label, "Maya options")
        composer.tap()
        composer.typeText("TEST FIXTURE — hi from a DM")
        app.buttons["send-message"].tap()
        XCTAssertTrue(app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "hi from a DM")).firstMatch.waitForExistence(timeout: 10))
        screenshot("uikit-dm")
        app.buttons["back-to-browse"].tap()
        let dm = app.cells["dm-dm0000000004"]
        XCTAssertTrue(dm.waitForExistence(timeout: 10), "The new DM joins the list")
        XCTAssertEqual(dm.value as? String, "Selected")
        screenshot("uikit-browse-dms")
    }

    /// An incoming request opens read-only with Accept, Decline and Block in
    /// place of the composer; blocking collapses the peer's messages.
    func testMessageRequestAcceptBlockAndUnblock() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        control(["messageRequest": [String: String]()])
        app.buttons["back-to-browse"].tap()
        let requests = app.cells["message-requests"]
        XCTAssertTrue(requests.waitForExistence(timeout: 30), "An incoming request adds the Message requests row")
        XCTAssertEqual(requests.value as? String, "1 request")
        requests.tap()
        let request = app.buttons["message-request-dm0000000003"]
        XCTAssertTrue(request.waitForExistence(timeout: 10), "The requests page lists Jordan")
        screenshot("uikit-requests")
        request.tap()
        let accept = app.buttons["message-request-accept"]
        XCTAssertTrue(accept.waitForExistence(timeout: 10), "A request replaces the composer")
        XCTAssertFalse(app.textViews["message-composer"].isHittable)
        screenshot("uikit-request-bar")
        accept.tap()
        let composer = app.textViews["message-composer"]
        let accepted = NSPredicate(format: "hittable == true AND label == %@", "Message Jordan")
        expectation(for: accepted, evaluatedWith: composer)
        waitForExpectations(timeout: 10)

        app.buttons["channel-title-menu"].tap()
        app.buttons["Block @jordan…"].tap()
        let confirm = app.alerts.buttons["Block"].firstMatch.exists ? app.alerts.buttons["Block"] : app.buttons["Block"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        confirm.tap()
        let unblock = app.buttons["dm-blocked-unblock"]
        XCTAssertTrue(unblock.waitForExistence(timeout: 10), "Blocking replaces the composer with Unblock")
        let run = app.cells.matching(NSPredicate(format: "identifier BEGINSWITH %@", "blocked-")).firstMatch
        XCTAssertTrue(run.waitForExistence(timeout: 5), "The blocked peer's messages collapse")
        let show = run.buttons["blocked-run-toggle"]
        XCTAssertEqual(show.label, "Show 1 blocked message")
        show.tap()
        XCTAssertEqual(run.buttons["blocked-run-toggle"].label, "Hide 1 blocked message")
        screenshot("uikit-dm-blocked")
        unblock.tap()
        expectation(for: NSPredicate(format: "hittable == true"), evaluatedWith: composer)
        waitForExpectations(timeout: 10)
        XCTAssertFalse(run.exists, "Unblocking restores the messages")
    }

    /// VoiceOver reads who and when from each row's header and can move into
    /// the text itself; controls carry labels.
    func testAccessibilityLabels() {
        let app = launch()
        let latest = row(Self.latest)
        XCTAssertTrue(latest.waitForExistence(timeout: 30))
        let text = app.textViews["message-text-\(Self.latest)"]
        XCTAssertTrue(text.exists)
        XCTAssertFalse((text.value as? String ?? "").isEmpty)
        let headers = latest.descendants(matching: .any).matching(NSPredicate(format: "label MATCHES %@", ".+, [0-9]{1,2}:[0-9]{2}.*"))
        XCTAssertGreaterThan(headers.count + (text.label.isEmpty ? 0 : 1), 0)
        XCTAssertEqual(app.buttons["back-to-browse"].label, "Back to Browse")
        XCTAssertEqual(app.buttons["send-message"].label, "Send message")
        XCTAssertEqual(app.textViews["message-composer"].label, "Message #general")
        let hierarchy = XCTAttachment(string: app.debugDescription)
        hierarchy.name = "accessibility-conversation"
        hierarchy.lifetime = .keepAlways
        add(hierarchy)
    }

    /// Launch with the last session's conversation cached paints it at once.
    func testStartupPaintsCachedConversation() {
        // First launch writes the cache on backgrounding.
        var app = launch(cache: true)
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        sleep(2)
        XCUIDevice.shared.press(.home)
        sleep(2)
        app.terminate()
        app = launch(cache: true)
        let frame = app.windows.matching(NSPredicate(format: "identifier BEGINSWITH 'first-frame-'")).firstMatch
        XCTAssertTrue(frame.waitForExistence(timeout: 10))
        XCTAssertTrue(frame.identifier.hasSuffix("cached-true"), frame.identifier)
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 5))
        let attachment = XCTAttachment(string: frame.identifier)
        attachment.name = "first-frame"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}

/// The same fixture states captured from the UIKit client and, when it is
/// installed on the simulator, the shipping SwiftUI app (`chat.caper.ios`), so
/// the pairs can be compared. Launch time is measured the same way for both.
@MainActor
final class CaperUIKitParityTests: XCTestCase {
    private static let swiftUI = "chat.caper.ios"
    private static let uiKit = "chat.caper.uikit-development"

    private func configured(_ bundle: String, signedIn: Bool = true, cache: Bool = false, fixture: String? = nil) -> XCUIApplication {
        let app = XCUIApplication(bundleIdentifier: bundle)
        app.launchArguments += ["-caper.space-landing.owner0000001", "", "-caper.channelSidebarChannelsShare", "0.5"]
        app.launchEnvironment["CAPER_TEST_MODE"] = "parity"
        app.launchEnvironment["CAPER_API_BASE_URL"] = "http://127.0.0.1:3001"
        if signedIn {
            app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-owner-token"
            app.launchEnvironment["CAPER_TEST_SPACE_ID"] = "space0000001"
        }
        if cache { app.launchEnvironment["CAPER_UIKIT_STARTUP_CACHE"] = "1" }
        if let fixture { app.launchEnvironment["CAPER_UI_FIXTURE"] = fixture }
        return app
    }

    /// The pairs need the shipping SwiftUI app on the same simulator. XCUITest
    /// can't tell an uninstalled app from a stopped one, so the run says so:
    /// `CAPER_PARITY_SWIFTUI=1 uikit.sh test` (TEST_RUNNER_CAPER_PARITY_SWIFTUI=1 for xcodebuild).
    private func requireSwiftUI() throws {
        guard ProcessInfo.processInfo.environment["CAPER_PARITY_SWIFTUI"] == "1" else {
            throw XCTSkip("Install the SwiftUI iOS app (chat.caper.ios) on this simulator and set CAPER_PARITY_SWIFTUI=1 to capture references.")
        }
    }

    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testConversationAndBrowsePairs() throws {
        try requireSwiftUI()
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            let app = configured(bundle)
            app.launch()
            let back = app.buttons["Back to Browse"]
            XCTAssertTrue(back.waitForExistence(timeout: 30))
            // Rows settle and the composer's draft restoration finishes.
            sleep(3)
            capture("\(prefix)-conversation")
            back.tap()
            sleep(2)
            capture("\(prefix)-browse")
            app.terminate()
        }
    }

    func testSignInPairs() throws {
        try requireSwiftUI()
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            let app = configured(bundle, signedIn: false)
            app.launch()
            XCTAssertTrue(app.textFields["Email address"].waitForExistence(timeout: 30))
            sleep(2)
            capture("\(prefix)-login")
            app.terminate()
        }
    }

    /// A failed send in each app: unknown outcome (Retry send), then rejected
    /// (Edit and Dismiss).
    func testFailedSendPairs() throws {
        try requireSwiftUI()
        let path = "/api/chat/channels/chan00000001/messages"
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            control(["reset": true])
            let app = configured(bundle)
            app.launch()
            let composer = app.textViews["message-composer"]
            XCTAssertTrue(composer.waitForExistence(timeout: 30))
            sleep(2)
            // SwiftUI may restore Browse open over the conversation.
            let close = app.buttons["Close navigation"]
            if close.exists && close.isHittable { close.tap(); sleep(1) }
            for (status, text, action) in [(503, "TEST FIXTURE unsure", "Retry send"), (422, "TEST FIXTURE refused", "Edit")] {
                control(["failure": ["path": path, "method": "POST", "status": status, "error": "TEST FIXTURE: refused."]])
                composer.tap()
                composer.typeText(text)
                app.buttons["Send message"].firstMatch.tap()
                XCTAssertTrue(app.buttons[action].firstMatch.waitForExistence(timeout: 10), "\(prefix) \(action)")
                sleep(1)
                capture("\(prefix)-pending-\(status)")
                if status == 503 { app.buttons[action].firstMatch.tap(); sleep(2) }
            }
            app.terminate()
        }
    }

    /// Notes to self with the keyboard up: each app keeps its header in place
    /// and the same gap between Send and the keyboard.
    func testDirectMessageKeyboardPairs() throws {
        try requireSwiftUI()
        var gaps: [String: CGFloat] = [:]
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            control(["reset": true])
            let app = configured(bundle)
            app.launch()
            let back = app.buttons["Back to Browse"]
            XCTAssertTrue(back.waitForExistence(timeout: 30), prefix)
            back.tap()
            // A cell in UIKit, a button in SwiftUI.
            let notes = app.descendants(matching: .any)["dm-self"].firstMatch
            XCTAssertTrue(notes.waitForExistence(timeout: 10), prefix)
            notes.tap()
            let composer = app.textViews["message-composer"]
            XCTAssertTrue(composer.waitForExistence(timeout: 10), prefix)
            sleep(1)
            let headerY = back.frame.minY
            composer.tap()
            let keyboard = app.keyboards.firstMatch
            XCTAssertTrue(keyboard.waitForExistence(timeout: 10), prefix)
            composer.typeText("TEST FIXTURE keyboard")
            sleep(1)
            XCTAssertEqual(back.frame.minY, headerY, accuracy: 2, "\(prefix): the keyboard must not move the header")
            gaps[prefix] = keyboard.frame.minY - app.buttons["Send message"].firstMatch.frame.maxY
            capture("\(prefix)-dm-keyboard-open")
            app.terminate()
        }
        XCTAssertEqual(gaps["uikit"] ?? 0, gaps["swiftui"] ?? 0, accuracy: 2, "Send sits as far above the keyboard in both apps: \(gaps)")
    }

    /// A thread whose replies are still loading: each app's two-row skeleton.
    func testThreadLoadingSkeletonPairs() throws {
        try requireSwiftUI()
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            control(["reset": true])
            control(["incomingReply": ["rootId": "b00000000003000", "text": "a reply in the thread"]])
            let app = configured(bundle)
            app.launch()
            let view = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "View thread")).firstMatch
            XCTAssertTrue(view.waitForExistence(timeout: 30), prefix)
            sleep(2)
            control(["holdThreads": true])
            view.tap()
            let skeleton = app.descendants(matching: .any)["Loading thread replies"].firstMatch
            XCTAssertTrue(skeleton.waitForExistence(timeout: 15), prefix)
            sleep(1)
            capture("\(prefix)-thread-loading")
            control(["holdThreads": false])
            XCTAssertTrue(skeleton.waitForNonExistence(timeout: 15), prefix)
            app.terminate()
        }
    }

    /// A channel's own first-page skeleton in each app: an empty #general
    /// reloads after the gateway's resync while its history is held.
    func testChannelLoadingSkeletonPairs() throws {
        try requireSwiftUI()
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            control(["reset": true])
            control(["emptyChannel": "chan00000001"])
            let app = configured(bundle)
            app.launch()
            XCTAssertTrue(app.staticTexts["No messages yet."].waitForExistence(timeout: 30), prefix)
            sleep(2)
            control(["holdHistory": true])
            control(["resync": true])
            let skeleton = app.descendants(matching: .any)["Loading messages"].firstMatch
            XCTAssertTrue(skeleton.waitForExistence(timeout: 15), prefix)
            sleep(1)
            capture("\(prefix)-channel-loading")
            control(["holdHistory": false])
            XCTAssertTrue(skeleton.waitForNonExistence(timeout: 15), prefix)
            XCTAssertTrue(app.staticTexts["No messages yet."].waitForExistence(timeout: 15), prefix)
            app.terminate()
        }
        control(["reset": true])
    }

    /// The unread line in each app, with #general read through seq 2997.
    func testNewMessagesPairs() throws {
        try requireSwiftUI()
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            control(["reset": true])
            control(["channelReads": ["chan00000001": "2997"]])
            let app = configured(bundle)
            app.launch()
            XCTAssertTrue(app.buttons["Back to Browse"].waitForExistence(timeout: 30))
            sleep(4)
            capture("\(prefix)-new-messages")
            XCTAssertTrue(app.descendants(matching: .any)["channel-new-messages"].exists, prefix)
            app.terminate()
        }
    }

    /// The latest message with a reply and a pin, then its actions, Members,
    /// Pins and the thread, in each app.
    func testThreadPinsActionsAndMembersPairs() throws {
        try requireSwiftUI()
        let rows = NSPredicate(format: "identifier IN %@", ["message-row-00000000-0000-4000-9000-000000003000", "message-row-b00000000003000"])
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            control(["reset": true])
            control(["incomingReply": ["rootId": "b00000000003000", "text": "a reply in the thread"]])
            control(["incomingPin": ["channelId": "chan00000001", "messageId": "b00000000003000"]])
            let app = configured(bundle)
            app.launch()
            let latest = app.descendants(matching: .any).matching(rows).firstMatch
            XCTAssertTrue(latest.waitForExistence(timeout: 30), prefix)
            sleep(3)
            capture("\(prefix)-thread-and-pin")

            // On the text, clear of the pin badge above and the thread link below.
            let words = NSPredicate(format: "label CONTAINS %@ OR value CONTAINS %@", "first headphones", "first headphones")
            let text = app.descendants(matching: .any).matching(words).element(boundBy: 0)
            text.press(forDuration: 0.8)
            let sheet = app.descendants(matching: .any)["message-actions-sheet"]
            XCTAssertTrue(sheet.waitForExistence(timeout: 10), prefix)
            sleep(1)
            capture("\(prefix)-message-actions")
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.08)).tap()
            XCTAssertTrue(sheet.waitForNonExistence(timeout: 10), prefix)

            app.buttons["channel-title-menu"].tap()
            app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Members")).firstMatch.tap()
            sleep(2)
            capture("\(prefix)-members")
            app.buttons["Close member list"].firstMatch.tap()
            sleep(1)

            app.buttons["channel-title-menu"].tap()
            app.buttons["Pins"].tap()
            XCTAssertTrue(app.descendants(matching: .any)["pinned-message-b00000000003000"].waitForExistence(timeout: 10), prefix)
            sleep(1)
            capture("\(prefix)-pins")
            let close = app.buttons["Close pins"].firstMatch
            if close.exists { close.tap() } else { app.swipeDown(velocity: .fast) }
            sleep(1)

            app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "View thread")).firstMatch.tap()
            XCTAssertTrue(app.staticTexts["Thread"].waitForExistence(timeout: 10), prefix)
            sleep(2)
            capture("\(prefix)-thread-view")
            app.terminate()
        }
    }

    /// Browse's sheets and other conversations in each app: profile, account
    /// settings (notifications, privacy), space settings, the notifications
    /// inbox, your notes DM and a channel with people in voice.
    func testSheetsDirectMessagesAndVoicePairs() throws {
        try requireSwiftUI()
        let label = { (app: XCUIApplication, text: String) in
            app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", text)).firstMatch
        }
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            control(["reset": true])
            let app = configured(bundle)
            app.launch()
            let back = app.buttons["Back to Browse"]
            XCTAssertTrue(back.waitForExistence(timeout: 30), prefix)
            back.tap()
            sleep(1)

            app.buttons["account-profile"].firstMatch.tap()
            XCTAssertTrue(app.staticTexts["Edit profile"].firstMatch.waitForExistence(timeout: 10), prefix)
            sleep(1)
            capture("\(prefix)-profile")
            app.buttons["Close profile"].firstMatch.tap()
            sleep(1)

            for (item, name, title) in [("open-notifications", "notification-settings", "Notifications"),
                                        ("open-privacy", "privacy", "Privacy")] {
                app.buttons.matching(NSPredicate(format: "label == %@", "Account settings")).firstMatch.tap()
                app.buttons[item].firstMatch.tap()
                XCTAssertTrue(app.staticTexts[title].firstMatch.waitForExistence(timeout: 10), "\(prefix) \(item)")
                sleep(1)
                capture("\(prefix)-\(name)")
                // The dialog's X ("Close", "Close notifications"…), not Browse's.
                let close = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Close' AND label != 'Close navigation'")).firstMatch
                if close.exists { close.tap() } else { app.swipeDown(velocity: .fast) }
                sleep(1)
            }

            app.buttons["selected-space-name"].firstMatch.tap()
            app.buttons["Space settings"].firstMatch.tap()
            XCTAssertTrue(app.staticTexts["Manage space"].firstMatch.waitForExistence(timeout: 10), prefix)
            sleep(1)
            capture("\(prefix)-space-settings")
            app.buttons["Close"].firstMatch.tap()
            sleep(1)

            label(app, "Notifications").tap()
            let mention = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "check the narrow layout")).firstMatch
            XCTAssertTrue(mention.waitForExistence(timeout: 10), prefix)
            sleep(1)
            capture("\(prefix)-activity-inbox")
            app.buttons["Done"].firstMatch.tap()
            sleep(1)

            app.descendants(matching: .any)["dm-self"].firstMatch.tap()
            XCTAssertTrue(app.staticTexts["You can message yourself here to keep notes, reminders, and ideas."].waitForExistence(timeout: 15), prefix)
            sleep(2)
            capture("\(prefix)-dm-self")

            back.tap()
            sleep(1)
            app.descendants(matching: .any)["channel-chan00000002"].firstMatch.tap()
            XCTAssertTrue(app.descendants(matching: .any)["voice-strip"].waitForExistence(timeout: 10), prefix)
            sleep(2)
            capture("\(prefix)-voice-strip")
            // Leave each app in #general: the shipping app restores where it was.
            if !app.buttons["Close navigation"].isHittable { app.buttons["Back to Browse"].tap(); sleep(1) }
            app.descendants(matching: .any)["channel-chan00000001"].firstMatch.tap()
            XCTAssertTrue(app.textViews["message-composer"].waitForExistence(timeout: 10), prefix)
            sleep(1)
            app.terminate()
        }
    }

    /// Your call (the local seven-person roster) in each app: the strip, the
    /// call card in Browse, and the participant sheet a held face opens from
    /// inside the hosted channel row.
    func testCallCardAndParticipantSheetPairs() throws {
        try requireSwiftUI()
        for (bundle, prefix) in [(Self.uiKit, "uikit"), (Self.swiftUI, "swiftui")] {
            control(["reset": true])
            let app = configured(bundle, fixture: "voice-crowd")
            app.launch()
            let open = app.buttons["voice-strip-open"]
            XCTAssertTrue(open.waitForExistence(timeout: 30), "\(prefix): the call's strip")
            sleep(2)
            capture("\(prefix)-call-strip")
            open.tap()
            let card = app.descendants(matching: .any)["voice-call-card"]
            XCTAssertTrue(card.waitForExistence(timeout: 10), prefix)
            sleep(2)
            capture("\(prefix)-call-card")
            app.descendants(matching: .any)["voice-face-fixture-remote"].firstMatch.press(forDuration: 0.8)
            let title = app.descendants(matching: .any)["voice-list-title"]
            XCTAssertTrue(title.waitForExistence(timeout: 10), "\(prefix): holding a face opens everyone in the call")
            sleep(1)
            capture("\(prefix)-call-participants")
            app.descendants(matching: .any)["voice-list-close"].firstMatch.tap()
            XCTAssertTrue(title.waitForNonExistence(timeout: 10), prefix)
            // Leave each app in #general: the shipping app restores where it was.
            if !app.buttons["Close navigation"].isHittable { app.buttons["Back to Browse"].tap(); sleep(1) }
            app.descendants(matching: .any)["channel-chan00000001"].firstMatch.tap()
            XCTAssertTrue(app.textViews["message-composer"].waitForExistence(timeout: 10), prefix)
            sleep(1)
            app.terminate()
        }
    }

    /// XCTest's own launch metric (process start to first frame) for each app.
    func testLaunchUIKitWithCache() {
        // Prime the startup cache once.
        let first = configured(Self.uiKit, cache: true)
        first.launch()
        _ = first.buttons["Back to Browse"].waitForExistence(timeout: 30)
        sleep(2)
        XCUIDevice.shared.press(.home)
        sleep(2)
        first.terminate()
        let options = XCTMeasureOptions()
        options.iterationCount = 5
        measure(metrics: [XCTApplicationLaunchMetric(waitUntilResponsive: false)], options: options) {
            configured(Self.uiKit, cache: true).launch()
        }
    }

    func testLaunchUIKitWithoutCache() {
        let options = XCTMeasureOptions()
        options.iterationCount = 5
        measure(metrics: [XCTApplicationLaunchMetric(waitUntilResponsive: false)], options: options) {
            configured(Self.uiKit).launch()
        }
    }

    func testLaunchSwiftUI() throws {
        try requireSwiftUI()
        let options = XCTMeasureOptions()
        options.iterationCount = 5
        measure(metrics: [XCTApplicationLaunchMetric(waitUntilResponsive: false)], options: options) {
            configured(Self.swiftUI).launch()
        }
    }
}
