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

    private func launch(signedIn: Bool = true, cache: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
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
        let older = row("00000000-0000-4000-9000-000000002920")
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
        let design = app.cells["channel-design"]
        XCTAssertTrue(design.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["selected-space-name"].exists)
        XCTAssertEqual(app.cells["channel-general"].value as? String, "Selected")
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
