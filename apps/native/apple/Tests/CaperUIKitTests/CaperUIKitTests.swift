import XCTest

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

    private func launch(signedIn: Bool = true, cache: Bool = false, openURL: String? = nil) -> XCUIApplication {
        let app = XCUIApplication()
        if let openURL { app.launchEnvironment["CAPER_TEST_OPEN_URL"] = openURL }
        app.launchArguments += ["-caper.space-landing.owner0000001", ""]
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

    /// Fixture controls: reset, live messages from Alex.
    private func control(_ body: [String: Any]) {
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
        screenshot("uikit-browse")
        design.tap()
        let title = app.buttons["channel-title-menu"]
        XCTAssertTrue(title.waitForExistence(timeout: 10))
        let switched = NSPredicate(format: "label == %@", "#design options")
        expectation(for: switched, evaluatedWith: title)
        waitForExpectations(timeout: 10)
        XCTAssertEqual(app.textViews["message-composer"].label, "Message #design")
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
        if allow.waitForExistence(timeout: 5) { allow.tap() }
        let tiles = app.buttons.matching(identifier: "attach-drawer-tile")
        XCTAssertTrue(tiles.firstMatch.waitForExistence(timeout: 15), "The simulator's sample photos must appear")
        tiles.firstMatch.tap()
        XCTAssertTrue(app.buttons["attach-drawer-send"].waitForExistence(timeout: 10))
        screenshot("uikit-attach-drawer")
        app.buttons["attach-drawer-resize"].swipeDown()
        XCTAssertTrue(app.otherElements["attach-drawer"].waitForNonExistence(timeout: 10))
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
        let badge = latest.staticTexts["Pinned by you"]
        XCTAssertTrue(badge.waitForExistence(timeout: 10), "Pinned rows say who pinned them")
        screenshot("uikit-pinned-row")

        app.buttons["channel-title-menu"].tap()
        app.buttons["Pins"].tap()
        let card = app.descendants(matching: .any)["pinned-message-b00000000003000"]
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        screenshot("uikit-pins")
        app.buttons["go-to-message-b00000000003000"].tap()
        XCTAssertTrue(card.waitForNonExistence(timeout: 10), "Go to message closes Pins")
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
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
        app.buttons["thread-back"].tap()
        XCTAssertTrue(thread.waitForNonExistence(timeout: 10))
        let link = app.buttons["view-thread-b00000000003000"]
        XCTAssertTrue(link.waitForExistence(timeout: 10), "The root shows its thread link")
        XCTAssertEqual(link.value as? String, "1 reply")
        screenshot("uikit-thread-link")
        link.tap()
        XCTAssertTrue(thread.waitForExistence(timeout: 10))
        XCTAssertTrue(reply.waitForExistence(timeout: 10))
    }

    /// Someone typing shows above the composer and clears when they stop.
    func testTypingIndicator() {
        let app = launch()
        XCTAssertTrue(row(Self.latest).waitForExistence(timeout: 30))
        control(["typing": ["channelId": "chan00000001"]])
        let typing = app.otherElements["typing-indicator"]
        XCTAssertTrue(typing.waitForExistence(timeout: 10))
        XCTAssertEqual(typing.label, "Maya is typing…")
        screenshot("uikit-typing")
        control(["typing": ["channelId": "chan00000001", "active": false]])
        XCTAssertTrue(typing.waitForNonExistence(timeout: 10), "Stopping clears it")
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

    private func configured(_ bundle: String, signedIn: Bool = true, cache: Bool = false) -> XCUIApplication {
        let app = XCUIApplication(bundleIdentifier: bundle)
        app.launchArguments += ["-caper.space-landing.owner0000001", ""]
        app.launchEnvironment["CAPER_TEST_MODE"] = "parity"
        app.launchEnvironment["CAPER_API_BASE_URL"] = "http://127.0.0.1:3001"
        if signedIn {
            app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-owner-token"
            app.launchEnvironment["CAPER_TEST_SPACE_ID"] = "space0000001"
        }
        if cache { app.launchEnvironment["CAPER_UIKIT_STARTUP_CACHE"] = "1" }
        return app
    }

    private func requireSwiftUI() throws {
        let app = XCUIApplication(bundleIdentifier: Self.swiftUI)
        // An app that is not installed reports .unknown before any launch.
        if app.state == .unknown { throw XCTSkip("Install the SwiftUI iOS app (chat.caper.ios) on this simulator to capture references.") }
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
