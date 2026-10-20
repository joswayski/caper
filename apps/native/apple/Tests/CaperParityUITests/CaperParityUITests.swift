import Foundation
import XCTest
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
final class CaperParityUITests: XCTestCase {
    private var launchedApp: XCUIApplication?

    private static func configuredApp(fixture: String? = nil, signedIn: Bool = true) -> XCUIApplication {
        let app = XCUIApplication()
        // Audio preferences persist in the app's own defaults, and the audio
        // tests move them (e.g. speaker volume to ~150%, voice processing to
        // 100%). A persistent runner keeps that domain between runs, so start
        // every launch from the app's defaults through the argument domain,
        // which outranks stored values and is never written back.
        app.launchArguments += [
            "-caper.voice.outputGain", "100",
            "-caper.voice.inputGain", "100",
            "-caper.voice.processingStrength", "25",
        ]
        #if os(macOS)
        // A persistent runner can restore closed or Settings-only windows from
        // an earlier launch. Ignore AppKit's saved window state for this test
        // process without deleting user preferences or changing normal launches.
        app.launchArguments += ["-ApplePersistenceIgnoreState", "YES"]
        #endif
        app.launchEnvironment["CAPER_TEST_MODE"] = "parity"
        app.launchEnvironment["CAPER_API_BASE_URL"] = "http://127.0.0.1:3001"
        if signedIn {
            app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-owner-token"
            app.launchEnvironment["CAPER_TEST_SPACE_ID"] = "space0000001"
        }
        if let fixture { app.launchEnvironment["CAPER_UI_FIXTURE"] = fixture }
        return app
    }

    private func launch(fixture: String? = nil, signedIn: Bool = true) -> XCUIApplication {
        let app = Self.configuredApp(fixture: fixture, signedIn: signedIn)
        app.launch()
        launchedApp = app
        return app
    }

    func testInvitePeopleOpensInviteOnlyInsteadOfSpaceManagement() {
        let app = launch()
        let spaceMenu = app.descendants(matching: .any)["selected-space-name"]
        if !spaceMenu.isHittable {
            let navigation = app.buttons["Back to Browse"]
            if navigation.waitForExistence(timeout: 10) { navigation.tap() }
        }
        XCTAssertTrue(spaceMenu.waitForExistence(timeout: 10))
        spaceMenu.tap()
        let invite = app.descendants(matching: .any)["Invite people…"].firstMatch
        XCTAssertTrue(invite.waitForExistence(timeout: 10))
        invite.tap()
        XCTAssertTrue(app.staticTexts["Invite by username"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.textFields["Exact username"].exists)
        XCTAssertFalse(app.textFields["Space name"].exists)
        XCTAssertFalse(app.buttons["Save name"].exists)
        XCTAssertFalse(app.buttons["Delete space"].exists)
        XCTAssertFalse(app.descendants(matching: .any)["space-members-heading"].exists)
        let copy = app.buttons["default-invite-link-copy"]
        XCTAssertTrue(copy.waitForExistence(timeout: 10))
        XCTAssertFalse(app.descendants(matching: .any)["invite-link-channel"].exists)
        XCTAssertFalse(app.staticTexts["Opens in"].exists)
        XCTAssertFalse(app.staticTexts["Default channel"].exists)
        let options = app.descendants(matching: .any)["default-invite-link-options"].firstMatch
        XCTAssertTrue(options.exists)
        XCTAssertEqual(options.label, "More invite options")
        XCTAssertGreaterThanOrEqual(options.frame.minX, copy.frame.maxX)
        options.tap()
        XCTAssertTrue(app.descendants(matching: .any)["Regenerate link"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any)["Custom links"].firstMatch.exists)
        XCTAssertTrue(app.descendants(matching: .any)["Deactivate link"].firstMatch.exists)
        app.descendants(matching: .any)["Regenerate link"].firstMatch.tap()
        let regenerateAlert = app.alerts["Regenerate link?"]
        XCTAssertTrue(regenerateAlert.waitForExistence(timeout: 5))
        XCTAssertTrue(regenerateAlert.staticTexts["The previous URL stops admitting new members. The new link resets to 30 days and 100 joins."].exists)
        regenerateAlert.buttons["Cancel"].tap()
        XCTAssertTrue(copy.exists)
        XCTAssertFalse(regenerateAlert.exists)
        options.tap()
        app.descendants(matching: .any)["Deactivate link"].firstMatch.tap()
        let deactivateAlert = app.alerts["Deactivate link?"]
        XCTAssertTrue(deactivateAlert.waitForExistence(timeout: 5))
        deactivateAlert.buttons["Cancel"].tap()
        XCTAssertTrue(copy.exists)
        XCTAssertFalse(app.buttons["default-invite-link-create"].exists)
    }

    #if os(iOS)
    /// On a freshly booted simulator the automation session can report an
    /// empty accessibility tree for about a minute while the app is already on
    /// screen, failing whichever test runs first. Warm it up once, before any
    /// test's own deadlines start. Nothing is asserted or sent here.
    nonisolated override class func setUp() {
        super.setUp()
        MainActor.assumeIsolated {
            let app = configuredApp()
            app.launch()
            _ = app.descendants(matching: .any)["message-composer"].waitForExistence(timeout: 120)
            app.terminate()
        }
    }
    #endif

    override func tearDown() {
        // A test that already terminated the app has no screen to capture.
        if (testRun?.failureCount ?? 0) > 0, let app = launchedApp, app.state != .notRunning {
            let hierarchy = XCTAttachment(string: app.debugDescription)
            hierarchy.name = "accessibility-hierarchy-\(name)"
            hierarchy.lifetime = .keepAlways
            add(hierarchy)
            #if os(macOS)
            let screenshot = XCTAttachment(screenshot: app.windows.firstMatch.exists ? app.windows.firstMatch.screenshot() : app.screenshot())
            #else
            let screenshot = XCTAttachment(screenshot: app.screenshot())
            #endif
            screenshot.name = "failure-\(name)"
            screenshot.lifetime = .keepAlways
            add(screenshot)
        }
        launchedApp = nil
        super.tearDown()
    }

    /// Taps `field` until it holds keyboard focus, then types. Moving focus
    /// between fields while the iOS keyboard animates in can drop the first tap,
    /// and typing then fails with "Neither element nor any descendant has
    /// keyboard focus".
    private func hasKeyboardFocus(_ field: XCUIElement, timeout: TimeInterval = 2) -> Bool {
        let focused = NSPredicate(format: "hasKeyboardFocus == true")
        let expectation = XCTNSPredicateExpectation(predicate: focused, object: field)
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    /// iPhone: Return in the field saves, because the error line can push the
    /// button under the keyboard. macOS clicks the button.
    private func save(_ button: XCUIElement, from field: XCUIElement) {
        #if os(iOS)
        // A slow simulator sometimes won't refocus the field after a rejected
        // save; with no keyboard up, tap the button instead.
        if focus(field) {
            field.typeText("\n")
        } else if button.isHittable {
            button.tap()
        } else {
            XCTFail("\(field.label) never took keyboard focus")
        }
        #else
        button.tap()
        #endif
    }

    private func focus(_ field: XCUIElement) -> Bool {
        for _ in 0..<3 {
            field.tap()
            if hasKeyboardFocus(field) { return true }
        }
        return false
    }

    private func type(_ text: String, into field: XCUIElement) {
        #if os(macOS)
        field.tap()
        field.typeText(text)
        #else
        if focus(field) {
            field.typeText(text)
        } else {
            XCTFail("\(field.label) never took keyboard focus")
        }
        #endif
    }

    #if os(macOS)
    /// Apple silicon, including the x86_64 build under Rosetta on the
    /// self-hosted Mac, whose VM screen fits only the medium layout. Only a
    /// real Intel Mac gets the 1,440×900 desktop reference window.
    private static let runsOnAppleSilicon: Bool = {
        #if arch(arm64)
        return true
        #else
        var translated: Int32 = 0
        var size = MemoryLayout<Int32>.size
        return sysctlbyname("sysctl.proc_translated", &translated, &size, nil, 0) == 0 && translated == 1
        #endif
    }()
    #endif

    private func capture(_ name: String, app: XCUIApplication) {
        #if os(macOS)
        // Screenshot the frontmost window (the fixed-size Settings window when
        // it is open), but hold the layout to the main app window: Settings is
        // 500×380 by design and is not the desktop parity layout.
        let window = app.windows.firstMatch
        XCTAssertTrue(window.waitForExistence(timeout: 2))
        let main = app.windows.matching(NSPredicate(format: "identifier != %@", "com_apple_SwiftUI_Settings_window")).firstMatch
        XCTAssertTrue(main.exists, "The main app window must stay open behind any capture")
        let size = main.frame.size
        let layout: String
        if Self.runsOnAppleSilicon {
            let desktop = size.width >= 1_400
            layout = desktop ? "desktop" : "medium"
            XCTAssertGreaterThanOrEqual(size.width, desktop ? 1_400 : 980, "ARM medium capture requires at least 980 points of width")
            XCTAssertGreaterThanOrEqual(size.height, desktop ? 880 : 640, "ARM capture is too short for its declared layout")
        } else {
            layout = "desktop"
            XCTAssertGreaterThanOrEqual(size.width, 1_400, "Intel desktop parity capture requires a 1,400-point-wide app window")
            XCTAssertGreaterThanOrEqual(size.height, 880, "Intel desktop parity capture requires an app window close to the 1,440×900 reference")
        }
        let captureName = "\(layout)-\(name)"
        let dimensions = XCTAttachment(string: "layout=\(layout) width=\(Int(size.width)) height=\(Int(size.height))")
        dimensions.name = "\(captureName)-window-size"
        dimensions.lifetime = .keepAlways
        add(dimensions)
        let attachment = XCTAttachment(screenshot: window.screenshot())
        #else
        let attachment = XCTAttachment(screenshot: app.screenshot())
        #endif
        #if os(macOS)
        attachment.name = captureName
        #else
        attachment.name = name
        #endif
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testWhatsNewFixtureShowsHighlightsAndDismisses() {
        let app = launch(fixture: "whats-new")
        assertStaticText("TEST FIXTURE — Message editing", in: app, timeout: 15)
        capture("whats-new-fixture", app: app)
        let close = app.buttons["Close What’s new"]
        XCTAssertTrue(close.waitForExistence(timeout: 3))
        close.tap()
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: close)
        XCTAssertEqual(XCTWaiter().wait(for: [dismissed], timeout: 3), .completed)
    }

    /// Whether a voice card sits above the Channels heading, where only voice without a row goes.
    private func voiceAboveChannels(in app: XCUIApplication) -> Bool {
        let channels = app.buttons["channels-toggle"].frame
        let cards = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier == 'voice-call-card' OR identifier BEGINSWITH 'live-room-chan'"))
        return cards.allElementsBoundByIndex.contains { $0.frame.maxY <= channels.minY }
    }

    private func assertElement(_ identifier: String, label: String, in app: XCUIApplication, timeout: TimeInterval = 10) {
        let element = app.descendants(matching: .any)[identifier]
        XCTAssertTrue(element.waitForExistence(timeout: timeout), "Missing accessibility identifier \(identifier)")
        #if os(macOS)
        // Static text usually reports its text as its value; some (a popover's header) as its label.
        if element.elementType == .staticText {
            XCTAssertEqual((element.value as? String).flatMap { $0.isEmpty ? nil : $0 } ?? element.label, label)
        } else { XCTAssertEqual(element.label, label) }
        #else
        XCTAssertEqual(element.label, label)
        #endif
    }

    private func staticTexts(_ text: String, in root: XCUIElement) -> XCUIElementQuery {
        #if os(macOS)
        root.staticTexts.matching(NSPredicate(format: "value == %@", text))
        #else
        root.staticTexts.matching(NSPredicate(format: "label == %@", text))
        #endif
    }

    /// Closes the call's participant list: Escape for the Mac's popover, Close
    /// for the phone's sheet, as a person would.
    private func closeVoiceList(in app: XCUIApplication) {
        let list = app.descendants(matching: .any)["voice-participants"]
        #if os(macOS)
        app.typeKey(.escape, modifierFlags: [])
        #else
        app.buttons["voice-list-close"].tap()
        #endif
        wait(for: [expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: list)], timeout: 3)
    }

    #if os(iOS)
    /// Phones keep Pins and Members in the menu the conversation's name opens.
    /// Members carries the member count ("Members · 3"), so match its start.
    private func chooseFromTitleMenu(_ item: String, in app: XCUIApplication) {
        let title = app.descendants(matching: .any)["channel-title-menu"]
        XCTAssertTrue(title.waitForExistence(timeout: 3), "The phone header's title opens the conversation menu")
        title.tap()
        let choice = app.buttons.matching(NSPredicate(format: "label == %@ OR label BEGINSWITH %@", item, "\(item) ·")).firstMatch
        XCTAssertTrue(choice.waitForExistence(timeout: 3), "The title menu offers \(item)")
        choice.tap()
    }
    #endif

    private struct MissingElement: Error, CustomStringConvertible { let description: String }

    /// Waits for `element` and throws if it never appears. Tapping, typing into
    /// or reading a missing element records an interrupting failure; an async
    /// test cannot be unwound by it, so XCTest starts tearDown and the next
    /// test while the abandoned test keeps driving the app, hanging the suite.
    /// Throwing ends the test normally and lets its cleanup run.
    @discardableResult
    private func require(_ element: XCUIElement, timeout: TimeInterval, _ message: String,
                         line: UInt = #line) throws -> XCUIElement {
        guard element.waitForExistence(timeout: timeout) else {
            throw MissingElement(description: "\(message) (line \(line))")
        }
        return element
    }

    #if os(iOS)
    /// Holds a message the way a person does: on its header text. The row's
    /// centre can fall on a reaction chip, and since iOS 26 holding a chip
    /// activates that button instead of the row's long press.
    private func hold(_ row: XCUIElement) {
        row.staticTexts.firstMatch.press(forDuration: 0.8)
    }

    /// Since iOS 26 this runner may not read a pasteboard item the app wrote
    /// (PBErrorDomain code 13, "Operation not authorized"), so the app reads
    /// its own pasteboard back into a parity-only accessibility element.
    private func assertCopied(_ expected: String, in app: XCUIApplication,
                              file: StaticString = #filePath, line: UInt = #line) {
        let pasteboard = app.descendants(matching: .any)["parity-pasteboard"]
        let copied = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", expected), object: pasteboard)
        let result = XCTWaiter.wait(for: [copied], timeout: 5)
        XCTAssertEqual(result, .completed, "Pasteboard holds \(pasteboard.exists ? pasteboard.label : "nothing"), expected \(expected)",
                       file: file, line: line)
    }
    #else
    /// Hovers visible text, not a reaction chip whose "who reacted" tooltip
    /// can cover Add reaction. A grouped row's first static text is its
    /// hover-only timestamp, which can have a zero frame before hovering.
    private func hoverMessage(_ row: XCUIElement) throws {
        let text = try XCTUnwrap(row.staticTexts.allElementsBoundByIndex.first(where: { $0.isHittable }),
                                "The message has no visible text to hover")
        text.hover()
    }
    #endif

    private func openReactionPicker(for messageID: String, in app: XCUIApplication) throws {
        #if os(iOS)
        let row = try require(app.descendants(matching: .any)["message-row-\(messageID)"], timeout: 10,
                              "Missing message-row-\(messageID)")
        hold(row)
        let add = try require(app.buttons["message-action-add-reaction"], timeout: 5,
                              "Holding \(messageID) did not offer Add reaction")
        #else
        let row = try require(app.descendants(matching: .any)["message-row-\(messageID)"], timeout: 10,
                              "Missing message-row-\(messageID)")
        // Just after launch the timeline can still move the row out from
        // under a pointer that stays put (CI saw the controls appear and then
        // vanish), so hover again until they stay up.
        let add = app.buttons["add-reaction-\(messageID)"]
        var shown = false
        for _ in 0..<4 where !shown {
            try hoverMessage(row)
            shown = add.waitForExistence(timeout: 3) && add.isHittable
        }
        XCTAssertTrue(shown, "Hovering \(messageID) did not reveal Add reaction")
        #endif
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: add)
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 10), .completed)
        #if os(macOS)
        // Keep the action's declared size and trailing placement, and verify
        // moving the pointer into its overlay does not hide the click target.
        XCTAssertEqual(add.frame.width, 24, accuracy: 1)
        XCTAssertEqual(add.frame.height, 24, accuracy: 1)
        XCTAssertGreaterThan(add.frame.minX, row.frame.midX, "Message actions must stay at the trailing edge")
        XCTAssertTrue(add.isHittable)
        add.hover()
        XCTAssertTrue(add.isHittable, "Moving from the row onto Add reaction must not hide its click target")
        #endif
        add.tap()
    }

    func testColonEmojiSuggestionsPreserveDraftAndFocusWithoutSending() throws {
        let app = launch()
        let composer = try require(app.descendants(matching: .any)["message-composer"], timeout: 30,
                                   "Missing message composer")
        XCTAssertTrue(focus(composer))
        composer.typeText("Before :tomato")
        let tomato = try require(app.buttons["emoji-suggestion-1f345"], timeout: 5,
                                 "Typing a colon query must offer tomato")
        capture("emoji-composer-suggestions", app: app)
        tomato.tap()
        let inserted = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Before 🍅"), object: composer)
        XCTAssertEqual(XCTWaiter.wait(for: [inserted], timeout: 3), .completed,
                       "Selecting emoji must retain the draft without sending")
        XCTAssertTrue(hasKeyboardFocus(composer))
        composer.typeText(" after")
        XCTAssertEqual(composer.value as? String, "Before 🍅 after", "Insertion must preserve the caret")
        XCTAssertFalse(app.buttons["emoji-suggestion-1f345"].exists)
    }

    #if os(macOS)
    func testComposerSuggestionsDoNotResizeOrMoveChannelAndThreadHistory() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        try await Self.fixtureControl(["incomingReply": ["rootId": "chan00000001m01", "text": "Stationary thread reply"]])
        let app = launch()
        let root = try require(app.descendants(matching: .any)["message-row-chan00000001m01"], timeout: 30,
                               "Missing fixture thread root")
        // Beside Members the thread is 232 pt and "Before :tomato" wraps, growing
        // the composer for reasons unrelated to suggestions.
        try hideMembers(in: app)
        for inThread in [false, true] {
            if inThread { try require(root.buttons["View thread"], timeout: 5, "Missing thread link").click() }
            let timeline = try require(app.scrollViews[inThread ? "thread-timeline" : "chat-timeline"], timeout: 5,
                                       "Missing conversation viewport")
            let composer = try require(app.textViews.matching(NSPredicate(format: "label BEGINSWITH %@", inThread ? "Reply to thread" : "Message #")).firstMatch,
                                       timeout: 5, "Missing composer")
            XCTAssertTrue(focus(composer))
            composer.typeText("Before ")
            let viewport = timeline.frame
            let editor = composer.frame
            let lastText = inThread ? "TEST FIXTURE — Stationary thread reply"
                : "Agreed, @fixture_owner. Let’s check the narrow layout and the management dialogs too."
            let lastMessage = try require(staticTexts(lastText, in: timeline).firstMatch, timeout: 5, "Missing latest message")
            let messageY = lastMessage.frame.minY
            let scope = inThread ? "thread" : "channel"
            func assertStationary() {
                XCTAssertEqual(timeline.frame.minY, viewport.minY, accuracy: 1)
                XCTAssertEqual(timeline.frame.height, viewport.height, accuracy: 1, "Suggestions must overlay, not shrink, history")
                XCTAssertEqual(lastMessage.frame.minY, messageY, accuracy: 1, "Suggestions must not scroll messages")
                XCTAssertEqual(composer.frame.minY, editor.minY, accuracy: 1)
                XCTAssertEqual(composer.frame.height, editor.height, accuracy: 1)
            }
            capture("\(scope)-autocomplete-closed", app: app)
            composer.typeText(":")
            let thumbsUp = try require(app.buttons["emoji-suggestion-1f44d"], timeout: 5, "Missing default emoji choices")
            XCTAssertLessThan(thumbsUp.frame.maxY, composer.frame.minY, "Suggestions belong above the composer")
            assertStationary()
            capture("\(scope)-autocomplete-defaults", app: app)
            composer.typeText("tomato")
            let tomato = try require(app.buttons["emoji-suggestion-1f345"], timeout: 5, "Missing narrowed emoji choice")
            assertStationary()
            capture("\(scope)-autocomplete-narrowed", app: app)
            // A real mouse click at the row's screen point, not an accessibility press: it must reach the
            // overlaid popup rather than the history beneath it. (`isHittable` uses the accessibility hit
            // test, which does not resolve SwiftUI overlay content here even though clicks land.)
            tomato.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click()
            XCTAssertEqual(composer.value as? String, "Before 🍅", "The overlaid popup must accept clicks")
            XCTAssertTrue(hasKeyboardFocus(composer))
            assertStationary()
            composer.typeText(" @")
            _ = try require(app.buttons["mention-suggestion-everyone"], timeout: 5, "Missing mention choices")
            assertStationary()
            capture("\(scope)-autocomplete-mentions", app: app)
            composer.typeKey(.escape, modifierFlags: [])
            XCTAssertFalse(app.buttons["mention-suggestion-everyone"].exists)
            XCTAssertEqual(composer.value as? String, "Before 🍅 @", "Dismissing suggestions must retain the draft")
            assertStationary()
        }
    }
    #endif

    func testAtMentionSuggestionsInsertUsernameWithoutSending() throws {
        let app = launch()
        let composer = try require(app.descendants(matching: .any)["message-composer"], timeout: 30,
                                   "Missing message composer")
        XCTAssertTrue(focus(composer))
        composer.typeText("Hi @")
        _ = try require(app.buttons["mention-suggestion-everyone"], timeout: 5,
                        "A space channel must offer @everyone")
        XCTAssertTrue(app.buttons["mention-suggestion-here"].exists)
        XCTAssertTrue(app.buttons["mention-suggestion-alex"].exists, "Space members are suggested")
        XCTAssertFalse(app.buttons["mention-suggestion-fixture_owner"].exists, "The signed-in account is never suggested")
        composer.typeText("ma")
        let maya = try require(app.buttons["mention-suggestion-maya"], timeout: 5,
                               "Typing @ma must offer the space member maya")
        capture("mention-composer-suggestions", app: app)
        maya.tap()
        let inserted = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Hi @maya "), object: composer)
        XCTAssertEqual(XCTWaiter.wait(for: [inserted], timeout: 3), .completed,
                       "Selecting a member must insert @username and a space without sending")
        XCTAssertTrue(hasKeyboardFocus(composer))
        composer.typeText("there")
        XCTAssertEqual(composer.value as? String, "Hi @maya there", "Insertion must leave the caret after the space")
        XCTAssertFalse(app.buttons["mention-suggestion-maya"].exists)
    }

    func testOwnMentionPillOpensCardWithoutMessageButton() throws {
        let app = launch()
        // The fixture's seeded message from Alex mentions @fixture_owner (you).
        let row = try require(app.descendants(matching: .any)["message-row-chan00000001m04"], timeout: 30,
                              "Missing the seeded message that mentions you")
        #if os(macOS)
        // macOS 15 exposes a message as one static text without its link runs.
        let linksExposed = ProcessInfo.processInfo.isOperatingSystemAtLeast(
            OperatingSystemVersion(majorVersion: 26, minorVersion: 0, patchVersion: 0))
        #else
        let linksExposed = true
        #endif
        if linksExposed {
            let pill = try require(row.links.matching(NSPredicate(format: "label CONTAINS %@", "fixture_owner")).firstMatch,
                                   timeout: 5, "The @fixture_owner pill must be a link")
            pill.tap()
        } else {
            #if os(macOS)
            // Click the pill's characters within the single-line message text,
            // which needs the timeline's width without Members beside it.
            try hideMembers(in: app)
            let mention = "@fixture_owner"
            let text = try require(row.staticTexts.matching(NSPredicate(format: "value CONTAINS %@", mention)).firstMatch,
                                   timeout: 5, "The message must show the @fixture_owner pill")
            XCTAssertLessThan(text.frame.height, 25, "The click below assumes the message fits on one line")
            let value = text.value as? String ?? ""
            let range = try XCTUnwrap(value.range(of: mention), "The message text must contain the pill")
            let middle = value.distance(from: value.startIndex, to: range.lowerBound) + mention.count / 2
            text.coordinate(withNormalizedOffset: CGVector(dx: Double(middle) / Double(value.count), dy: 0.5)).click()
            #endif
        }
        _ = try require(app.descendants(matching: .any)["mention-card"], timeout: 5, "Tapping a pill must open the mention card")
        XCTAssertTrue(app.descendants(matching: .any)["mention-card-you"].waitForExistence(timeout: 2), "Your own card says You")
        XCTAssertFalse(app.buttons["mention-card-message"].exists, "Your own card has no Message button")
        capture("mention-card-self", app: app)
        let edit = try require(app.buttons["mention-card-edit-profile"], timeout: 5,
                               "Your own profile card must offer Edit profile")
        edit.tap()
        XCTAssertTrue(app.textFields["Username"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.textFields["Display name"].exists)
        XCTAssertTrue(app.descendants(matching: .any)["change-profile-picture"].exists)
        capture("mention-card-edit-profile", app: app)
        app.buttons["Close profile"].tap()
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.buttons["profile-save"])
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 5), .completed)
        XCTAssertTrue(row.exists, "Closing profile settings must return to the same conversation")
    }

    func testProfileBlockMenuAndBackdropDismissalPreserveDraft() throws {
        let app = launch()
        let author = try require(app.buttons.matching(NSPredicate(format: "label == %@", "Open profile for Maya")).firstMatch, timeout: 30,
                                 "The seeded author's profile must be available")
        author.tap()
        let card = app.descendants(matching: .any)["mention-card"]
        let composer = try require(app.descendants(matching: .any)["mention-card-composer"], timeout: 5,
                                   "The profile must have its own composer")
        XCTAssertFalse(app.buttons["mention-card-edit-profile"].exists, "Other people's cards must not offer your profile settings")
        XCTAssertFalse(app.descendants(matching: .any)["mention-card-block"].exists,
                       "Block must stay inside the secondary menu")
        composer.tap()
        composer.typeText("Keep this profile draft")
        app.descendants(matching: .any)["mention-card-more"].tap()
        let block = try require(app.descendants(matching: .any)["mention-card-block"], timeout: 3,
                                "More profile actions must offer Block")
        block.tap()
        #if os(macOS)
        // The alert's Cancel is mirrored in the Touch Bar; take the one in the window.
        let cancel = try require(app.windows.firstMatch.buttons["Cancel"], timeout: 5, "Block still requires confirmation")
        #else
        let cancel = try require(app.buttons["Cancel"], timeout: 5, "Block still requires confirmation")
        #endif
        cancel.tap()
        author.tap()
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, "Keep this profile draft")
        #if os(macOS)
        // Members opens by default on wide windows (#597); in a 1440-pt window the
        // top-right corner is a member row, which opens that member's card instead.
        app.scrollViews["chat-timeline"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.95)).click()
        #else
        app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: 0.1)).tap()
        #endif
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: card)
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 3), .completed,
                       "A backdrop tap closes the profile even with a draft")
        author.tap()
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, "Keep this profile draft")
        app.buttons["mention-card-close"].tap()
    }

    #if os(iOS)
    func testIPhoneMessageActionsAcceptTapsInTrailingWhitespace() throws {
        let app = launch()
        let row = try require(app.descendants(matching: .any)["message-row-chan00000001m01"], timeout: 30,
                              "Missing fixture message")
        let text = "TEST FIXTURE — local sample data, not a live conversation."
        for label in ["Copy text", "Copy message ID", "Edit message", "Reply"] {
            hold(row)
            let sheet = try require(app.descendants(matching: .any)["message-actions-sheet"], timeout: 5,
                                    "Missing message actions")
            let action = try require(sheet.buttons[label], timeout: 5, "Missing \(label) action")
            // Use the sheet's right edge, not the button's reported width:
            // a text-sized button must not silently move the tap onto its label.
            sheet.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(
                dx: sheet.frame.width - 36, dy: action.frame.midY - sheet.frame.minY
            )).tap()
            switch label {
            case "Copy text":
                assertCopied(text, in: app)
            case "Copy message ID":
                assertCopied("chan00000001m01", in: app)
            case "Edit message":
                let editor = try require(app.textViews["message-edit-text"], timeout: 5,
                                         "Tapping the row's blank right side must open the editor")
                XCTAssertEqual(editor.value as? String, text)
                try require(app.buttons["Cancel"], timeout: 5, "Missing editor cancel action").tap()
            default:
                try require(app.buttons["Cancel reply"], timeout: 5,
                            "Tapping the row's blank right side must start a reply")
            }
            let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: sheet)
            XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed,
                           "\(label) must respond to a tap in the row's trailing whitespace")
        }
    }

    func testIPhoneComposerUsesContentHeightAndShrinksAfterEditing() throws {
        let app = launch()
        let composer = try require(app.descendants(matching: .any)["message-composer"], timeout: 30,
                                   "Missing message composer")
        let send = app.buttons["send-message-button"]
        let timeline = try require(app.descendants(matching: .any)["chat-timeline"], timeout: 5,
                                   "Missing message timeline")
        let restingBottom = send.frame.maxY
        XCTAssertEqual(composer.frame.minY - timeline.frame.maxY, 12, accuracy: 2,
                       "The input must keep breathing room above the scrolling timeline")
        XCTAssertLessThan(restingBottom, app.frame.maxY, "The home-indicator safe area must remain when the keyboard is closed")
        capture("composer-empty-keyboard-closed", app: app)
        // The empty UITextView can report hasKeyboardFocus=false with a visible
        // caret and keyboard. Repeated taps then open AutoFill; verify focus by
        // typing into this exact composer below instead.
        composer.tap()
        // Hosted simulators can take several seconds to raise the keyboard.
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2)
        // The text view already measured 42 points before the fix. Its bottom
        // must align with Send too, or its background can still occupy 174.
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        XCTAssertEqual(composer.frame.minY - timeline.frame.maxY, 12, accuracy: 2)
        XCTAssertEqual(try keyboardTop(in: app) - send.frame.maxY, 16, accuracy: 2,
                       "The composer must keep 16 points of breathing room above the keyboard")
        capture("composer-empty-keyboard", app: app)

        let short = "Compact draft"
        composer.typeText(short)
        // Typing can finish before the accessibility snapshot has the full value.
        let inserted = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", short), object: composer)
        XCTAssertEqual(XCTWaiter.wait(for: [inserted], timeout: 3), .completed,
                       "The typed draft must reach the composer's accessibility value")
        XCTAssertEqual(composer.value as? String, short,
                       "The tapped composer must receive keyboard input")
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2)
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        XCTAssertEqual(try keyboardTop(in: app) - send.frame.maxY, 16, accuracy: 2)
        capture("composer-single-line-keyboard", app: app)

        timeline.swipeDown()
        let scrollDismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.keyboards.firstMatch)
        XCTAssertEqual(XCTWaiter.wait(for: [scrollDismissed], timeout: 5), .completed,
                       "Dragging message history must dismiss the keyboard")
        XCTAssertEqual(composer.value as? String, short, "Scrolling must keep the unsent draft")
        XCTAssertEqual(send.frame.maxY, restingBottom, accuracy: 2)
        XCTAssertTrue(focus(composer))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 3))

        let multiline = "\nSecond line\nThird line"
        composer.typeText(multiline)
        XCTAssertEqual(composer.value as? String, short + multiline)
        XCTAssertGreaterThan(composer.frame.height, 62, "The fix must not freeze the composer at one line")
        XCTAssertLessThan(composer.frame.height, 174, "A three-line draft must not jump straight to the cap")
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        XCTAssertEqual(composer.frame.minY - timeline.frame.maxY, 12, accuracy: 2)
        XCTAssertEqual(try keyboardTop(in: app) - send.frame.maxY, 16, accuracy: 2)
        capture("composer-multiline-keyboard", app: app)

        let overflow = String(repeating: "\nmore", count: 16)
        composer.typeText(overflow)
        XCTAssertEqual(composer.value as? String, short + multiline + overflow,
                       "Scrolling must retain lines beyond the visible height")
        XCTAssertEqual(composer.frame.height, 174, accuracy: 2)
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        XCTAssertEqual(try keyboardTop(in: app) - send.frame.maxY, 16, accuracy: 2)
        XCTAssertTrue(send.isHittable, "Send must remain available above the keyboard at the height cap")
        capture("composer-capped-keyboard", app: app)

        deleteBack(to: short, in: composer)
        XCTAssertEqual(composer.value as? String, short)
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2, "Removing extra lines must shrink the composer")
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        deleteBack(to: "", in: composer)
        XCTAssertEqual(composer.value as? String, "")
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2)
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)

        app.buttons["Back to Browse"].tap()
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.keyboards.firstMatch)
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed)
        try require(app.buttons["Close navigation"], timeout: 5, "Missing conversation return target").tap()
        XCTAssertFalse(app.keyboards.firstMatch.exists, "Returning from Browse must not reopen the keyboard")
        XCTAssertEqual(send.frame.maxY, restingBottom, accuracy: 2, "Dismissing the keyboard must restore the safe-area layout")
        capture("composer-keyboard-dismissed", app: app)
        XCTAssertTrue(focus(composer))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 3))
        XCTAssertEqual(try keyboardTop(in: app) - send.frame.maxY, 16, accuracy: 2)
        capture("composer-keyboard-reopened", app: app)
    }

    /// Measure UIKit's full keyboard (including predictions), independently of
    /// the controls under test. XCUITest's keyboard frame is only the keys.
    private func keyboardTop(in app: XCUIApplication) throws -> CGFloat {
        // Both composers can remain in the tree under a thread cover. They
        // observe the same system notification and report the same boundary.
        let probe = try require(app.descendants(matching: .any).matching(identifier: "parity-keyboard-top").firstMatch, timeout: 3,
                                "Missing system keyboard measurement")
        let measured = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value != %@", ""), object: probe)
        XCTAssertEqual(XCTWaiter.wait(for: [measured], timeout: 3), .completed)
        let top = try XCTUnwrap(Double(probe.value as? String ?? ""), "UIKit did not report the keyboard frame")
        XCTAssertGreaterThan(top, Double(app.frame.minY))
        XCTAssertLessThan(top, Double(app.frame.maxY), "The measured keyboard must be onscreen")
        return CGFloat(top)
    }

    /// Deletes from the end until the field holds `target`. On the simulator a
    /// long burst of Delete keys loses some while the composer shrinks a line
    /// at a time (in CI, 36 of 103 characters were left), so this deletes in
    /// small batches and recounts.
    private func deleteBack(to target: String, in field: XCUIElement) {
        for _ in 0..<30 {
            let value = field.value as? String ?? ""
            guard value.count > target.count, value.hasPrefix(target) else { return }
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: min(10, value.count - target.count)))
        }
    }

    /// The first match a person could touch. On iPhone a thread covers the
    /// channel, but the channel's rows and composer stay in the accessibility
    /// tree behind it, so a plain query can return those hidden copies.
    private func visible(_ query: XCUIElementQuery, timeout: TimeInterval = 5) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if let match = query.allElementsBoundByIndex.first(where: { $0.exists && $0.isHittable }) { return match }
            Thread.sleep(forTimeInterval: 0.2)
        } while Date() < deadline
        return nil
    }

    func testIPhoneThreadComposerDoesNotFillItsMaximumHeight() throws {
        let app = launch()
        let row = try require(app.descendants(matching: .any)["message-row-chan00000001m01"], timeout: 30,
                              "Missing fixture message")
        XCTAssertFalse(app.buttons["Reply in thread"].exists, "Unthreaded messages must not show a reply link")
        hold(row)
        // Each row also has its own "Reply in thread" link, so look in the sheet.
        let rowActions = try require(app.descendants(matching: .any)["message-actions-sheet"], timeout: 5,
                                     "Missing message actions")
        try require(rowActions.buttons["Reply in thread"], timeout: 5, "Missing thread action").tap()
        let back = try require(app.buttons["Back to channel"], timeout: 5, "Thread did not open")
        let title = try require(app.staticTexts["Thread"], timeout: 5, "Missing thread title")
        XCTAssertLessThanOrEqual(back.frame.maxX, title.frame.minX, "Back arrow must lead the thread title")
        XCTAssertGreaterThanOrEqual(back.frame.width, 44)
        try require(app.staticTexts["No replies yet. Start the thread."], timeout: 5, "Missing empty thread state")
        XCTAssertFalse(app.staticTexts["0 replies"].exists, "Empty threads must not repeat the zero reply count")
        // By label: the channel's composer is still in the tree too.
        let composer = try require(app.textViews.matching(NSPredicate(format: "label BEGINSWITH %@", "Reply to thread")).firstMatch,
                                   timeout: 5, "Missing thread composer")
        XCTAssertTrue(focus(composer))
        composer.typeText("Short reply")
        let send = app.buttons["Send reply"]
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2)
        XCTAssertEqual(send.frame.midY, composer.frame.midY, accuracy: 2,
                       "Send must align with the single-line reply input")
        for label in ["Attach files", "Insert emoji"] {
            let button = try XCTUnwrap(visible(app.buttons.matching(NSPredicate(format: "label == %@", label))), "Missing \(label)")
            XCTAssertEqual(button.frame.midY, composer.frame.midY, accuracy: 2,
                           "Thread tools must align with the single-line reply input")
        }
        let broadcast = try require(app.switches["Also send to #general"], timeout: 5, "Missing named broadcast option")
        XCTAssertLessThanOrEqual(broadcast.frame.maxY, composer.frame.minY,
                                "The broadcast checkbox must stay above the input, away from Send")
        broadcast.tap()
        XCTAssertEqual(broadcast.value as? String, "1")
        broadcast.tap()
        XCTAssertEqual(broadcast.value as? String, "0")
        XCTAssertTrue(send.isHittable)
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 3))
        XCTAssertEqual(try keyboardTop(in: app) - send.frame.maxY, 16, accuracy: 2,
                       "Thread controls must keep the same 16-point keyboard spacing as the channel composer")
        capture("thread-composer-single-line-keyboard", app: app)

        let timeline = try XCTUnwrap(visible(app.scrollViews), "Missing the visible thread timeline")
        timeline.swipeDown()
        let scrollDismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.keyboards.firstMatch)
        XCTAssertEqual(XCTWaiter.wait(for: [scrollDismissed], timeout: 5), .completed,
                       "Dragging thread history must dismiss the keyboard, even without replies")
        XCTAssertEqual(composer.value as? String, "Short reply", "Scrolling must keep the reply draft")
        XCTAssertTrue(back.isHittable, "Vertical scrolling must not close the thread")
        timeline.swipeLeft()
        XCTAssertTrue(back.isHittable, "A left swipe must not close the thread")
        let start = timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.7))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 35, dy: 0)))
        XCTAssertTrue(back.isHittable, "A short right drag must not close the thread")
        // Start on selectable message text, not the empty part of the list:
        // the old gesture excluded every UITextView, including read-only bodies.
        let body = try XCTUnwrap(visible(app.staticTexts.matching(NSPredicate(
            format: "label CONTAINS %@", "local sample data, not a live conversation"))), "Missing root body")
        let textStart = body.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.5))
        textStart.press(forDuration: 0.05, thenDragTo: textStart.withOffset(CGVector(dx: 160, dy: 0)))
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: back)
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 5), .completed,
                       "A right swipe starting on message text must return to the channel")
        // Reopening the same root must retain the unsent draft.
        hold(row)
        let reopenActions = try require(app.descendants(matching: .any)["message-actions-sheet"], timeout: 5,
                                        "Missing root actions after returning to channel")
        try require(reopenActions.buttons["Reply in thread"], timeout: 5, "Missing reopen action").tap()
        try require(back, timeout: 5, "Thread did not reopen")
        XCTAssertEqual(composer.value as? String, "Short reply", "Swipe navigation must keep the reply draft")
        XCTAssertTrue(focus(composer))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 3))

        let threadRoot = try XCTUnwrap(visible(app.descendants(matching: .any).matching(identifier: "message-row-chan00000001m01")),
                                       "Missing the thread's root message")
        hold(threadRoot)
        let actions = try require(app.descendants(matching: .any)["message-actions-sheet"], timeout: 5,
                                  "Missing thread root actions")
        XCTAssertFalse(actions.buttons["Reply in thread"].exists, "The root is already open in its thread")
        XCTAssertTrue(actions.buttons["Copy text"].exists)
        actions.buttons["Copy text"].tap()
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: actions)
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed)

        send.tap()
        // "Also send to #general" can put a copy in the hidden channel too.
        _ = try XCTUnwrap(visible(app.staticTexts.matching(NSPredicate(format: "label == %@", "Short reply"))),
                          "Reply was not sent")
        try require(app.staticTexts["1 reply"], timeout: 5, "Populated threads must keep their reply count")
        XCTAssertFalse(app.staticTexts["No replies yet. Start the thread."].exists)
        // Hold the reply like the root above, on its header text. Since profile
        // cards (#483), holding its body text on CI opened the author's profile
        // card instead of the message's actions.
        let replyRow = try require(app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH %@", "message-row-"))
            .containing(NSPredicate(format: "label == %@", "Short reply")).firstMatch,
            timeout: 5, "Missing the reply's row")
        hold(replyRow)
        let replyActions = try require(app.descendants(matching: .any)["message-actions-sheet"], timeout: 5,
                                       "Missing thread reply actions")
        XCTAssertFalse(replyActions.buttons["Reply in thread"].exists, "Replies cannot start nested threads")
        XCTAssertTrue(replyActions.buttons["Copy text"].exists)
        capture("thread-reply-actions-fixture", app: app)
        replyActions.buttons["Copy text"].tap()
        try require(app.buttons["Back to channel"], timeout: 5, "Missing thread navigation").tap()
        try require(app.buttons["View thread"], timeout: 5, "Existing thread links must remain available")
        XCTAssertFalse(app.buttons["Reply in thread"].exists)
    }
    #endif

    func testReactionChipsPickerAndEmptySearchState() throws {
        let app = launch(fixture: "reaction-chips")
        let own = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "selected by you")).firstMatch
        XCTAssertTrue(own.waitForExistence(timeout: 10))
        let other = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "not selected by you")).firstMatch
        XCTAssertTrue(other.exists)
        XCTAssertGreaterThanOrEqual(app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "reaction,")).count, 20)
        #if os(macOS)
        let hoverTargetID = "chan00000001m01"
        let target = app.descendants(matching: .any)["message-row-\(hoverTargetID)"]
        XCTAssertTrue(target.waitForExistence(timeout: 5))
        try hoverMessage(target)
        XCTAssertTrue(app.buttons["add-reaction-\(hoverTargetID)"].waitForExistence(timeout: 2), "Message controls appear on hover")
        #endif
        let rowShortcut = app.buttons["reaction-row-add-chan00000001m01"]
        XCTAssertTrue(rowShortcut.exists, "Messages with reactions offer an inline add-reaction shortcut")
        capture("reaction-chips-wrapped-fixture", app: app)
        let targetID = "chan00000001m01"
        #if os(iOS)
        let row = app.descendants(matching: .any)["message-row-\(targetID)"]
        XCTAssertTrue(row.waitForExistence(timeout: 5)); hold(row)
        XCTAssertTrue(app.descendants(matching: .any)["message-actions-sheet"].waitForExistence(timeout: 5))
        capture("message-actions-drawer-fixture", app: app)
        app.buttons["Copy text"].tap()
        assertCopied("TEST FIXTURE — local sample data, not a live conversation.", in: app)
        var dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.descendants(matching: .any)["message-actions-sheet"])
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed)
        hold(row)
        XCTAssertTrue(app.descendants(matching: .any)["message-actions-sheet"].waitForExistence(timeout: 5))
        app.buttons["Copy message ID"].tap()
        assertCopied(targetID, in: app)
        dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.descendants(matching: .any)["message-actions-sheet"])
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed)
        #endif
        rowShortcut.tap()
        let search = app.textFields["reaction-picker-search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        XCTAssertFalse(app.descendants(matching: .any)["message-actions-sheet"].exists,
                       "The inline shortcut opens the full picker directly")
        let smileys = app.buttons["emoji-category-smileys_people"]
        let activities = app.buttons["emoji-category-activities"]
        XCTAssertTrue(smileys.isSelected)
        XCTAssertTrue(app.buttons["grinning-face"].exists)
        activities.tap()
        XCTAssertTrue(activities.isSelected)
        XCTAssertFalse(app.buttons["grinning-face"].exists)
        search.tap(); search.typeText("grinning-face")
        XCTAssertTrue(app.buttons["grinning-face"].waitForExistence(timeout: 5), "Search must cross categories")
        activities.tap()
        let clearedQuery = search.value as? String
        XCTAssertTrue(clearedQuery == "" || clearedQuery == search.placeholderValue,
                      "Category selection must clear the search")
        XCTAssertFalse(app.buttons["grinning-face"].exists)
        capture("reaction-picker-open-fixture", app: app)
        // Retapping until focus succeeds can hide a dismiss/re-present loop.
        search.tap()
        XCTAssertTrue(hasKeyboardFocus(search), "Emoji search must keep focus after one tap")
        search.typeText("definitely-no-such-emoji")
        XCTAssertTrue(app.descendants(matching: .any)["reaction-picker-empty"].waitForExistence(timeout: 5))
        capture("reaction-picker-empty-fixture", app: app)
    }

    #if os(macOS)
    func testReactionPickerIsCompactAndDismissesOutsideAndWithEscape() throws {
        let app = launch(fixture: "reaction-chips")
        let targetID = "chan00000001m01"
        try openReactionPicker(for: targetID, in: app)
        let picker = try require(app.descendants(matching: .any)["reaction-picker"], timeout: 5,
                                 "The picker must expose its bounded content")
        let search = try require(app.textFields["reaction-picker-search"], timeout: 5, "Missing emoji search")
        let size = picker.frame.size
        XCTAssertEqual(size.width, 352, accuracy: 1)
        XCTAssertEqual(size.height, 420, accuracy: 1)
        let grid = try require(app.scrollViews["reaction-picker-grid"], timeout: 5, "The catalog must scroll inside the picker")
        XCTAssertLessThan(grid.frame.height, size.height)
        XCTAssertTrue(grid.buttons.firstMatch.isHittable)
        XCTAssertTrue(try require(picker.buttons["Cancel"], timeout: 5, "Cancel must be inside the popover").isHittable)
        capture("reaction-picker-compact-catalog-fixture", app: app)
        search.tap()
        XCTAssertTrue(hasKeyboardFocus(search))
        search.typeText("definitely-no-such-emoji")
        try require(app.descendants(matching: .any)["reaction-picker-empty"], timeout: 5, "Missing empty search state")
        XCTAssertEqual(picker.frame.width, size.width, accuracy: 1)
        XCTAssertEqual(picker.frame.height, size.height, accuracy: 1)
        XCTAssertTrue(picker.buttons["Cancel"].isHittable, "Empty results must leave Cancel available")
        capture("reaction-picker-compact-empty-fixture", app: app)

        // Click a real control outside, rather than cancelling the sheet.
        let composer = try require(app.descendants(matching: .any)["message-composer"], timeout: 5, "Missing composer")
        XCTAssertFalse(picker.frame.intersects(composer.frame), "The compact picker must leave the composer available")
        composer.tap()
        let outsideClosed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: picker)
        XCTAssertEqual(XCTWaiter.wait(for: [outsideClosed], timeout: 5), .completed)
        XCTAssertFalse(search.exists)

        try openReactionPicker(for: targetID, in: app)
        try require(search, timeout: 5, "Outside dismissal must allow reopening")
        XCTAssertEqual(search.value as? String, "", "Reopening must clear the old query")
        app.typeKey(.escape, modifierFlags: [])
        let escapeClosed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: picker)
        XCTAssertEqual(XCTWaiter.wait(for: [escapeClosed], timeout: 5), .completed)
    }

    func testReactionFocusDoesNotOutlineUnrelatedButtons() throws {
        let app = launch(fixture: "reaction-chips")
        let targetID = "chan00000001m01"
        let reactions = try require(app.descendants(matching: .any)["reaction-row-\(targetID)"], timeout: 10,
                                    "Missing fixture reactions")
        let own = try require(reactions.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "👍 reaction")).firstMatch,
                              timeout: 5, "Missing selected fixture chip")
        let other = try require(reactions.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "❤️ reaction")).firstMatch,
                                timeout: 5, "Missing unselected fixture chip")
        XCTAssertTrue(own.label.hasSuffix(", selected by you"))
        XCTAssertTrue(other.label.hasSuffix(", not selected by you"))
        try openReactionPicker(for: targetID, in: app)
        try require(app.buttons["Cancel"], timeout: 5, "Missing picker Cancel").tap()
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"),
                                               object: app.descendants(matching: .any)["reaction-picker"])
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 5), .completed)
        let unrelatedRow = try require(app.descendants(matching: .any)["message-row-chan00000001m02"], timeout: 5,
                                       "Missing unrelated hover target")
        try hoverMessage(unrelatedRow)
        let unrelatedAdd = try require(app.buttons["add-reaction-chan00000001m02"], timeout: 5,
                                       "Hovering another message did not reveal Add reaction")

        // Accessibility focus alone cannot detect the bug: the old modifier
        // painted inherited timeline focus around every otherwise valid button.
        let window = app.windows.firstMatch
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: window.screenshot().pngRepresentation))
        let scaleX = CGFloat(bitmap.pixelsWide) / window.frame.width
        let scaleY = CGFloat(bitmap.pixelsHigh) / window.frame.height
        func hasTerracottaOutline(_ element: XCUIElement) throws -> Bool {
            // Just after the picker closes, hit-testing can briefly report every
            // timeline control as covered (CI saw this once, with the window
            // drawn normally), so wait for it rather than checking once.
            let hittable = XCTNSPredicateExpectation(predicate: NSPredicate(format: "isHittable == true"), object: element)
            XCTAssertEqual(XCTWaiter.wait(for: [hittable], timeout: 5), .completed)
            let x = Int((element.frame.midX - window.frame.minX) * scaleX)
            // Sample only the straight top border, away from emoji artwork.
            let y = Int((element.frame.minY - window.frame.minY) * scaleY)
            for offset in -1...2 {
                let color = try XCTUnwrap(bitmap.colorAt(x: x, y: y + offset)?.usingColorSpace(.sRGB))
                if color.redComponent > 0.6 && color.redComponent - color.greenComponent > 0.25 {
                    return true
                }
            }
            return false
        }
        XCTAssertTrue(try hasTerracottaOutline(own), "Your selected reaction must keep its terracotta outline (also calibrates pixel coordinates)")
        XCTAssertFalse(try hasTerracottaOutline(other), "An unselected chip must not inherit another control's focus ring")
        XCTAssertFalse(try hasTerracottaOutline(unrelatedAdd), "Focusing one add-reaction button must not highlight another message's button")
        capture("reaction-focus-isolated-fixture", app: app)
    }

    func testThreadFocusDoesNotOutlineOtherThreadLinks() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        for rootID in ["chan00000001m01", "chan00000001m02", "chan00000001m03"] {
            try await Self.fixtureControl(["incomingReply": ["rootId": rootID, "text": "Focus isolation reply"]])
        }
        // The message list is a plain `.focusable()` view, which macOS only
        // focuses when "Keyboard navigation" is on; CI's Macs leave it off.
        // Turn it on for this launch only (an argument-domain default).
        let app = Self.configuredApp()
        app.launchArguments += ["-AppleKeyboardUIMode", "2"]
        app.launch()
        launchedApp = app
        let selectedRow = try require(app.descendants(matching: .any)["message-row-chan00000001m01"], timeout: 30,
                                      "Missing fixture thread root")
        try require(selectedRow.buttons["View thread"], timeout: 5, "Missing existing thread link").click()
        try require(app.buttons["Back to channel"], timeout: 5, "Thread did not open")

        // Focusing the list must not outline every descendant link. Clicking a
        // thread button can leave this focusable ancestor active on macOS.
        let timeline = try require(app.descendants(matching: .any)["chat-timeline"], timeout: 5, "Missing timeline")
        timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.9)).click()
        // Without focus on the list this can't reproduce the old outline, but
        // the check below still guards what people see after these clicks.
        if !hasKeyboardFocus(timeline) {
            XCTContext.runActivity(named: "The message list did not take focus; checking the links as they look after the clicks") { _ in }
        }
        let window = app.windows.firstMatch
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: window.screenshot().pngRepresentation))
        let scaleX = CGFloat(bitmap.pixelsWide) / window.frame.width
        let scaleY = CGFloat(bitmap.pixelsHigh) / window.frame.height
        for rootID in ["chan00000001m02", "chan00000001m03"] {
            let row = try require(app.descendants(matching: .any)["message-row-\(rootID)"], timeout: 5,
                                  "Missing unrelated thread root")
            let link = try require(row.buttons["View thread"], timeout: 5, "Missing unrelated thread link")
            XCTAssertTrue(link.isHittable)
            let x = Int((link.frame.midX - window.frame.minX) * scaleX)
            let y = Int((link.frame.minY - window.frame.minY) * scaleY)
            // Only sample the straight top border, not the terracotta label.
            for offset in -1...2 {
                let color = try XCTUnwrap(bitmap.colorAt(x: x, y: y + offset)?.usingColorSpace(.sRGB))
                XCTAssertFalse(color.redComponent > 0.6 && color.redComponent - color.greenComponent > 0.25,
                               "An unrelated thread link must not inherit the timeline's focus outline")
            }
        }
        capture("thread-focus-isolated-fixture", app: app)
    }
    #endif

    private func waitForLabel(_ element: XCUIElement, _ text: String, _ message: String,
                              timeout: TimeInterval = 5, line: UInt = #line) {
        let named = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@ OR value == %@", text, text), object: element)
        XCTAssertEqual(XCTWaiter.wait(for: [named], timeout: timeout), .completed,
                       "\(message); found \(element.exists ? element.label : "nothing")", line: line)
    }

    /// Who reacted, from the reaction-chips fixture's locally served lists
    /// (its chips carry local-only authors the fixture server cannot name):
    /// hovering a chip on macOS, holding one on iPhone.
    func testWhoReactedTooltipAndSheetFixture() throws {
        let app = launch(fixture: "reaction-chips")
        let targetID = "chan00000001m01"
        // Scope to this message's reactions: a macOS Touch Bar can repeat controls.
        let reactions = try require(app.descendants(matching: .any)["reaction-row-\(targetID)"], timeout: 10,
                                    "Missing reaction-row-\(targetID)")
        let chip = try require(reactions.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "👍 reaction")).firstMatch,
                               timeout: 10, "Missing the 👍 chip on \(targetID)")
        XCTAssertTrue(chip.label.hasSuffix(", selected by you"), "The fixture's 👍 includes your own reaction")
        #if os(macOS)
        chip.hover()
        let tooltip = try require(app.descendants(matching: .any)["reaction-tooltip"], timeout: 5,
                                  "Hovering a reaction chip did not show who reacted")
        waitForLabel(tooltip, "You and TEST FIXTURE Other reacted with :thumbs-up:", "The tooltip must name who reacted")
        capture("reaction-tooltip-fixture", app: app)
        #else
        chip.press(forDuration: 0.8)
        let sheet = try require(app.descendants(matching: .any)["reactors-sheet"], timeout: 5,
                                "Holding a reaction chip did not open who reacted")
        XCTAssertFalse(app.descendants(matching: .any)["message-actions-sheet"].exists, "Holding a chip must not open message actions")
        try require(sheet.descendants(matching: .any)["reactor-row-fixture-other"], timeout: 5, "The sheet did not list TEST FIXTURE Other")
        XCTAssertTrue(sheet.descendants(matching: .any)["reactor-row-owner0000001"].exists, "The sheet must list your own reaction")
        let emojiName = sheet.descendants(matching: .any)["reactors-emoji-name"]
        waitForLabel(emojiName, ":thumbs-up:", "The held emoji's tab must be selected")
        capture("reactors-sheet-fixture", app: app)
        try require(sheet.descendants(matching: .any)["reactors-tab-😂"], timeout: 2, "Missing the 😂 tab").tap()
        waitForLabel(emojiName, ":face-with-tears-of-joy:", "Choosing a tab must show its people")
        let ownGone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"),
                                                object: sheet.descendants(matching: .any)["reactor-row-owner0000001"])
        XCTAssertEqual(XCTWaiter.wait(for: [ownGone], timeout: 5), .completed, "Only TEST FIXTURE Other reacted with 😂")
        try require(app.navigationBars.buttons["Done"].firstMatch, timeout: 2, "The sheet has no Done button").tap()
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: sheet)
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 5), .completed)
        XCTAssertTrue(chip.label.hasSuffix(", selected by you"), "Holding a chip must not toggle the reaction")
        #endif
    }

    private nonisolated static func fixtureControl(_ body: [String: Any]) async throws {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (_, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
    }

    #if os(iOS)
    /// The attach drawer asks for photo access the first time, then shows the
    /// simulator's sample library; taps attach in order, numbered, and the
    /// draft shows no count until it reaches the limit.
    func testIPhoneAttachDrawerAddsLibraryItemsInTapOrder() async throws {
        try await Self.fixtureControl(["reset": true, "uploadsEnabled": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        let app = launch()
        let attach = try require(app.buttons["attach-files-button"], timeout: 30, "Missing attach button")
        let enabled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "isEnabled == true"), object: attach)
        XCTAssertEqual(XCTWaiter.wait(for: [enabled], timeout: 15), .completed, "Uploads must enable Attach")
        attach.tap()
        // iOS asks once; any of its "allow" choices lets the drawer read the library.
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'Full Access' OR label BEGINSWITH[c] 'Allow'")).firstMatch
        if allow.waitForExistence(timeout: 5) { allow.tap() }
        let tiles = app.buttons.matching(identifier: "attach-drawer-tile")
        XCTAssertTrue(tiles.firstMatch.waitForExistence(timeout: 10), "The simulator's sample photos must appear")
        XCTAssertFalse(app.descendants(matching: .any)["attach-drawer-capacity"].exists, "Below the limit the drawer shows no count")
        capture("attach-drawer-open", app: app)
        guard tiles.count >= 2 else { return }
        tiles.element(boundBy: 1).tap()
        tiles.element(boundBy: 0).tap()
        let first = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Attached 1"), object: tiles.element(boundBy: 1))
        let second = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Attached 2"), object: tiles.element(boundBy: 0))
        XCTAssertEqual(XCTWaiter.wait(for: [first, second], timeout: 15), .completed, "Taps attach in the order they were made")
        XCTAssertTrue(app.buttons["attach-drawer-send"].waitForExistence(timeout: 5), "Send appears once files are attached")
        capture("attach-drawer-selected", app: app)
        // Tapping again removes it from the draft.
        tiles.element(boundBy: 1).tap()
        let removed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Attached 1"), object: tiles.element(boundBy: 0))
        XCTAssertEqual(XCTWaiter.wait(for: [removed], timeout: 5), .completed, "The remaining file moves up to first")
    }
    #endif

    #if os(macOS)
    func testGroupedAttachmentsStayBelowHoverActions() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        let messageID = "chan00000001m03"
        // An unavailable image still renders the real media frame. This tests
        // layout/hit targets, not image decoding or delivery from a CDN.
        for caption in ["Image test", ""] {
            try await Self.fixtureControl(["incomingEdit": [
                "channelId": "chan00000001", "messageId": messageID, "text": caption,
                "attachments": [["id": "fixtureimage001", "kind": "image", "status": "ready",
                                 "name": "TEST FIXTURE image.png", "contentType": "image/png",
                                 "size": 1024, "width": 512, "height": 512,
                                 "url": "http://127.0.0.1:3001/fixture-image.png"]],
            ]])
            let app = launch()
            let row = try require(app.descendants(matching: .any)["message-row-\(messageID)"], timeout: 30,
                                  "Missing grouped media message")
            let media = try require(row.buttons["attachment-fixtureimage001"], timeout: 10, "Missing media frame")
            media.hover()
            let action = try require(row.buttons["add-reaction-\(messageID)"], timeout: 5, "Missing hover action")
            XCTAssertGreaterThanOrEqual(media.frame.minY, action.frame.maxY + 2,
                                       "A follow-up's attachment must clear the entire hover-button target")
            XCTAssertTrue(media.isHittable)
            action.hover()
            XCTAssertTrue(action.isHittable, "Moving off the media onto its action must keep the action available")
            app.terminate()
        }
    }
    #endif

    #if os(iOS)
    func testLatestReactionsStayAboveOpenKeyboard() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        let app = launch()
        let messageID = "chan00000001m04"
        let composer = try require(app.descendants(matching: .any)["message-composer"], timeout: 30,
                                   "Missing composer")
        type("Unsent reaction test draft", into: composer)
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let timeline = try require(app.descendants(matching: .any)["chat-timeline"], timeout: 5,
                                   "Missing timeline")

        // A real fixture reaction event grows the latest row without touching
        // keyboard focus. Use the normal session/PUT flow, never production.
        var session = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/chat/session")!)
        session.httpMethod = "POST"
        session.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        session.setValue("application/json", forHTTPHeaderField: "content-type")
        session.httpBody = Data("{}".utf8)
        let (data, response) = try await URLSession.shared.data(for: session)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        let value = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let token = try XCTUnwrap(value["token"] as? String)
        let reactions = app.descendants(matching: .any)["reaction-row-\(messageID)"]
        for emoji in ["👍", "🎉", "🚀", "❤️", "😂", "🔥", "👏", "👀", "✅", "💯", "🙌", "🤔", "⭐"] {
            var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/chat/channels/chan00000001/messages/\(messageID)/reactions")!)
            request.httpMethod = "PUT"
            request.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
            request.setValue(token, forHTTPHeaderField: "x-caper-chat-token")
            request.setValue("application/json", forHTTPHeaderField: "content-type")
            request.httpBody = try JSONSerialization.data(withJSONObject: ["emoji": emoji, "active": true])
            let (_, saved) = try await URLSession.shared.data(for: request)
            XCTAssertEqual((saved as? HTTPURLResponse)?.statusCode, 200)
            let chip = reactions.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "\(emoji) reaction,")).firstMatch
            try require(chip, timeout: 5, "Missing reaction \(emoji)")
            let fullyVisible = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
                // Each frame read queries the app again; four reads per check
                // could outlast the wait on a slow simulator before a second check.
                let chipFrame = chip.frame, timelineFrame = timeline.frame
                return chipFrame.minY >= timelineFrame.minY && chipFrame.maxY <= timelineFrame.maxY + 1
            }, object: chip)
            XCTAssertEqual(XCTWaiter.wait(for: [fullyVisible], timeout: 5), .completed,
                           "The entire reaction must remain above the composer with the keyboard open")
            XCTAssertTrue(app.keyboards.firstMatch.exists)
            XCTAssertEqual(composer.value as? String, "Unsent reaction test draft")
        }
        XCTAssertLessThanOrEqual(reactions.frame.maxY, timeline.frame.maxY + 1,
                                "Every wrapped reaction row must clear the composer")
        capture("latest-reactions-open-keyboard-fixture", app: app)
    }
    #endif

    func testReactionPickerRetainsSearchAndTargetDuringLiveScroll() async throws {
        try await Self.fixtureControl(["reset": true])
        // Runs however this test ends, so its live messages and reaction never
        // leak into later tests.
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }

        let app = launch()
        let targetID = "chan00000001m01"
        let row = try require(app.descendants(matching: .any)["message-row-\(targetID)"], timeout: 30,
                              "Missing message-row-\(targetID)")
        try openReactionPicker(for: targetID, in: app)
        let search = app.textFields["reaction-picker-search"]
        try require(search, timeout: 5, "The reaction picker search field never appeared")
        search.tap()
        XCTAssertTrue(hasKeyboardFocus(search), "One tap must focus search without reopening the sheet")
        search.typeText("rocket")
        // Scope to the picker grid: on macOS a Touch Bar item also titled
        // "rocket" appears while searching, so app.buttons["rocket"] matches
        // twice and tapping it fails.
        let rockets = try require(app.scrollViews["reaction-picker-grid"].buttons["rocket"], timeout: 5,
                                  "Searching rocket never showed the rocket emoji")

        // Push the presenting row out of the lazy timeline's viewport
        // while the picker is open and its keyboard has focus.
        var lastText = ""
        for index in 0..<12 {
            lastText = "Picker live scroll \(index)\n" + String(repeating: "TEST FIXTURE timeline layout change.\n", count: 8)
            try await Self.fixtureControl(["incomingMessage": ["channelId": "chan00000001", "text": lastText]])
            try require(search, timeout: 2, "Live delivery must not dismiss the reaction picker")
            XCTAssertEqual(search.value as? String, "rocket", "Live delivery must not reset the search")
            #if os(iOS)
            // The private focus predicate can be false with a visible caret and
            // keyboard. Type through the app without retapping the search field;
            // unique input also rejects a stale value from an earlier iteration.
            let input = String(index)
            app.typeText(input)
            let received = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "rocket" + input), object: search)
            XCTAssertEqual(XCTWaiter.wait(for: [received], timeout: 3), .completed,
                           "Live scrolling must retain keyboard input in the existing search field")
            app.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: input.count))
            let restored = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "rocket"), object: search)
            XCTAssertEqual(XCTWaiter.wait(for: [restored], timeout: 3), .completed,
                           "Deleting the focus probe must restore the search without refocusing")
            #else
            XCTAssertTrue(hasKeyboardFocus(search), "Live scrolling must not replace the sheet")
            #endif
        }
        capture("reaction-picker-focused-after-live-scroll-fixture", app: app)
        try require(rockets, timeout: 2, "Live delivery must not drop the rocket search result")
        rockets.tap()
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: search)
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 5), .completed)
        assertStaticText("TEST FIXTURE — \(lastText)", in: app)
        // A lazy timeline may drop the off-screen row entirely; that also
        // means it is not visible.
        XCTAssertFalse(row.exists && row.isHittable, "The original presenter must have left the visible timeline")

        // Check persistence on the original target, not only picker closure
        // or an optimistic chip on whichever message is now visible.
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/chat/channels/chan00000001/messages")!,
                                 cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 5)
        request.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        var messages: [[String: Any]] = []
        var saved = false
        for _ in 0..<50 {
            let (data, response) = try await URLSession.shared.data(for: request)
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
            let history = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            messages = try XCTUnwrap(history["messages"] as? [[String: Any]])
            let target = try XCTUnwrap(messages.first { $0["id"] as? String == targetID })
            saved = (target["reactions"] as? [[String: Any]])?.contains {
                $0["emoji"] as? String == "🚀" && $0["authorIds"] as? [String] == ["owner0000001"]
            } == true
            if saved { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertTrue(saved, "The reaction must be stored on the original message")
        let latest = try XCTUnwrap(messages.last)
        XCTAssertTrue((latest["reactions"] as? [[String: Any]] ?? []).isEmpty)
        let latestID = try XCTUnwrap(latest["id"] as? String)
        try openReactionPicker(for: latestID, in: app)
        try require(search, timeout: 5, "The reopened reaction picker has no search field")
        // An empty field reports its placeholder as its value on iOS 27.
        let query = search.value as? String
        XCTAssertTrue(query == "" || query == search.placeholderValue,
                      "A newly opened picker starts with a fresh query, got \(query ?? "nil")")
        try require(app.buttons["Cancel"], timeout: 2, "The reaction picker has no Cancel button").tap()
        let cancelled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: search)
        XCTAssertEqual(XCTWaiter.wait(for: [cancelled], timeout: 5), .completed)
        app.terminate()
    }

    private func assertStaticText(_ text: String, in app: XCUIApplication, timeout: TimeInterval = 10) {
        XCTAssertTrue(staticTexts(text, in: app).firstMatch.waitForExistence(timeout: timeout), "Missing text: \(text)")
    }

    /// The sidebar handle is a slider to accessibility: AppKit reports its
    /// width as a number, the spoken description as "N pixels".
    private func sidebarWidth(of handle: XCUIElement) -> Int? {
        if let number = handle.value as? NSNumber { return number.intValue }
        return (handle.value as? String).flatMap { Double($0.split(separator: " ").first ?? "") }.map { Int($0.rounded()) }
    }

    #if os(macOS)
    /// Wide windows open Members by default (#597). In CI's 1024-pt window it
    /// leaves the timeline 464 pt: messages wrap and the thread's room for
    /// widening shrinks below its 300-pt minimum, as on web. Tests about the
    /// timeline or thread width close it first, as a person would.
    private func hideMembers(in app: XCUIApplication) throws {
        try require(app.buttons["Hide member list"], timeout: 30, "Wide layouts open Members by default").click()
        try require(app.buttons["Show member list"], timeout: 3, "Members must close")
    }
    #endif

    private func outputGain(of slider: XCUIElement) -> Int? {
        #if os(macOS)
        // AppKit exposes the domain value as NSNumber, not the spoken value.
        return (slider.value as? NSNumber)?.intValue
        #else
        guard let value = slider.value as? String, value.hasSuffix("%") else { return nil }
        return Int(value.dropLast())
        #endif
    }

    func testPopulatedWorkspace() {
        let app = launch()
        assertElement("selected-channel-name", label: "# general", in: app)
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app)
        XCTAssertEqual(staticTexts("caper", in: app).count, 0, "The workspace must not have a web-style branding header")
        #if os(macOS)
        assertElement("selected-space-name", label: "Fixture Studio", in: app)
        XCTAssertTrue(app.buttons["Hide member list"].exists)
        assertStaticText("Members", in: app)
        #else
        XCTAssertEqual(staticTexts("Members", in: app).count, 0, "Chat opens with the members list closed")
        #endif
        capture("populated", app: app)
    }

    func testStableChannelRowsAndOwnerSettingsMenu() {
        let app = launch()
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        let general = app.buttons["channel-chan00000001"]
        let design = app.buttons["channel-chan00000002"]
        let generalVoice = app.buttons["join-voice-chan00000001"]
        let designVoice = app.buttons["join-voice-chan00000002"]
        XCTAssertTrue(general.waitForExistence(timeout: 10))
        XCTAssertTrue(design.exists)
        let lastChannel = app.buttons["channel-chan00000003"]
        let directHeading = staticTexts("Direct messages", in: app).firstMatch
        XCTAssertTrue(lastChannel.exists)
        XCTAssertTrue(directHeading.exists)
        XCTAssertGreaterThanOrEqual(directHeading.frame.minY - lastChannel.frame.maxY, 28,
                                    "The DM divider needs breathing room below the last channel's highlight")
        XCTAssertFalse(app.descendants(matching: .any)["voice-count-chan00000001"].exists, "Empty channels show no voice count")
        // A live row keeps its line, ending with a green waveform and its count,
        // and grows into a card in place, with who is in it and Join beneath.
        let designCount = app.descendants(matching: .any)["voice-count-chan00000002"]
        XCTAssertTrue(designCount.waitForExistence(timeout: 10), "The fixture's design-channel occupants are counted on its row")
        XCTAssertEqual(designCount.label, "2 in voice")
        XCTAssertEqual(designCount.frame.midY, design.frame.midY, accuracy: 2, "The count sits on the channel's own line")
        XCTAssertFalse(app.buttons["voice-stack-chan00000002"].exists, "Rows no longer expand a voice roster")
        let designCard = app.descendants(matching: .any)["live-room-chan00000002"]
        XCTAssertTrue(designCard.exists, "The live row grows into its room card")
        XCTAssertTrue(designCard.frame.contains(design.frame), "The row is its card's first line")
        XCTAssertLessThanOrEqual(design.frame.minY - general.frame.maxY, 10,
                                 "An empty channel is a single line, with no voice action beneath it; the card's row sits just inside its border")
        XCTAssertEqual(design.frame.height, general.frame.height, accuracy: 0.5, "A live channel's line keeps the row's height")
        XCTAssertGreaterThan(lastChannel.frame.minY, designCard.frame.maxY, "The card keeps its row's place in the list")
        #if os(iOS)
        // Touch shows the one-click join on quiet rows; a live row's card has Join.
        XCTAssertTrue(generalVoice.exists, "Touch shows a quiet row's join without hover")
        XCTAssertEqual(generalVoice.label, "Join voice in #general")
        XCTAssertGreaterThanOrEqual(generalVoice.frame.width, 44)
        XCTAssertGreaterThanOrEqual(generalVoice.frame.height, 44)
        XCTAssertEqual(generalVoice.frame.midY, general.frame.midY, accuracy: 1, "The join sits on the channel's own line")
        XCTAssertFalse(designVoice.exists, "Touch hides a live row's join: its card has Join")
        XCTAssertGreaterThanOrEqual(general.frame.height, 44)
        XCTAssertGreaterThanOrEqual(design.frame.height, 44)
        #else
        XCTAssertEqual(general.frame.height, 32)
        XCTAssertEqual(design.frame.height, 32)
        // The pointer shortcut is revealed by hovering its row, on the name's line.
        design.hover()
        XCTAssertTrue(designVoice.waitForExistence(timeout: 3), "Hover reveals an occupied channel's join shortcut")
        XCTAssertEqual(designVoice.label, "Join voice in #design")
        general.hover()
        XCTAssertTrue(generalVoice.waitForExistence(timeout: 3), "Hover reveals an empty channel's join shortcut")
        XCTAssertEqual(generalVoice.label, "Join voice in #general")
        XCTAssertEqual(generalVoice.frame.midY, general.frame.midY, accuracy: 1, "The shortcut sits on the channel's own line")
        XCTAssertGreaterThanOrEqual(generalVoice.frame.minX, general.frame.maxX, "The shortcut follows the channel name")
        XCTAssertEqual(generalVoice.frame.width, 28, accuracy: 0.5)
        XCTAssertEqual(generalVoice.frame.height, 28, accuracy: 0.5)
        XCTAssertLessThanOrEqual(generalVoice.frame.maxX,
                                 app.descendants(matching: .any)["channel-options-chan00000001"].frame.minX,
                                 "The shortcut sits before the channel menu")
        #endif

        let generalFrame = general.frame
        #if os(macOS)
        general.hover()
        XCTAssertTrue(generalVoice.waitForExistence(timeout: 3))
        let voiceFrame = generalVoice.frame
        #endif
        let optionsFrame = app.descendants(matching: .any)["channel-options-chan00000001"].frame
        // The occupied room is in its own row's card: nothing sits above the channels.
        XCTAssertFalse(voiceAboveChannels(in: app), "Every live room here has a row: no cards above the channels")
        XCTAssertGreaterThan(designCard.frame.minY, general.frame.maxY, "The room's card follows #general, in list order")
        #if os(macOS)
        design.hover()
        general.hover()
        XCTAssertTrue(generalVoice.waitForExistence(timeout: 3))
        XCTAssertEqual(generalVoice.frame, voiceFrame, "Hovering other rows must not move the join shortcut")
        #endif
        XCTAssertEqual(general.frame, generalFrame, "Hover must not move the channel name")
        XCTAssertEqual(app.descendants(matching: .any)["channel-options-chan00000001"].frame, optionsFrame,
                       "Hover must not move the channel menu")

        let options = app.descendants(matching: .any)["channel-options-chan00000001"]
        XCTAssertTrue(options.exists, "Hover reveals the channel menu; touch always shows it")
        options.tap()
        let settings = app.descendants(matching: .any)["Channel settings"].firstMatch
        XCTAssertTrue(settings.waitForExistence(timeout: 3))
        settings.tap()
        assertStaticText("Channel settings", in: app, timeout: 3)
        XCTAssertEqual(app.textFields["project-updates"].value as? String, "general", "The menu opens its own channel's editor")
        capture("channel-settings-from-menu", app: app)
    }

    func testInvitePeopleUsesSpaceDefaultAndClickedChannel() throws {
        let app = launch()
        assertElement("selected-channel-name", label: "# general", in: app)
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        try require(app.buttons["new-message"], timeout: 10, "Owners also get New message in the DM footer").tap()
        _ = try require(app.textFields["dm-username"], timeout: 5, "The footer opens a new DM, not space settings")
        app.buttons["Close"].firstMatch.tap()
        app.descendants(matching: .any)["selected-space-name"].tap()
        try require(app.descendants(matching: .any)["Invite people…"].firstMatch, timeout: 5, "The owner space menu offers invitations").tap()
        assertStaticText("Invite by username", in: app, timeout: 5)
        _ = try require(app.buttons["default-invite-link-copy"], timeout: 10, "The primary link is ready to copy")
        let defaultSummary = try require(app.staticTexts["default-invite-link-summary"], timeout: 5, "The returned link reports its default landing channel")
        XCTAssertTrue(defaultSummary.label.hasPrefix("#general · 100 of 100 joins remaining · Expires in 30 days"))
        XCTAssertFalse(app.staticTexts["Opens in"].exists)
        app.buttons["Close"].firstMatch.tap()
        #if os(macOS)
        app.buttons["channel-chan00000002"].hover()
        #endif
        try require(app.descendants(matching: .any)["channel-options-chan00000002"], timeout: 10, "Open the unselected design channel's menu").tap()
        try require(app.descendants(matching: .any)["Invite people…"].firstMatch, timeout: 5, "Public channel offers invitations").tap()
        assertStaticText("Invite by username", in: app, timeout: 5)
        _ = try require(app.buttons["default-invite-link-copy"], timeout: 10, "The clicked channel's primary link is ready to copy")
        let channelSummary = try require(app.staticTexts["default-invite-link-summary"], timeout: 5, "The returned link reports the clicked channel, not the selected channel")
        XCTAssertTrue(channelSummary.label.hasPrefix("#design · 100 of 100 joins remaining · Expires in 30 days"))
        XCTAssertFalse(app.staticTexts["Opens in"].exists)
        app.buttons["Close"].firstMatch.tap()
        #if os(iOS)
        app.buttons["Close navigation"].tap()
        #endif
        assertElement("selected-channel-name", label: "# general", in: app)
    }

    func testLeaveChannelLivesInItsOwnOptionsMenu() {
        let app = launch()
        assertElement("selected-channel-name", label: "# general", in: app)
        XCTAssertFalse(app.buttons["Leave channel"].exists, "Leave must not appear in the chat header")
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #else
        app.buttons["channel-chan00000002"].hover()
        #endif
        let options = app.descendants(matching: .any)["channel-options-chan00000002"]
        XCTAssertTrue(options.waitForExistence(timeout: 10))
        options.tap()
        let leave = app.descendants(matching: .any)["Leave channel"].firstMatch
        XCTAssertTrue(leave.waitForExistence(timeout: 3))
        leave.tap()
        assertStaticText("Leave #design?", in: app, timeout: 3)
        app.buttons["Cancel"].tap()
        #if os(iOS)
        app.buttons["Close navigation"].tap()
        #endif
        assertElement("selected-channel-name", label: "# general", in: app)
        XCTAssertFalse(app.buttons["Leave channel"].exists)
        capture("channel-leave-from-menu", app: app)
    }

    /// The signed-in fixture account's overrides, each as "scope id:mutedUntil".
    private nonisolated static func fixtureNotificationOverrides() async throws -> [String] {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/notifications/settings")!)
        request.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        let (data, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        let settings = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let overrides = try XCTUnwrap(settings["overrides"] as? [[String: Any]])
        return overrides.map { entry in
            let channel: String? = entry["channelId"] as? String
            let conversation: String? = entry["conversationId"] as? String
            let space: String? = entry["spaceId"] as? String
            let id: String = channel ?? conversation ?? space ?? "?"
            let mute: String = entry["mutedUntil"] as? String ?? "null"
            return "\(id):\(mute)"
        }
    }

    /// Waits until the element's accessibility value does (or does not) mention "Muted".
    private func waitForMuted(_ element: XCUIElement, _ muted: Bool, _ message: String) {
        let format = muted ? "value CONTAINS %@" : "NOT (value CONTAINS %@)"
        let matches = XCTNSPredicateExpectation(predicate: NSPredicate(format: format, "Muted"), object: element)
        XCTAssertEqual(XCTWaiter.wait(for: [matches], timeout: 5), .completed, message)
    }

    /// Mutes #design for the fixture account through the API, as another client would.
    private nonisolated static func muteDesignInFixture() async throws {
        let url = URL(string: "http://127.0.0.1:3001/api/spaces/space0000001/channels/chan00000002/notifications")!
        var request = URLRequest(url: url)
        request.httpMethod = "PUT"
        request.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.httpBody = try JSONSerialization.data(withJSONObject: ["mutedUntil": "forever"])
        let (_, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
    }

    func testMuteChannelFromItsOptionsMenu() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        #if os(macOS)
        // People can choose a preset with the mouse or the keyboard (checked
        // by hand on a Mac), but XCUITest on CI's Macs can't: its pointer
        // closes the "Mute channel" submenu, and its arrow keys skip both
        // submenus. iPhone drives this same SwiftUI menu end to end, so here
        // the mute comes from the server before launch, and the test checks
        // the app shows it and that Unmute, a top-level item, clears it.
        try await Self.muteDesignInFixture()
        #endif
        let app = launch()
        assertElement("selected-channel-name", label: "# general", in: app)
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        let design = app.buttons["channel-chan00000002"]
        XCTAssertTrue(design.waitForExistence(timeout: 10))
        let options = app.descendants(matching: .any)["channel-options-chan00000002"]
        let mute = app.descendants(matching: .any)["Mute channel"].firstMatch

        #if os(iOS)
        XCTAssertEqual(design.value as? String ?? "", "", "An unselected, unmuted channel has no state")
        options.tap()
        XCTAssertTrue(app.descendants(matching: .any)["Notifications"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(mute.waitForExistence(timeout: 5), "The channel menu offers Mute channel")
        let enabled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "isEnabled == true"), object: mute)
        XCTAssertEqual(XCTWaiter.wait(for: [enabled], timeout: 5), .completed, "Mute is available once settings load")
        let forever = app.descendants(matching: .any)["Until I turn it back on"].firstMatch
        mute.tap()
        XCTAssertTrue(forever.waitForExistence(timeout: 3))
        forever.tap()
        #endif
        waitForMuted(design, true, "A muted channel is marked in the sidebar")
        var overrides = try await Self.fixtureNotificationOverrides()
        XCTAssertEqual(overrides, ["chan00000002:forever"], "The fixture stored the channel mute")
        capture("channel-muted", app: app)

        #if os(macOS)
        design.hover() // Row options appear on hover (#404); hidden ones aren't in the tree.
        #endif
        options.tap()
        let unmute = app.descendants(matching: .any)["Unmute channel"].firstMatch
        XCTAssertTrue(unmute.waitForExistence(timeout: 3), "A muted channel's menu offers Unmute channel")
        XCTAssertTrue(app.descendants(matching: .any)["Muted"].firstMatch.exists, "The menu says it is muted")
        unmute.tap()
        waitForMuted(design, false, "Unmuting clears the sidebar mark")
        overrides = try await Self.fixtureNotificationOverrides()
        XCTAssertEqual(overrides, [], "Unmuting clears the override on the server")
        #if os(macOS)
        design.hover()
        options.tap()
        XCTAssertTrue(mute.waitForExistence(timeout: 3), "An unmuted channel's menu offers Mute channel again")
        app.typeKey(.escape, modifierFlags: [])
        #endif
    }

    /// Waits for the open conversation, by the header's channel name. Phones
    /// show it once Browse has closed.
    private func assertOpenChannel(_ name: String, in app: XCUIApplication, _ message: String,
                                   file: StaticString = #filePath, line: UInt = #line) {
        let header = app.descendants(matching: .any)["selected-channel-name"]
        let open = NSPredicate(format: "label == %@ OR value == %@", "# \(name)", "# \(name)")
        let opened = XCTNSPredicateExpectation(predicate: open, object: header)
        XCTAssertEqual(XCTWaiter.wait(for: [opened], timeout: 5), .completed, message, file: file, line: line)
    }

    func testLiveRoomCardGrowsInItsRowAndVoiceTargetDoesNotChangeChat() async throws {
        let app = launch()
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app)
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        // The occupied room's own row grows into its card, in place, without joining.
        let room = app.descendants(matching: .any)["live-room-chan00000002"]
        XCTAssertTrue(room.waitForExistence(timeout: 10), "The fixture's design-channel occupants must be visible without joining")
        XCTAssertTrue(room.frame.contains(app.buttons["channel-chan00000002"].frame), "#design's row is its card's first line")
        XCTAssertGreaterThan(room.frame.minY, app.buttons["channel-chan00000001"].frame.maxY, "The card keeps list order, after #general")
        // No heading, room count or fold: nothing above the channels says Live now.
        XCTAssertFalse(voiceAboveChannels(in: app), "A live room with a row has no card above the channels")
        XCTAssertFalse(app.descendants(matching: .any)["live-now-heading"].exists, "No Live now heading")
        XCTAssertEqual(staticTexts("Live now", in: app).count, 0, "Nothing visible says Live now")
        XCTAssertFalse(app.descendants(matching: .any)["voice-call-card"].exists, "Not in a call: no call card")
        XCTAssertFalse(app.descendants(matching: .any)["live-more"].exists, "No fold")
        // One line of names, which truncates rather than wraps.
        assertElement("live-room-names-chan00000002", label: "TEST FIXTURE Maya and TEST FIXTURE Alex", in: app)
        let join = app.buttons["live-join-chan00000002"]
        XCTAssertTrue(join.exists, "The room card offers Join")
        XCTAssertEqual(join.label, "Join voice in #design")
        #if os(iOS)
        XCTAssertGreaterThanOrEqual(join.frame.height, 44, "Touch keeps a 44-point target around the 40-point Join")
        #endif
        let count = app.descendants(matching: .any)["voice-count-chan00000002"]
        XCTAssertTrue(count.exists, "The live row ends with its voice count")
        XCTAssertEqual(count.label, "2 in voice")
        #if os(iOS)
        let selected = app.buttons["channel-chan00000001"]
        XCTAssertEqual(selected.value as? String, "Selected")
        #else
        let selected = app.descendants(matching: .any)["selected-channel-name"]
        XCTAssertEqual(selected.value as? String, "# general")
        #endif
        #if os(macOS)
        app.buttons["channel-chan00000002"].hover()
        // The shortcut is transparent (absent from accessibility) until SwiftUI applies the hover.
        XCTAssertTrue(app.buttons["join-voice-chan00000002"].waitForExistence(timeout: 3), "Hover reveals the occupied channel's join shortcut")
        #else
        XCTAssertFalse(app.buttons["join-voice-chan00000002"].exists, "Touch hides a live row's join: its card has Join")
        #endif
        #if os(iOS)
        XCTAssertEqual(selected.value as? String, "Selected", "Showing who is in voice must not navigate text chat")
        #else
        XCTAssertEqual(selected.value as? String, "# general", "Showing who is in voice must not navigate text chat")
        #endif
        capture("live-room-card", app: app)

        var control = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        control.httpMethod = "POST"
        control.setValue("application/json", forHTTPHeaderField: "content-type")
        control.httpBody = Data(#"{"mediaAccessDenied":{"channelId":"chan00000002"}}"#.utf8)
        let (_, response) = try await URLSession.shared.data(for: control)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        let removed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: room)
        XCTAssertEqual(XCTWaiter.wait(for: [removed], timeout: 10), .completed,
                       "Revoked private-channel occupancy must disappear without altering selected chat")
        XCTAssertFalse(count.exists, "The row's count leaves with the occupancy")
        XCTAssertTrue(app.buttons["channel-chan00000002"].exists, "The quiet row stays, without its card")
        XCTAssertFalse(voiceAboveChannels(in: app), "Nobody live and no call: nothing above the channels")
        #if os(macOS)
        app.buttons["channel-chan00000002"].hover()
        let deniedAction = app.buttons["join-voice-chan00000002"]
        XCTAssertTrue(deniedAction.waitForExistence(timeout: 3), "Revocation keeps the shortcut's slot")
        XCTAssertFalse(deniedAction.isEnabled, "A revoked channel cannot be joined")
        #endif
        #if os(iOS)
        XCTAssertEqual(selected.value as? String, "Selected")
        #else
        XCTAssertEqual(selected.value as? String, "# general")
        #endif
        // The fixture clears this denial for subsequent tests; it does not
        // restore an evicted watcher in this already-running app.
        control.httpBody = Data(#"{"mediaAccessDenied":{"channelId":"chan00000002","denied":false}}"#.utf8)
        let (_, restored) = try await URLSession.shared.data(for: control)
        XCTAssertEqual((restored as? HTTPURLResponse)?.statusCode, 200)
    }

    /// A tap anywhere on a live room's card but its controls opens the room's
    /// chat, like its row; its faces and names are not separate targets.
    func testLiveRoomCardTapOpensItsChat() throws {
        let app = launch()
        assertOpenChannel("general", in: app, "The fixture opens #general")
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        let names = try require(app.descendants(matching: .any)["live-room-names-chan00000002"], timeout: 10, "#design's card lists who is in it")
        // The names let taps through to the card, so tap where they are drawn.
        #if os(iOS)
        names.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        #else
        names.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click()
        #endif
        assertOpenChannel("design", in: app, "A tap on the card's names opens #design, without joining")
        XCTAssertFalse(app.descendants(matching: .any)["voice-call-card"].exists, "Opening the chat does not join its voice")
    }

    #if os(iOS)
    /// Touch holds a face to open the full list.
    private func openList(from face: XCUIElement) { face.press(forDuration: 0.8) }
    #else
    /// A pointer right-clicks a face to open the full list.
    private func openList(from face: XCUIElement) { face.rightClick() }
    #endif

    func testCallCardGrowsInItsRowWithListAndPhoneStripFixture() {
        let app = launch(fixture: "voice-roster")
        assertElement("selected-channel-name", label: "# general", in: app)
        // Voice is not in the conversation header on any layout.
        for control in ["header-leave-voice", "join-voice-button", "header-mute", "header-deafen"] {
            XCTAssertFalse(app.buttons[control].exists, "The chat header carries no voice controls (\(control))")
        }
        XCTAssertFalse(app.descendants(matching: .any)["header-voice-people"].exists, "The chat header shows no voice faces")
        #if os(iOS)
        // Phones: a 56-point strip directly under the header, with the call and its Mute.
        let strip = app.descendants(matching: .any)["voice-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 10), "Phones show the call in a strip under the chat header")
        XCTAssertEqual(strip.frame.height, 56, accuracy: 1)
        let title = app.descendants(matching: .any)["channel-title-menu"]
        XCTAssertEqual(title.label, "#general options")
        XCTAssertGreaterThanOrEqual(strip.frame.minY, title.frame.maxY - 1, "The strip sits under the header")
        XCTAssertEqual(staticTexts("Leave", in: app).count, 0, "The strip has no Leave; the call's card does")
        let open = app.buttons["voice-strip-open"]
        XCTAssertEqual(open.label, "Voice in #general")
        XCTAssertEqual(open.value as? String, "2 people", "The strip counts the call and names no speaker")
        let stripMute = app.buttons["voice-strip-mute"]
        XCTAssertTrue(stripMute.exists)
        XCTAssertGreaterThanOrEqual(stripMute.frame.width, 44)
        XCTAssertGreaterThanOrEqual(stripMute.frame.height, 44)
        XCTAssertEqual(stripMute.label, "Mute microphone")
        stripMute.tap()
        XCTAssertEqual(stripMute.value as? String, "Muted")
        stripMute.tap()
        XCTAssertEqual(stripMute.value as? String, "On")
        let closeBrowse = app.buttons["Close navigation"]
        XCTAssertFalse(closeBrowse.exists && closeBrowse.isHittable, "Mute does not open Browse")
        capture("voice-strip-in-call-test-fixture", app: app)
        // The rest of the strip opens Browse, where the call's card is.
        open.tap()
        XCTAssertTrue(app.buttons["Close navigation"].waitForExistence(timeout: 5), "The strip opens Browse")
        #endif
        // Your call's own row grows into its card, in place: nothing above the channels.
        let card = app.descendants(matching: .any)["voice-call-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 10), "Your call's row grows into its card")
        let general = app.buttons["channel-chan00000001"]
        let design = app.buttons["channel-chan00000002"]
        XCTAssertTrue(card.frame.contains(general.frame), "#general's own row is the card's first line")
        XCTAssertEqual(general.value as? String, "Selected", "The row keeps its selected style for the open chat")
        let room = app.descendants(matching: .any)["live-room-chan00000002"]
        XCTAssertLessThan(card.frame.maxY, room.frame.minY, "Live channels keep list order: #general, then #design")
        XCTAssertGreaterThan(card.frame.minY, app.buttons["channels-toggle"].frame.maxY, "Your call sits among the channels")
        XCTAssertFalse(voiceAboveChannels(in: app), "Both live channels have rows: nothing above the channels")
        XCTAssertEqual(staticTexts("Live now", in: app).count, 0, "Nothing visible says Live now")
        XCTAssertFalse(app.descendants(matching: .any)["voice-call-channel"].exists, "The row, not a second heading, names the call")
        // The participant count lives with the faces, not beside the timer.
        XCTAssertFalse(app.descendants(matching: .any)["voice-count-chan00000001"].exists, "Your call's row shows its status, not a count")
        assertElement("voice-call-count", label: "2 people", in: app)
        XCTAssertFalse(app.descendants(matching: .any)["voice-call-status"].exists)
        // Faces: you first, then join order.
        let me = app.descendants(matching: .any)["voice-face-fixture-self"]
        let maya = app.descendants(matching: .any)["voice-face-fixture-remote"]
        XCTAssertTrue(me.exists)
        XCTAssertTrue(maya.exists)
        XCTAssertEqual(me.label, "Fixture Owner (you)", "Your face carries your saved display name (#527)")
        XCTAssertEqual(maya.label, "TEST FIXTURE Maya")
        XCTAssertEqual(maya.value as? String, "Muted", "A muted face says so")
        XCTAssertLessThan(me.frame.minX, maya.frame.minX, "You come first")
        XCTAssertFalse(app.descendants(matching: .any)["voice-faces-more"].exists, "Two people need no +N tile")
        XCTAssertFalse(app.descendants(matching: .any)["voice-call-speaker"].exists, "No line says who is speaking")
        // Mute, Deafen and Leave. The dock is gone, and the card and account bar share state.
        let leave = app.buttons["voice-call-leave"]
        XCTAssertEqual(leave.label, "Leave voice")
        XCTAssertEqual(app.buttons.matching(NSPredicate(format: "label == %@", "Leave voice")).count, 1, "Leave lives only in the call card")
        XCTAssertFalse(app.descendants(matching: .any)["active-voice-context"].exists, "No bottom voice dock")
        XCTAssertTrue(app.buttons["voice-call-deafen"].exists)
        let cardMute = app.buttons["voice-call-mute"]
        let accountMute = app.buttons["microphone-toggle"]
        XCTAssertEqual(cardMute.value as? String, "On")
        cardMute.tap()
        XCTAssertEqual(cardMute.value as? String, "Muted")
        #if os(macOS)
        XCTAssertEqual(accountMute.value as? String, "Muted", "The card and the account bar show the same microphone")
        accountMute.tap()
        #else
        XCTAssertFalse(accountMute.exists, "Mobile audio controls live on the call card, not the account bar")
        cardMute.tap()
        #endif
        XCTAssertEqual(cardMute.value as? String, "On")
        XCTAssertFalse(app.buttons["join-voice-chan00000001"].exists, "The connected channel's row has no join shortcut")
        let switchHere = app.buttons["live-join-chan00000002"]
        XCTAssertTrue(switchHere.exists, "Other live rooms keep their cards")
        XCTAssertEqual(switchHere.label, "Switch voice to #design", "In a call elsewhere, a room's action switches")
        capture("call-card-in-row-test-fixture", app: app)

        // Read #design, then use the card: its controls keep their own actions.
        design.tap()
        assertOpenChannel("design", in: app, "#design's row opens its chat")
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        XCTAssertTrue(cardMute.waitForExistence(timeout: 5))
        cardMute.tap()
        XCTAssertEqual(cardMute.value as? String, "Muted")
        cardMute.tap()
        XCTAssertEqual(cardMute.value as? String, "On")
        XCTAssertEqual(design.value as? String, "Selected", "Mute does not open the call's chat")

        // Holding (touch) or right-clicking another person's face opens the full
        // list with their volume controls open, and does not open the chat.
        openList(from: maya)
        let list = app.descendants(matching: .any)["voice-participants"]
        XCTAssertTrue(list.waitForExistence(timeout: 5))
        assertElement("voice-list-title", label: "In #general · 2 people", in: app)
        assertStaticText("Fixture Owner (you)", in: app)
        assertStaticText("TEST FIXTURE Maya", in: app)
        XCTAssertFalse(app.buttons["participant-audio-fixture-self"].exists, "Your own row has no local playback controls")
        XCTAssertTrue(app.sliders["TEST FIXTURE Maya volume"].waitForExistence(timeout: 3), "Maya's controls are already open")
        XCTAssertEqual(app.buttons["participant-audio-fixture-remote"].value as? String, "Expanded")
        #if os(macOS)
        let mute = app.checkBoxes["Mute for me"]
        #else
        // The labelled row is a Switch; its centre is the label, so tap the inner control.
        let mute = app.switches["Mute for me"].switches.firstMatch
        #endif
        XCTAssertTrue(mute.exists)
        capture("call-participant-list-test-fixture", app: app)
        mute.tap()
        #if os(iOS)
        XCTAssertEqual(mute.value as? String, "1")
        #endif
        let localMute = app.descendants(matching: .any)["participant-local-muted-fixture-remote"]
        XCTAssertTrue(localMute.waitForExistence(timeout: 3))
        assertStaticText("You muted TEST FIXTURE Maya", in: app)
        closeVoiceList(in: app)
        XCTAssertEqual(design.value as? String, "Selected", "Opening the list does not open the call's chat")
        // Your own face opens the list with nobody's controls open; Audio opens them.
        openList(from: me)
        XCTAssertTrue(list.waitForExistence(timeout: 5))
        XCTAssertFalse(app.sliders["TEST FIXTURE Maya volume"].exists, "Your own face opens no one's controls")
        let audio = app.buttons["participant-audio-fixture-remote"]
        XCTAssertEqual(audio.value as? String, "Collapsed")
        audio.tap()
        XCTAssertTrue(app.sliders["TEST FIXTURE Maya volume"].waitForExistence(timeout: 3))
        mute.tap()
        #if os(iOS)
        XCTAssertEqual(mute.value as? String, "0")
        #endif
        XCTAssertFalse(localMute.waitForExistence(timeout: 1), "Local mute status disappears when remote playback is restored")
        closeVoiceList(in: app)
        XCTAssertEqual(design.value as? String, "Selected")

        // A tap on a face, like anywhere on the card but its controls, opens the call's chat.
        maya.tap()
        assertOpenChannel("general", in: app, "A tap on the card opens the call's chat")
        XCTAssertFalse(list.exists, "A tap opens the chat, not the list")
    }

    /// Seven in the call with Alex speaking: the card shows everyone in join
    /// order (the phone strip the same first four and "+3", by `LiveNow.stripFold`),
    /// and speaking is a ring on a face, never a line of text.
    func testCrowdedCallShowsEveryoneInOrderWithoutSpeakerText() throws {
        let app = launch(fixture: "voice-crowd")
        assertElement("selected-channel-name", label: "# general", in: app)
        #if os(iOS)
        let open = try require(app.buttons["voice-strip-open"], timeout: 10, "Phones show the call in a strip")
        XCTAssertEqual(open.value as? String, "7 people", "The strip counts the call and names nobody while Alex speaks")
        capture("voice-strip-crowd-test-fixture", app: app)
        open.tap()
        #endif
        try require(app.descendants(matching: .any)["voice-call-card"], timeout: 10, "Your call's row grows into its card")
        assertElement("voice-call-count", label: "7 people", in: app)
        let order = ["fixture-self", "fixture-remote", "fixture-crowd-alex", "fixture-crowd-sam", "fixture-crowd-jo", "fixture-crowd-ri", "fixture-crowd-kai"]
        let faces = order.map { app.descendants(matching: .any)["voice-face-\($0)"] }
        for (id, face) in zip(order, faces) { XCTAssertTrue(face.exists, "Up to ten: the card shows everyone (\(id))") }
        XCTAssertFalse(app.descendants(matching: .any)["voice-faces-more"].exists, "Seven need no +N tile")
        for (earlier, later) in zip(faces, faces.dropFirst()) {
            let sameRow = abs(earlier.frame.midY - later.frame.midY) < 2
            XCTAssertTrue(sameRow ? earlier.frame.minX < later.frame.minX : earlier.frame.midY < later.frame.midY,
                          "You first, then join order, across at most two rows")
        }
        XCTAssertEqual(faces[2].value as? String, "Speaking", "Alex's face carries the speaking ring")
        XCTAssertEqual(faces[3].value as? String, "Deafened")
        let speakerText = NSPredicate(format: "label CONTAINS %@ OR value CONTAINS %@", "is speaking", "is speaking")
        XCTAssertEqual(app.staticTexts.matching(speakerText).count, 0, "No text says who is speaking")
        XCTAssertFalse(app.descendants(matching: .any)["voice-call-speaker"].exists)
        capture("call-card-crowd-test-fixture", app: app)
    }

    /// A call whose channel has no row in the listed space (here another
    /// space's) keeps its card above the Channels heading, with no heading of
    /// its own. Collapsing Channels hides only quiet rows.
    func testCallWithoutARowSitsAboveTheChannelsAndCollapsingKeepsVoice() throws {
        let app = launch(fixture: "voice-elsewhere")
        assertElement("selected-channel-name", label: "# general", in: app)
        #if os(iOS)
        let open = try require(app.buttons["voice-strip-open"], timeout: 10, "Phones show the call in a strip")
        XCTAssertEqual(open.label, "Voice in #lobby")
        open.tap()
        #endif
        let card = try require(app.descendants(matching: .any)["voice-call-card"], timeout: 10, "The call keeps a card")
        let channels = app.buttons["channels-toggle"]
        XCTAssertTrue(voiceAboveChannels(in: app), "Voice without a row sits above the channels")
        XCTAssertEqual(staticTexts("Live now", in: app).count, 0, "Nothing visible says Live now")
        XCTAssertLessThan(card.frame.maxY, channels.frame.minY, "Above the Channels heading")
        assertElement("voice-call-channel", label: "#lobby in TEST FIXTURE Elsewhere", in: app)
        assertElement("voice-call-count", label: "2 people", in: app)
        XCTAssertTrue(app.descendants(matching: .any)["voice-face-fixture-remote"].exists, "The card keeps its faces")
        XCTAssertTrue(app.buttons["voice-call-leave"].exists, "and its controls")
        let general = app.buttons["channel-chan00000001"]
        XCTAssertTrue(general.exists)
        XCTAssertFalse(card.frame.intersects(general.frame), "#general, which is not the call's channel, stays a quiet row")
        XCTAssertFalse(app.descendants(matching: .any)["voice-count-chan00000001"].exists)
        let room = app.descendants(matching: .any)["live-room-chan00000002"]
        XCTAssertTrue(room.exists, "#design's room keeps its card in its row")
        XCTAssertEqual(app.buttons["live-join-chan00000002"].label, "Switch voice to #design")
        capture("call-elsewhere-test-fixture", app: app)

        // Collapsed, Channels hides its quiet rows only: voice stays listed.
        channels.tap()
        XCTAssertEqual(channels.value as? String, "Collapsed")
        let hidden = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: general)
        XCTAssertEqual(XCTWaiter.wait(for: [hidden], timeout: 3), .completed, "A quiet row hides")
        XCTAssertFalse(app.buttons["channel-chan00000003"].exists, "Every quiet row hides")
        XCTAssertTrue(app.buttons["channel-chan00000002"].exists, "A live room's row stays")
        XCTAssertTrue(room.exists, "with its card")
        XCTAssertTrue(card.exists, "Your call's card stays above the channels")
        XCTAssertGreaterThan(room.frame.minY, channels.frame.maxY, "The live room stays under the Channels heading")
        capture("channels-collapsed-live-test-fixture", app: app)
        channels.tap()
        XCTAssertTrue(general.waitForExistence(timeout: 3), "Expanding shows the quiet rows again")
    }

    #if os(macOS)
    func testThreadResizeDragKeyboardAndSavedWidth() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        try await Self.fixtureControl(["incomingReply": ["rootId": "chan00000001m01", "text": "Resize fixture reply"]])
        let app = Self.configuredApp()
        app.launchArguments += ["-AppleKeyboardUIMode", "2"]
        app.launch(); launchedApp = app
        func openThread(_ application: XCUIApplication) throws {
            let row = try require(application.descendants(matching: .any)["message-row-chan00000001m01"], timeout: 30, "Missing thread root")
            try require(row.buttons["View thread"], timeout: 5, "Missing thread link").click()
        }
        // Members would take the room the thread widens into; it opens again on relaunch.
        try hideMembers(in: app)
        try openThread(app)
        let handle = try require(app.descendants(matching: .any)["thread-panel-resize"], timeout: 5, "Missing resize handle")
        let panel = app.descendants(matching: .any)["message-thread"]
        // The Slider is a synthetic accessibilityRepresentation, not a visible
        // control. Xcode 16 can fail to compute its hit point: click the actual
        // handle's midpoint, and keep the Slider for width and keyboard checks.
        // The widest the thread may be: 600, or less when the window leaves the
        // timeline too little room (CI windows are narrower than a desktop's).
        handle.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click()
        handle.typeKey(.end, modifierFlags: [])
        let maximum = try XCTUnwrap(sidebarWidth(of: handle))
        XCTAssertLessThanOrEqual(maximum, 600)
        XCTAssertGreaterThanOrEqual(maximum, 350, "The window leaves room to widen the thread")
        handle.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).doubleClick()
        XCTAssertEqual(sidebarWidth(of: handle), 340)
        let initialEdge = panel.frame.minX
        let window = app.windows.firstMatch
        let start = window.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(
            dx: handle.frame.midX - window.frame.minX, dy: handle.frame.midY - window.frame.minY
        ))
        start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: -63, dy: 0)))
        let dragged = try XCTUnwrap(sidebarWidth(of: handle))
        XCTAssertEqual(Double(dragged), Double(min(403, maximum)), accuracy: 2, "Dragging left widens the thread")
        XCTAssertEqual(initialEdge - panel.frame.minX, CGFloat(dragged - 340), accuracy: 2)
        handle.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).doubleClick()
        handle.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click()
        handle.typeKey(.leftArrow, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(of: handle), 350)
        handle.typeKey(.rightArrow, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(of: handle), 340)
        handle.typeKey(.home, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(of: handle), 300)
        handle.typeKey(.end, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(of: handle), maximum)
        handle.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).doubleClick()
        handle.typeKey(.leftArrow, modifierFlags: [])
        app.buttons["Back to channel"].click()
        try openThread(app)
        XCTAssertEqual(sidebarWidth(of: handle), 350, "Closing does not discard the width")
        app.terminate()
        let reopened = launch()
        try hideMembers(in: reopened)
        try openThread(reopened)
        let saved = try require(reopened.descendants(matching: .any)["thread-panel-resize"], timeout: 5, "Missing saved handle")
        XCTAssertEqual(sidebarWidth(of: saved), 350, "Width survives relaunch")
        capture("thread-resized", app: reopened)
        saved.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).doubleClick()
    }

    func testSidebarResizeKeyboardBoundsAndSavedWidth() {
        let app = launch()
        let handle = app.descendants(matching: .any)["channel-sidebar-resize"]
        let channelTitle = app.descendants(matching: .any)["selected-channel-name"]
        XCTAssertTrue(handle.waitForExistence(timeout: 5))
        handle.doubleClick()
        XCTAssertEqual(sidebarWidth(of: handle), 280)
        let initialEdge = channelTitle.frame.minX
        // The handle moves during resize. Anchor the synthesized pointer path
        // to the stationary window, not a lazily resolved moving element.
        let window = app.windows.firstMatch
        let handleFrame = handle.frame
        let start = window.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(
            dx: handleFrame.midX - window.frame.minX, dy: handleFrame.midY - window.frame.minY
        ))
        start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: 35, dy: 0)))
        let dragged = sidebarWidth(of: handle) ?? 0
        XCTAssertTrue((310...320).contains(dragged), "35-point drag should produce width 315, got \(dragged)")
        XCTAssertEqual(channelTitle.frame.minX - initialEdge, CGFloat(dragged - 280), accuracy: 2,
                       "the actual conversation edge must follow the reported sidebar width")
        handle.doubleClick()
        handle.click()
        handle.typeKey(.rightArrow, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(of: handle), 290)
        handle.typeKey(.home, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(of: handle), 220)
        handle.typeKey(.end, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(of: handle), 440)
        handle.doubleClick()
        XCTAssertEqual(sidebarWidth(of: handle), 280, "double-click resets after a keyboard resize")
        handle.typeKey(.rightArrow, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(of: handle), 290, "reset keeps the resize handle focused")
        app.terminate()
        let reopened = launch()
        let saved = reopened.descendants(matching: .any)["channel-sidebar-resize"]
        XCTAssertTrue(saved.waitForExistence(timeout: 5))
        XCTAssertEqual(sidebarWidth(of: saved), 290, "resized width survives relaunch")
        XCTAssertEqual(reopened.descendants(matching: .any)["selected-channel-name"].frame.minX - initialEdge, 10, accuracy: 2)
        capture("sidebar-resized", app: reopened)
        saved.doubleClick()
    }

    func testUpdateButtonStaysVisibleAndReopensReviewWithoutCoveringAccountControls() {
        let app = launch(fixture: "update")
        let update = app.buttons["desktop-update-action"]
        XCTAssertTrue(update.waitForExistence(timeout: 10))
        let handle = app.descendants(matching: .any)["channel-sidebar-resize"]
        XCTAssertTrue(handle.exists)
        for key in [XCUIKeyboardKey.home, .end] {
            handle.click()
            handle.typeKey(key, modifierFlags: [])
            XCTAssertEqual(update.label, "Update available")
            XCTAssertTrue(update.isHittable)
            XCTAssertGreaterThanOrEqual(update.frame.height, 36)
            XCTAssertTrue(app.windows.firstMatch.frame.contains(update.frame))
            for id in ["account-profile", "microphone-toggle", "deafen-toggle", "account-settings-menu"] {
                let control = app.descendants(matching: .any)[id]
                XCTAssertTrue(control.isHittable, id)
                XCTAssertTrue(app.windows.firstMatch.frame.contains(control.frame), id)
                XCTAssertGreaterThanOrEqual(control.frame.minY, update.frame.maxY, id)
            }
            capture(key == .home ? "update-compact-sidebar" : "update-wide-sidebar", app: app)
            update.click()
            XCTAssertTrue(app.buttons["desktop-update-install"].waitForExistence(timeout: 3))
            capture("update-review", app: app)
            app.buttons["Later"].click()
            XCTAssertTrue(update.waitForExistence(timeout: 3))
            XCTAssertTrue(update.isHittable)
            update.click()
            XCTAssertTrue(app.buttons["desktop-update-install"].waitForExistence(timeout: 3))
            app.typeKey(.escape, modifierFlags: [])
            XCTAssertTrue(update.waitForExistence(timeout: 3))
            XCTAssertTrue(app.buttons["account-profile"].isHittable)
        }
        handle.doubleClick()
    }

    func testRowJoinShortcutAndProfileBackdropDismissal() {
        let app = launch()
        let channel = app.buttons["channel-chan00000001"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.hover()
        let join = app.buttons["join-voice-chan00000001"]
        XCTAssertTrue(join.waitForExistence(timeout: 3))
        XCTAssertEqual(join.frame.midY, channel.frame.midY, accuracy: 2, "The join shortcut shares the channel's row, with no line beneath it")
        let settings = app.descendants(matching: .any)["account-settings-menu"]
        XCTAssertTrue(settings.isHittable)
        XCTAssertLessThan(settings.frame.maxX, app.descendants(matching: .any)["channel-sidebar-resize"].frame.midX)
        app.buttons["account-profile"].tap()
        XCTAssertTrue(app.buttons["profile-save"].waitForExistence(timeout: 5))
        app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.02, dy: 0.5)).click()
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.buttons["profile-save"])
        XCTAssertEqual(XCTWaiter().wait(for: [dismissed], timeout: 3), .completed)
        XCTAssertTrue(app.buttons["account-profile"].isHittable)
    }

    func testMembersStartOpenAndCanBeToggledWithoutChangingConversation() {
        let app = launch()
        let toggle = app.buttons["Hide member list"]
        XCTAssertTrue(toggle.waitForExistence(timeout: 10))
        assertStaticText("Members", in: app)
        // Wide headers: the name, then Pins and Members. Voice lives in the sidebar's live cards.
        let name = app.descendants(matching: .any)["selected-channel-name"]
        let pins = app.buttons["channel-pins"]
        XCTAssertTrue(pins.waitForExistence(timeout: 5))
        XCTAssertGreaterThan(pins.frame.minX, name.frame.maxX - 1, "Pins follows the channel name")
        XCTAssertLessThan(pins.frame.maxX, toggle.frame.minX + 1, "Members sits right of Pins")
        XCTAssertFalse(app.buttons["join-voice-button"].exists, "Wide headers have no Join")
        XCTAssertFalse(app.descendants(matching: .any)["header-voice-people"].exists, "Wide headers show no voice faces")
        XCTAssertFalse(app.descendants(matching: .any)["voice-strip"].exists, "Wide layouts have no phone strip")
        XCTAssertFalse(app.descendants(matching: .any)["channel-title-menu"].exists, "Wide headers keep separate Pins and Members buttons")
        XCTAssertFalse(app.buttons["header-mute"].exists, "Wide layouts keep the microphone in the account bar")
        XCTAssertFalse(app.buttons["header-deafen"].exists, "Wide layouts keep deafen in the account bar")
        toggle.tap()
        XCTAssertTrue(app.buttons["Show member list"].waitForExistence(timeout: 2))
        XCTAssertEqual(staticTexts("Members", in: app).count, 0)
        capture("members-hidden", app: app)
        app.buttons["Show member list"].tap()
        assertStaticText("Members", in: app, timeout: 2)
        assertElement("selected-channel-name", label: "# general", in: app, timeout: 2)
        app.buttons["Hide member list"].tap()
        XCTAssertTrue(app.buttons["Show member list"].waitForExistence(timeout: 2))
        assertElement("selected-channel-name", label: "# general", in: app, timeout: 2)
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app, timeout: 2)
        XCTAssertEqual(staticTexts("Members", in: app).count, 0)
    }

    func testCompletedLocalRecordingLayoutWithoutCapture() {
        let app = launch(fixture: "audio-recorded")
        app.descendants(matching: .any)["account-settings-menu"].tap()
        app.descendants(matching: .any)["Audio test"].tap()
        assertStaticText("Only you can hear these tests.", in: app)
        assertStaticText("Try your microphone", in: app)
        assertStaticText("TEST FIXTURE — completed local recording layout only; no microphone or playback.", in: app)
        XCTAssertTrue(app.buttons["local-mic-test"].exists)
        for title in ["Play natural", "Play enhanced", "Stop playback"] {
            let button = app.buttons[title]
            XCTAssertTrue(button.exists, "Missing \(title) in the completed recording layout")
            XCTAssertFalse(button.isEnabled, "A visual fixture must not play synthetic audio")
        }
        XCTAssertTrue(app.sliders["Voice processing"].exists)
        assertStaticText("25%", in: app)
        capture("audio-recorded-test-fixture", app: app)
        let controls = app.scrollViews["audio-preferences-controls"]
        controls.scroll(byDeltaX: 0, deltaY: -600)
        XCTAssertTrue(controls.frame.contains(app.buttons["Play enhanced"].frame), "Replay controls must be reachable in the constrained window")
        XCTAssertFalse(staticTexts("Natural playback includes input gain and on-device noise suppression. Enhanced playback also applies live voice processing strength.", in: app).firstMatch.exists)
        capture("audio-recorded-scrolled-test-fixture", app: app)
        app.buttons["close-audio-preferences"].tap()
        let closed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"),
            object: app.descendants(matching: .any)["audio-preferences-sheet"]
        )
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 5), .completed)
    }

    func testConnectionStatisticsLayoutWithoutVoiceConnection() {
        let app = launch(fixture: "audio-statistics")
        app.descendants(matching: .any)["account-settings-menu"].tap()
        app.descendants(matching: .any)["Connection details"].tap()
        assertStaticText("TEST FIXTURE — synthetic statistics layout; no voice connection.", in: app)
        // Web's ConnectionDiagnostics formatting.
        for value in ["Joined in 812 ms", "214 ms", "391 ms", "4 sent · 4 answered", "88 ms",
                      "0.07 MB", "13 kbps", "0.01 MB", "24 kbps", "17 ms", "42 ms", "TURN relay", "Counters reset on reconnect."] {
            assertStaticText(value, in: app)
        }
        XCTAssertTrue(app.buttons["Copy connection details"].exists)
        capture("audio-statistics-test-fixture", app: app)
    }

    func testAudioDevicesOpenSeparateChoicesAndDismissWithoutChangingVolume() {
        let app = launch(fixture: "audio-recorded")
        app.descendants(matching: .any)["account-settings-menu"].tap()
        app.descendants(matching: .any)["Audio test"].tap()
        let device = app.buttons["audio-input-device"]
        XCTAssertTrue(device.waitForExistence(timeout: 5))
        let volume = app.sliders["Test microphone volume"]
        let originalVolume = volume.value as? String
        device.tap()
        let close = app.buttons["Close device picker"]
        XCTAssertTrue(close.waitForExistence(timeout: 3))
        XCTAssertTrue(app.buttons["System default"].exists)
        app.typeKey(XCUIKeyboardKey.escape.rawValue, modifierFlags: [])
        XCTAssertFalse(close.exists)
        XCTAssertEqual(volume.value as? String, originalVolume)
        device.tap()
        app.buttons["System default"].tap()
        XCTAssertFalse(close.exists)
        XCTAssertEqual(device.value as? String, "System default")
    }

    /// AppKit reports a switch's value as a number (0 or 1), not a string.
    private func switchState(_ toggle: XCUIElement) -> Bool? {
        if let number = toggle.value as? NSNumber { return number.boolValue }
        return (toggle.value as? String).flatMap { ["1": true, "0": false][$0] }
    }

    func testSettingsShortcutWorksWhileSignedOut() {
        let app = launch(signedIn: false)
        assertStaticText("Welcome to Caper", in: app)
        app.typeKey(",", modifierFlags: .command)
        let startup = app.descendants(matching: .any)["launch-at-login"]
        XCTAssertTrue(startup.waitForExistence(timeout: 5))
        XCTAssertFalse(startup.isEnabled, "Parity mode must never change real Login Items")
        let sounds = app.descendants(matching: .any)["sound-effects"]
        let original = switchState(sounds)
        XCTAssertNotNil(original)
        sounds.tap()
        XCTAssertNotEqual(switchState(sounds), original)
        app.typeKey("w", modifierFlags: .command)
        app.typeKey(",", modifierFlags: .command)
        XCTAssertTrue(startup.waitForExistence(timeout: 3))
        XCTAssertNotEqual(switchState(sounds), original, "Settings saves without an Apply button")
        sounds.tap()
        XCTAssertEqual(switchState(sounds), original)
        capture("settings-signed-out", app: app)
    }
    #endif

    func testLogin() {
        // Exercise normal session restoration, not a forced login presentation.
        let app = launch(signedIn: false)
        assertStaticText("Welcome to Caper", in: app)
        XCTAssertEqual(staticTexts("WELCOME TO CAPER", in: app).count, 0)
        let email = app.textFields["Email address"]
        XCTAssertFalse(app.descendants(matching: .any)["message-composer"].exists)
        XCTAssertFalse(app.buttons["Create space"].exists)
        XCTAssertTrue(app.windows.firstMatch.frame.contains(email.frame), "Login must fit the viewport")
        capture("login", app: app)
        email.tap(); email.typeText("owner@example.test")
        app.buttons["Email me a code"].tap()
        XCTAssertTrue(app.textFields["Sign-in code"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.windows.firstMatch.frame.contains(app.textFields["Sign-in code"].frame))
        XCTAssertEqual(staticTexts("WELCOME TO CAPER", in: app).count, 0)
        assertStaticText("Enter the six-character code sent to owner@example.test. It expires in 10 minutes.", in: app)
        XCTAssertTrue(app.buttons["Use a different email"].exists)
        let resend = app.buttons["login-resend-code"]
        XCTAssertTrue(resend.exists, "The code step offers Resend code")
        XCTAssertFalse(resend.isEnabled, "Resend code waits a minute after each code")
        XCTAssertTrue(resend.label.hasPrefix("Resend code in 0:") || resend.label == "Resend code in 1:00", resend.label)
        capture("login-code", app: app)
        let code = app.textFields["Sign-in code"]
        // Separate bursts: the field rewrites itself between keystrokes, as it does for a person typing.
        code.tap(); code.typeText("ZZZ")
        // One rejected keystroke at a time, as a person types: macOS applies the rewrite on the next turn.
        for rejected in ["o", "-"] {
            code.typeText(rejected)
            let filtered = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "ZZZ"), object: code)
            XCTAssertEqual(XCTWaiter.wait(for: [filtered], timeout: 3), .completed, "Letters outside the code alphabet are dropped")
        }
        code.typeText("ZZ9")
        let complete = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "ZZZZZ9"), object: code)
        XCTAssertEqual(XCTWaiter.wait(for: [complete], timeout: 3), .completed, "Code input keeps web's six-character alphabet")
        app.buttons["Continue"].tap()
        assertStaticText("That code is incorrect or expired. Request a new one if needed.", in: app, timeout: 5)
    }

    func testDeleteSpaceRequiresConfirmationAndCanCancel() {
        let app = launch(fixture: "manage-space")
        let delete = app.buttons["Delete space"]
        XCTAssertTrue(delete.waitForExistence(timeout: 10))
        #if os(macOS)
        // The dialog is capped to the window, so Delete sits below its fold. A
        // Mac click is not scrolled into view: it would land on the backdrop,
        // which dismisses the dialog. Scroll to it as a person would.
        // The space icon section sits above, so it can take more than one scroll.
        for _ in 0..<3 where !delete.isHittable {
            app.scrollViews["space-settings-scroll"].scroll(byDeltaX: 0, deltaY: -600)
        }
        XCTAssertTrue(delete.isHittable, "Delete space must be reachable by scrolling the dialog")
        #endif
        delete.tap()
        let confirm = app.buttons["confirm-destructive-action"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        assertStaticText("Delete Fixture Studio for everyone? All its channels and their messages will disappear from the space. This cannot be undone.", in: app)
        capture("delete-space-confirmation", app: app)
        app.buttons["Cancel"].tap()
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: confirm)
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 5), .completed)
        XCTAssertTrue(app.buttons["Save name"].exists)
    }

    func testDeleteChannelRequiresConfirmationAndCanCancel() {
        let app = launch(fixture: "manage-channel")
        let delete = app.buttons["Delete channel"]
        XCTAssertTrue(delete.waitForExistence(timeout: 10))
        #if os(macOS)
        // As with Delete space: in a window shorter than the dialog's 680-point
        // cap, Delete sits at the fold, and a Mac click is not scrolled into view.
        app.scrollViews["channel-settings-scroll"].scroll(byDeltaX: 0, deltaY: -600)
        XCTAssertTrue(delete.isHittable, "Delete channel must be reachable by scrolling the dialog")
        #endif
        delete.tap()
        let confirm = app.buttons["confirm-destructive-action"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        assertStaticText("Delete #planning for everyone? This channel and its messages will disappear from the space. This cannot be undone.", in: app)
        capture("delete-channel-confirmation", app: app)
        app.buttons["Cancel"].tap()
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: confirm)
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 5), .completed)
        XCTAssertTrue(app.buttons["Delete channel"].exists, "Cancelling keeps the overview open")
        // Web shows its save bar only once something changed.
        XCTAssertFalse(app.buttons["Save changes"].exists)
        assertStaticText("Delete this channel for everyone in the space.", in: app)
    }

    func testPendingSendIsVisibleAndLeftAlignedBeforeConfirmation() async throws {
        let app = launch()
        let composer = try require(app.descendants(matching: .any)["message-composer"], timeout: 30,
                                   "Missing message composer")
        let timeline = app.scrollViews["chat-timeline"]
        let send = app.buttons["send-message-button"]
        let path = "http://127.0.0.1:3001/api/chat/channels/chan00000001/messages"
        func holdSends(_ hold: Bool) async throws {
            var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
            request.httpMethod = "POST"
            request.setValue("application/json", forHTTPHeaderField: "content-type")
            request.httpBody = try JSONSerialization.data(withJSONObject: ["holdSends": hold])
            let (_, response) = try await URLSession.shared.data(for: request)
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        }
        // The short row catches intrinsic-width centering; the long row catches
        // wrapping/height changes with the keyboard reducing the viewport.
        let suffix = String(UUID().uuidString.prefix(4))
        for message in ["Hi \(suffix)", String(repeating: "A wrapping pending message. ", count: 8) + suffix] {
            XCTAssertTrue(focus(composer))
            composer.typeText(message)
            // Sending must reveal the row even from older history.
            #if os(macOS)
            timeline.scroll(byDeltaX: 0, deltaY: 600)
            #else
            timeline.swipeDown()
            // History browsing now dismisses the keyboard; reopen it to test sending with it up.
            XCTAssertTrue(focus(composer))
            #endif
            try await holdSends(true)
            do {
                send.tap()
                let cleared = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == ''"), object: composer)
                XCTAssertEqual(XCTWaiter.wait(for: [cleared], timeout: 3), .completed)
                // Do not wait for confirmation: the server has neither stored
                // nor broadcast this send. Inspect the local pending row now.
                let pending = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "pending-message-")).firstMatch
                XCTAssertTrue(pending.exists, "Clearing the composer must leave a visible pending message")
                // A predicate, not `staticTexts[message]`: keying a query by the long
                // message trips XCUIElementQuery's identifier assertion and aborts the run.
                let text = staticTexts(message, in: pending).firstMatch
                XCTAssertTrue(text.exists)
                XCTAssertTrue(timeline.frame.contains(text.frame), "Pending text must be in the visible timeline")
                // The row's own frame can't be checked: accessibility reports a
                // `.contain` container as the union of its children (in CI 130
                // points wide from the avatar, on iPhone and macOS alike), not
                // the full-width row. The text inset below checks the alignment.
                XCTAssertEqual(text.frame.minX, timeline.frame.minX + 62, accuracy: 2,
                               "Pending text must share confirmed messages' 18 + 34 + 10 point inset")
                #if os(iOS)
                XCTAssertTrue(app.keyboards.firstMatch.exists, "Sending must retain the software keyboard")
                #endif
                let pendingX = text.frame.minX
                let clientID = String(pending.identifier.dropFirst("pending-message-".count))
                var history = URLRequest(url: URL(string: path)!)
                history.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
                let (before, _) = try await URLSession.shared.data(for: history)
                let stored = try XCTUnwrap(JSONSerialization.jsonObject(with: before) as? [String: Any])
                XCTAssertFalse((stored["messages"] as? [[String: Any]] ?? []).contains { $0["clientMessageId"] as? String == clientID })
                capture(message.count < 20 ? "send-pending-short" : "send-pending-wrapped", app: app)
                try await holdSends(false)
                let confirmed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: pending)
                XCTAssertEqual(XCTWaiter.wait(for: [confirmed], timeout: 10), .completed)
                let delivered = staticTexts(message, in: app)
                XCTAssertEqual(delivered.count, 1)
                XCTAssertEqual(delivered.firstMatch.frame.minX, pendingX, accuracy: 2,
                               "Confirmation must not shift text horizontally")
            } catch {
                try await holdSends(false)
                throw error
            }
        }
    }

    func testAccountCanSendExactlyOneMessageAndComposerClears() async throws {
        let app = launch()
        // The first cold Intel launch can still be opening its account space
        // after 10 seconds. Keep the exact content assertion, but allow startup
        // to finish before testing the separate send/delivery deadlines below.
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app, timeout: 30)
        let composer = app.descendants(matching: .any)["message-composer"]
        XCTAssertTrue(composer.exists)
        let message = "Native parity send \(UUID().uuidString)"
        #if os(iOS)
        // The multiline SwiftUI field can be AX-visible before UIKit grants
        // first responder. Tap its actual center and require keyboard focus.
        composer.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        if !app.keyboards.firstMatch.waitForExistence(timeout: 2) {
            composer.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        }
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 3))
        #else
        composer.tap()
        #endif
        composer.typeText(message)
        XCTAssertEqual(composer.value as? String, message)
        let send = app.buttons["send-message-button"]
        XCTAssertTrue(send.isEnabled)
        #if os(macOS)
        // Click the visible button away from its small arrow glyph. A center
        // tap cannot catch a label whose styled padding is not hit-testable.
        send.coordinate(withNormalizedOffset: CGVector(dx: 0.18, dy: 0.75)).click()
        #else
        send.tap()
        #endif

        let delivered = staticTexts(message, in: app)
        XCTAssertTrue(delivered.firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(delivered.count, 1, "HTTP confirmation and gateway delivery must merge into one message")
        let cleared = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == ''"), object: composer)
        XCTAssertEqual(XCTWaiter.wait(for: [cleared], timeout: 5), .completed)
        #if os(iOS)
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2)
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2,
                       "Sending must leave a compact composer background")
        #endif

        // An optimistic row and an empty composer are not proof of delivery.
        // Verify the exact content was accepted once by the local fixture API.
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/chat/channels/chan00000001/messages")!,
                                 cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 5)
        request.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        var stored = 0
        for _ in 0..<50 {
            let (data, response) = try await URLSession.shared.data(for: request)
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
            let history = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            let messages = try XCTUnwrap(history["messages"] as? [[String: Any]])
            stored = messages.filter { ($0["content"] as? [String: Any])?["text"] as? String == message }.count
            if stored > 0 { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertEqual(stored, 1, "Send must reach the server exactly once, not merely draw a pending row")
    }

    func testChatReceivesAfterDisconnectWithoutStaleReconnectWarning() async throws {
        let app = launch()
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app, timeout: 30)
        var sessionRequest = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/chat/session")!)
        sessionRequest.httpMethod = "POST"
        sessionRequest.setValue("application/json", forHTTPHeaderField: "content-type")
        sessionRequest.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        sessionRequest.httpBody = Data(#"{"name":"Fixture Owner"}"#.utf8)
        let (data, sessionResponse) = try await URLSession.shared.data(for: sessionRequest)
        XCTAssertEqual((sessionResponse as? HTTPURLResponse)?.statusCode, 200)
        let session = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let token = try XCTUnwrap(session["token"] as? String)
        func postMessage(_ text: String) async throws {
            var send = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/chat/channels/chan00000001/messages")!)
            send.httpMethod = "POST"
            send.setValue("application/json", forHTTPHeaderField: "content-type")
            send.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
            send.setValue(token, forHTTPHeaderField: "x-caper-chat-token")
            send.httpBody = try JSONSerialization.data(withJSONObject: ["clientMessageId": UUID().uuidString, "text": text])
            let (_, sent) = try await URLSession.shared.data(for: send)
            XCTAssertEqual((sent as? HTTPURLResponse)?.statusCode, 200)
        }
        let initial = "TEST FIXTURE initial live delivery \(UUID().uuidString)"
        try await postMessage(initial)
        assertStaticText(initial, in: app, timeout: 15)
        // Receiving an externally posted message proves the initial socket is
        // subscribed before forcing a failure, rather than racing startup.
        var control = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        control.httpMethod = "POST"
        control.setValue("application/json", forHTTPHeaderField: "content-type")
        control.httpBody = Data(#"{"disconnect":true}"#.utf8)
        let (_, disconnected) = try await URLSession.shared.data(for: control)
        XCTAssertEqual((disconnected as? HTTPURLResponse)?.statusCode, 200)
        let message = "TEST FIXTURE reconnect delivery \(UUID().uuidString)"
        try await postMessage(message)

        // No app send or navigation can clear the warning. Only socket recovery
        // and replay/live delivery can make this externally posted message appear.
        assertStaticText(message, in: app, timeout: 15)
        XCTAssertEqual(staticTexts(message, in: app).count, 1)
        XCTAssertFalse(staticTexts("Live updates disconnected. Reconnecting…", in: app).firstMatch.exists)
        capture("chat-recovered-test-fixture", app: app)
    }

    func testVoiceEntryAndAudioPreferencesWithoutFeatureFlag() {
        let app = launch()
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app)
        XCTAssertFalse(app.buttons["join-voice-button"].exists, "Voice is not joined from the conversation header")
        XCTAssertFalse(app.descendants(matching: .any)["header-voice-people"].exists, "The conversation header shows no faces")
        #if os(iOS)
        // Nobody is in #general's voice and you are in no call: no strip.
        XCTAssertFalse(app.descendants(matching: .any)["voice-strip"].exists, "A quiet channel has no voice strip")
        app.buttons["Back to Browse"].tap()
        #endif
        // A quiet channel joins from its row's one-click headphones (shown on touch, revealed by hover with a pointer).
        let join = app.buttons["join-voice-chan00000001"]
        #if os(macOS)
        app.buttons["channel-chan00000001"].hover()
        #endif
        XCTAssertTrue(join.waitForExistence(timeout: 5))
        XCTAssertEqual(join.label, "Join voice in #general")
        let enabled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: join)
        XCTAssertEqual(XCTWaiter.wait(for: [enabled], timeout: 5), .completed,
                       "Normal launches must expose voice without a test-only environment flag")
        XCTAssertFalse(app.descendants(matching: .any)["voice-count-chan00000001"].exists, "An empty channel's row shows no voice count")
        capture("voice-ready", app: app)
        let microphone = app.buttons["microphone-toggle"]
        let headphones = app.buttons["deafen-toggle"]
        #if os(macOS)
        XCTAssertTrue(microphone.waitForExistence(timeout: 5))
        XCTAssertTrue(headphones.exists)
        XCTAssertEqual(microphone.value as? String, "On")
        microphone.tap()
        XCTAssertEqual(microphone.value as? String, "Muted")
        headphones.tap()
        XCTAssertEqual(headphones.value as? String, "Deafened")
        capture("audio-muted", app: app)
        headphones.tap()
        XCTAssertEqual(headphones.value as? String, "On")
        XCTAssertEqual(microphone.value as? String, "Muted", "Undeafen must preserve an explicitly muted microphone")
        microphone.tap()
        XCTAssertEqual(microphone.value as? String, "On")
        app.buttons["Input Options"].tap()
        XCTAssertTrue(app.sliders["Input volume"].waitForExistence(timeout: 2))
        XCTAssertFalse(app.sliders["Voice processing"].exists, "Web's input menu has only the device and input volume")
        capture("input-options", app: app)
        app.typeKey(.escape, modifierFlags: [])
        app.buttons["Output Options"].tap()
        XCTAssertTrue(app.sliders["Output volume"].waitForExistence(timeout: 2))
        capture("output-options", app: app)
        app.typeKey(.escape, modifierFlags: [])
        #else
        XCTAssertFalse(microphone.exists)
        XCTAssertFalse(headphones.exists)
        XCTAssertFalse(app.buttons["Input Options"].exists)
        XCTAssertFalse(app.buttons["Output Options"].exists)
        #endif
        let settings = app.descendants(matching: .any)["account-settings-menu"]
        XCTAssertTrue(settings.waitForExistence(timeout: 5))
        #if os(iOS)
        // SwiftUI's nested Menu Button is visible at the bottom of Browse,
        // but AX's scroll-to-visible can target an offscreen ancestor.
        let frame = settings.frame
        let window = app.windows.firstMatch.frame
        XCTAssertTrue(window.contains(CGPoint(x: frame.midX, y: frame.midY)))
        app.coordinate(withNormalizedOffset: CGVector(
            dx: frame.midX / window.width, dy: frame.midY / window.height
        )).tap()
        #else
        settings.tap()
        #endif
        #if os(macOS)
        XCTAssertFalse(app.descendants(matching: .any)["launch-at-login"].exists, "Startup must not appear in the quick menu")
        XCTAssertFalse(app.descendants(matching: .any)["sound-effects"].exists, "Persistent preferences belong in Settings")
        app.descendants(matching: .any)["open-settings"].tap()
        let startup = app.descendants(matching: .any)["launch-at-login"]
        XCTAssertTrue(startup.waitForExistence(timeout: 2))
        XCTAssertFalse(startup.isEnabled, "Fixture previews must not change real Login Items")
        XCTAssertTrue(app.descendants(matching: .any)["sound-effects"].exists)
        capture("startup-settings", app: app)
        app.typeKey("w", modifierFlags: .command)
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.descendants(matching: .any)["desktop-settings"])
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 3), .completed)
        settings.tap()
        #else
        XCTAssertFalse(app.descendants(matching: .any)["launch-at-login"].exists, "Startup is desktop-only")
        // iOS 26 builds menu items from UIKit actions, which may expose the
        // toggle by its title rather than its SwiftUI identifier.
        let soundEffects = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == %@ OR label == %@", "sound-effects", "Caper sound effects")).firstMatch
        XCTAssertTrue(soundEffects.waitForExistence(timeout: 5), "Web keeps Caper sound effects in the settings menu")
        #endif
        let preferences = app.descendants(matching: .any)["Audio test"]
        XCTAssertTrue(preferences.waitForExistence(timeout: 2))
        preferences.tap()

        XCTAssertTrue(app.descendants(matching: .any)["audio-preferences-sheet"].waitForExistence(timeout: 5))
        assertStaticText("Audio test", in: app, timeout: 2)
        assertStaticText("Only you can hear these tests.", in: app, timeout: 2)
        XCTAssertTrue(app.buttons["speaker-test"].exists)
        let gain = app.sliders["Test speaker volume"]
        XCTAssertTrue(gain.waitForExistence(timeout: 2))
        XCTAssertEqual(outputGain(of: gain), 100)
        assertStaticText("100%", in: app, timeout: 2)
        gain.adjust(toNormalizedSliderPosition: 0.75)
        guard let displayedGain = outputGain(of: gain) else {
            XCTFail("Output gain slider must expose its actual gain percentage")
            return
        }
        // XCUITest's slider drag lands only approximately on iOS simulators, so
        // accept a band around 150% and report what it actually selected.
        XCTAssertTrue((120...180).contains(displayedGain), "A 75% slider gesture should select approximately 150% of the 0–200% range (got \(displayedGain)%)")
        XCTAssertNotEqual(displayedGain, 100, "The gesture must change the gain")
        assertStaticText("\(displayedGain)%", in: app, timeout: 2)
        #if os(iOS)
        XCTAssertTrue(app.descendants(matching: .any)["system-audio-route-picker"].exists)
        XCTAssertTrue(app.sliders["Test microphone volume"].exists)
        XCTAssertTrue(app.sliders["Voice processing"].exists)
        XCTAssertTrue(app.buttons["local-mic-test"].exists)
        #else
        XCTAssertTrue(app.descendants(matching: .any)["audio-input-device"].exists)
        XCTAssertTrue(app.descendants(matching: .any)["audio-output-device"].exists)
        XCTAssertTrue(app.sliders["Test microphone volume"].exists)
        XCTAssertTrue(app.sliders["Voice processing"].exists)
        // XCTest's normalized drag stops inside the track, and typeKey does
        // not focus an NSSlider on runners with keyboard navigation disabled.
        // Grab the centered thumb and drag beyond the track to its real limit.
        let inputGain = app.sliders["Test microphone volume"]
        inputGain.adjust(toNormalizedSliderPosition: 0.5)
        // A slow drag that holds past the track's end: fast drags on the Intel
        // runner can release before AppKit tracks the final position.
        inputGain.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 0.1, thenDragTo: inputGain.coordinate(withNormalizedOffset: CGVector(dx: -0.5, dy: 0.5)),
                   withVelocity: .slow, thenHoldForDuration: 0.3)
        XCTAssertEqual(outputGain(of: app.sliders["Test microphone volume"]), 0)
        let strength = app.sliders["Voice processing"]
        strength.adjust(toNormalizedSliderPosition: 0.5)
        strength.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 0.1, thenDragTo: strength.coordinate(withNormalizedOffset: CGVector(dx: 1.5, dy: 0.5)),
                   withVelocity: .slow, thenHoldForDuration: 0.3)
        XCTAssertEqual(outputGain(of: app.sliders["Voice processing"]), 100)
        XCTAssertTrue(app.buttons["local-mic-test"].exists, "Prejoin mic test must be a deliberate action")
        #endif
        let controls = app.scrollViews["audio-preferences-controls"]
        #if os(iOS)
        controls.swipeUp()
        #else
        controls.scroll(byDeltaX: 0, deltaY: -600)
        #endif
        XCTAssertTrue(controls.frame.contains(app.buttons["local-mic-test"].frame))
        for removed in [
            "The voice contour runs before the sender; 0% bypasses the contour, not noise suppression.",
            "On-device noise suppression starts when you test or join.",
            "Caper routes this call to the selected devices without changing macOS system defaults.",
            "Choose an audio route",
        ] {
            XCTAssertFalse(staticTexts(removed, in: app).firstMatch.exists, "Normal settings must not expose extra explanatory copy")
        }
        XCTAssertFalse(staticTexts("Connection statistics", in: app).firstMatch.exists)
        capture("audio-preferences", app: app)
    }

    func testOwnMessageAuthorOpensProfileEditorRetainsRejectedValuesAndRetries() async throws {
        let app = launch()
        let row = try require(app.descendants(matching: .any)["message-row-chan00000001m01"], timeout: 30,
                              "Missing the seeded message from your account")
        let author = try require(row.buttons.matching(NSPredicate(format: "label == %@", "Open profile for Fixture Owner")).firstMatch,
                                 timeout: 5, "Your name in a chat message must open your profile")
        author.tap()
        let edit = try require(app.buttons["mention-card-edit-profile"], timeout: 5,
                               "Your own message's profile card must offer Edit profile")
        edit.tap()
        let username = app.textFields["Username"]
        let displayName = app.textFields["Display name"]
        let submit = app.buttons["profile-save"]
        XCTAssertTrue(username.waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any)["change-profile-picture"].exists)
        let savedUsername = try XCTUnwrap(username.value as? String)
        let originalName = try XCTUnwrap(displayName.value as? String)
        XCTAssertFalse(savedUsername.isEmpty)
        type(" UI edit", into: displayName)
        let editedName = try XCTUnwrap(displayName.value as? String)
        XCTAssertNotEqual(editedName, originalName)
        XCTAssertTrue(submit.isEnabled)
        var control = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        control.httpMethod = "POST"
        control.setValue("application/json", forHTTPHeaderField: "content-type")
        control.httpBody = Data(#"{"failure":{"path":"/api/account/profile","method":"POST","status":503,"error":"TEST FIXTURE: profile save temporarily unavailable."}}"#.utf8)
        let (_, response) = try await URLSession.shared.data(for: control)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        save(submit, from: displayName)
        assertStaticText("Your profile could not be saved. Please try again.", in: app)
        XCTAssertEqual(username.value as? String, savedUsername)
        XCTAssertEqual(displayName.value as? String, editedName)
        XCTAssertTrue(submit.isEnabled)
        capture("profile-rejected-save", app: app)
        save(submit, from: displayName)
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: submit)
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 10), .completed)
        // Keep the shared fixture's reference author stable for other UI cases.
        var restore = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/account/profile")!)
        restore.httpMethod = "POST"
        restore.setValue("application/json", forHTTPHeaderField: "content-type")
        restore.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        restore.httpBody = try JSONSerialization.data(withJSONObject: ["username": savedUsername, "displayName": originalName])
        let (_, restored) = try await URLSession.shared.data(for: restore)
        XCTAssertEqual((restored as? HTTPURLResponse)?.statusCode, 200)
    }

    func testProfileValidationAndErrorRenderWithoutSubmitting() {
        let app = launch(fixture: "profile-validation")
        let username = app.textFields["Username"]
        let displayName = app.textFields["Display name"]
        let submit = app.buttons["profile-continue"]
        XCTAssertTrue(username.waitForExistence(timeout: 5))
        assertStaticText("Choose how you show up.", in: app)
        XCTAssertEqual(submit.label, "Finish setup")
        XCTAssertFalse(submit.isEnabled)
        type("ab", into: username)
        #if os(iOS)
        // Return moves to Display name; on iPhone the keyboard covers that field,
        // so tapping it would hit the keyboard instead.
        username.typeText("\n")
        XCTAssertTrue(hasKeyboardFocus(displayName), "Return on Username must focus Display name")
        displayName.typeText("Fixture Name")
        #else
        type("Fixture Name", into: displayName)
        #endif
        XCTAssertFalse(submit.isEnabled, "two-letter usernames cannot submit")
        assertStaticText("TEST FIXTURE — username already taken. Choose another username.", in: app)
        capture("profile-validation", app: app)
        type("_user", into: username)
        XCTAssertTrue(submit.isEnabled)
    }

    func testRejectedMessageActionsRenderWithoutSending() async throws {
        let app = launch(fixture: "chat-rejected")
        assertStaticText("Fixture message that was rejected", in: app)
        let edit = app.buttons["Edit"]
        XCTAssertTrue(edit.waitForExistence(timeout: 5))
        XCTAssertTrue(edit.isEnabled)
        XCTAssertTrue(app.scrollViews["chat-timeline"].frame.contains(edit.frame), "Edit must be inside the visible chat viewport")
        XCTAssertTrue(app.buttons["Dismiss"].exists)
        XCTAssertFalse(app.buttons["send-message-button"].isEnabled)
        capture("chat-rejected-fixture", app: app)

        // Another message changing the history must not scroll to the last
        // committed row and hide the rejected row's Edit/Dismiss controls.
        var sessionRequest = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/chat/session")!)
        sessionRequest.httpMethod = "POST"
        sessionRequest.setValue("application/json", forHTTPHeaderField: "content-type")
        sessionRequest.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        sessionRequest.httpBody = Data(#"{"name":"Fixture Owner"}"#.utf8)
        let (data, sessionResponse) = try await URLSession.shared.data(for: sessionRequest)
        XCTAssertEqual((sessionResponse as? HTTPURLResponse)?.statusCode, 200)
        let session = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let token = try XCTUnwrap(session["token"] as? String)
        let message = "TEST FIXTURE live delivery while rejected \(UUID().uuidString)"
        var send = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/chat/channels/chan00000001/messages")!)
        send.httpMethod = "POST"
        send.setValue("application/json", forHTTPHeaderField: "content-type")
        send.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        send.setValue(token, forHTTPHeaderField: "x-caper-chat-token")
        send.httpBody = try JSONSerialization.data(withJSONObject: ["clientMessageId": UUID().uuidString.lowercased(), "text": message])
        let (_, sent) = try await URLSession.shared.data(for: send)
        XCTAssertEqual((sent as? HTTPURLResponse)?.statusCode, 200)
        assertStaticText(message, in: app, timeout: 15)
        XCTAssertTrue(edit.waitForExistence(timeout: 5))
        XCTAssertTrue(app.scrollViews["chat-timeline"].frame.contains(edit.frame), "Live delivery keeps rejected actions in the visible viewport")
        XCTAssertTrue(app.buttons["Dismiss"].exists)
        XCTAssertFalse(app.buttons["send-message-button"].isEnabled)
        capture("chat-rejected-after-live-fixture", app: app)
        edit.tap()
        XCTAssertFalse(app.buttons["Dismiss"].waitForExistence(timeout: 1))
        XCTAssertEqual(app.descendants(matching: .any)["message-composer"].value as? String,
                       "Fixture message that was rejected")
    }

    #if os(iOS)
    func testPhoneRecordedComparisonAndStatisticsFixtureWithoutCapture() {
        for (fixture, label, screenshot) in [
            ("audio-recorded", "TEST FIXTURE — completed local recording layout only; no microphone or playback.", "ios-audio-recorded-fixture"),
            ("audio-statistics", "TEST FIXTURE — synthetic statistics layout; no voice connection.", "ios-audio-statistics-fixture"),
        ] {
            let app = launch(fixture: fixture)
            app.buttons["Back to Browse"].tap()
            let settings = app.descendants(matching: .any)["account-settings-menu"]
            XCTAssertTrue(settings.waitForExistence(timeout: 5))
            let frame = settings.frame, window = app.windows.firstMatch.frame
            app.coordinate(withNormalizedOffset: CGVector(dx: frame.midX / window.width, dy: frame.midY / window.height)).tap()
            app.descendants(matching: .any)[fixture == "audio-recorded" ? "Audio test" : "Connection details"].tap()
            if fixture == "audio-recorded" {
                let controls = app.scrollViews["audio-preferences-controls"]
                XCTAssertTrue(controls.waitForExistence(timeout: 5))
                controls.swipeUp()
            }
            assertStaticText(label, in: app)
            if fixture == "audio-recorded" {
                for title in ["Play natural", "Play enhanced", "Stop playback"] {
                    let button = app.buttons[title]
                    XCTAssertTrue(button.exists)
                    XCTAssertFalse(button.isEnabled, "Fixture must not play synthetic audio")
                }
                XCTAssertTrue(app.sliders["Voice processing"].exists)
            } else {
                assertStaticText("Counters reset on reconnect.", in: app)
                assertStaticText("TURN relay", in: app)
            }
            capture(screenshot, app: app)
        }
    }
    #endif

    #if os(macOS)
    func testFailedChannelNavigationKeepsConversationAndDraftThenRetries() async throws {
        let app = launch()
        assertElement("selected-channel-name", label: "# general", in: app)
        let composer = app.descendants(matching: .any)["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        composer.tap(); composer.typeText("Keep this draft")
        var control = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        control.httpMethod = "POST"
        control.setValue("application/json", forHTTPHeaderField: "content-type")
        control.httpBody = Data(#"{"failure":{"path":"/api/spaces/space0000001","method":"GET","status":503,"persistent":true,"error":"TEST FIXTURE: space temporarily unavailable."}}"#.utf8)
        let (_, response) = try await URLSession.shared.data(for: control)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        app.buttons["channel-chan00000002"].tap()
        let retry = app.buttons["Retry opening"]
        XCTAssertTrue(retry.waitForExistence(timeout: 5))
        assertElement("selected-channel-name", label: "# general", in: app)
        XCTAssertEqual(composer.value as? String, "Keep this draft")
        capture("navigation-retry", app: app)
        control.httpBody = Data(#"{"clearFailures":true}"#.utf8)
        let (_, recovered) = try await URLSession.shared.data(for: control)
        XCTAssertEqual((recovered as? HTTPURLResponse)?.statusCode, 200)
        retry.tap()
        let design = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "# design"), object: app.descendants(matching: .any)["selected-channel-name"])
        XCTAssertEqual(XCTWaiter.wait(for: [design], timeout: 10), .completed)
        XCTAssertFalse(retry.exists)
    }
    #endif

    func testActionableLoginError() async throws {
        var control = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        control.httpMethod = "POST"
        control.setValue("application/json", forHTTPHeaderField: "content-type")
        control.httpBody = Data(#"{"failure":{"path":"/api/auth/email/request","method":"POST","status":503,"error":"TEST FIXTURE: email service unavailable."}}"#.utf8)
        let (_, response) = try await URLSession.shared.data(for: control)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)

        let app = launch(fixture: "login", signedIn: false)
        let email = app.textFields["Email address"]
        XCTAssertTrue(email.waitForExistence(timeout: 10))
        email.tap()
        email.typeText("owner@example.test")
        app.buttons["Email me a code"].tap()
        assertStaticText("Sign-in is temporarily unavailable. Please try again later.", in: app, timeout: 5)
        capture("login-error", app: app)
    }

    /// Blocks an account through the fixture as the signed-in owner.
    private nonisolated static func fixtureBlock(_ accountID: String) async throws {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/blocks/\(accountID)")!)
        request.httpMethod = "PUT"
        request.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "authorization")
        let (_, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 204)
    }

    func testBlockedAuthorsCollapseUntilShown() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        // Maya wrote the second and third seeded messages in #general.
        try await Self.fixtureBlock("member000001")
        let app = launch()
        let toggle = try require(app.buttons["blocked-run-toggle"], timeout: 30, "Maya's messages must collapse into one run")
        XCTAssertEqual(toggle.label, "Show 2 blocked messages")
        XCTAssertFalse(app.descendants(matching: .any)["message-row-chan00000001m02"].exists)
        capture("blocked-messages-collapsed", app: app)
        toggle.tap()
        _ = try require(app.descendants(matching: .any)["message-row-chan00000001m02"], timeout: 5, "Show must reveal the run in place")
        XCTAssertTrue(app.descendants(matching: .any)["message-row-chan00000001m03"].exists)
        XCTAssertEqual(app.buttons["blocked-run-toggle"].label, "Hide 2 blocked messages")
    }

    #if os(macOS)
    func testMessageRequestOpensReadOnlyAndDeclines() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        try await Self.fixtureControl(["messageRequest": [String: String]()])
        let app = launch()
        try require(app.buttons["message-requests"], timeout: 30, "An incoming request adds the Message requests row").tap()
        try require(app.buttons["message-request-dm0000000003"], timeout: 5, "The list shows Jordan's request").tap()
        _ = try require(app.descendants(matching: .any)["message-request-bar"], timeout: 10, "A request replaces the composer")
        XCTAssertFalse(app.descendants(matching: .any)["message-composer"].exists, "Requests are read-only")
        capture("message-request-bar", app: app)
        app.buttons["message-request-decline"].tap()
        let gone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.buttons["message-requests"])
        XCTAssertEqual(XCTWaiter.wait(for: [gone], timeout: 10), .completed, "Declining removes the last request")
    }
    #endif

    func testNewDirectMessageRefusalStaysInStableDialog() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        let message = "This person isn't accepting direct messages."
        try await Self.fixtureControl(["failure": ["path": "/api/dms", "method": "POST", "status": 403, "error": message]])
        let app = launch(fixture: "new-direct-message")
        let username = try require(app.textFields["dm-username"], timeout: 30, "The new-DM dialog must open")
        type("jordan", into: username)
        let submit = app.buttons["dm-create-submit"]
        let fieldFrame = username.frame, buttonFrame = submit.frame
        submit.tap()
        let failure = try require(app.staticTexts["dm-create-error"], timeout: 10, "The refusal must appear in the dialog")
        assertElement("dm-create-error", label: message, in: app)
        XCTAssertEqual(username.value as? String, "jordan")
        XCTAssertTrue(submit.isEnabled, "A refusal must leave the form retryable")
        XCTAssertEqual(username.frame, fieldFrame, "The error must not move the username field")
        XCTAssertEqual(submit.frame, buttonFrame, "The error must not move or resize the action")
        capture("new-dm-refused", app: app)
        type("_other", into: username)
        let cleared = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: failure)
        XCTAssertEqual(XCTWaiter.wait(for: [cleared], timeout: 3), .completed)
        XCTAssertEqual(submit.frame, buttonFrame, "Editing must not collapse the error's space")
        app.buttons["Close"].firstMatch.tap()
        XCTAssertFalse(app.staticTexts.matching(NSPredicate(format: "label == %@ OR value == %@", message, message)).firstMatch.exists,
                       "The refusal must not leak into the sidebar after closing")
    }

    func testNewDirectMessageSuggestionFillsUsernameWithoutOpening() throws {
        let app = launch(fixture: "new-direct-message")
        let username = try require(app.textFields["dm-username"], timeout: 30, "The new-DM dialog must open")
        type("ma", into: username)
        // GET /api/people: the fixture space's Maya and Alex, never you.
        let maya = try require(app.buttons["person-suggestion-maya"], timeout: 5, "Typing ma must suggest maya")
        XCTAssertFalse(app.buttons["person-suggestion-alex"].exists, "Only matching people are suggested")
        capture("new-dm-suggestions", app: app)
        maya.tap()
        let filled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "maya"), object: username)
        XCTAssertEqual(XCTWaiter.wait(for: [filled], timeout: 3), .completed, "Picking fills in the exact username")
        XCTAssertFalse(app.buttons["person-suggestion-maya"].exists, "The list closes after a pick")
        XCTAssertTrue(app.buttons["dm-create-submit"].isEnabled, "Picking must not open the conversation")
        XCTAssertTrue(username.exists)
    }

    func testManageSpace() {
        let app = launch(fixture: "manage-space")
        assertStaticText("Manage space", in: app)
        assertElement("space-members-heading", label: "Members 3", in: app)
        capture("manage-space", app: app)
        let username = app.textFields["Exact username"]
        XCTAssertTrue(username.isEnabled, "The dialog must not inherit the disabled workspace")
        let remove = app.buttons["Remove"].firstMatch
        #if os(macOS)
        // The space icon section above can push members below the fold, and a
        // Mac click is not scrolled into view.
        for _ in 0..<3 where !remove.isHittable {
            app.scrollViews["space-settings-scroll"].scroll(byDeltaX: 0, deltaY: -200)
        }
        #endif
        remove.tap()
        // Removal asks first, naming the person and the space.
        let confirmRemove = app.buttons["confirm-destructive-action"]
        XCTAssertTrue(confirmRemove.waitForExistence(timeout: 5), "Remove must ask for confirmation")
        assertStaticText("Remove Maya?", in: app)
        assertStaticText("They’ll lose access to Fixture Studio and its channels. You can invite them again later.", in: app)
        capture("remove-member-confirmation", app: app)
        confirmRemove.tap()
        let confirmClosed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: confirmRemove)
        XCTAssertEqual(XCTWaiter.wait(for: [confirmClosed], timeout: 5), .completed, "The confirmation closes after removing")
        #if os(macOS)
        let headingProperty = "value"
        #else
        let headingProperty = "label"
        #endif
        let twoMembers = XCTNSPredicateExpectation(predicate: NSPredicate(format: "%K == %@", headingProperty, "Members 2"),
            object: app.descendants(matching: .any)["space-members-heading"])
        XCTAssertEqual(XCTWaiter.wait(for: [twoMembers], timeout: 5), .completed)
        // Removed members have a 24-hour invitation cooldown. Invite an
        // existing fixture account that has never belonged to this space.
        type("sam", into: username)
        #if os(iOS)
        capture("manage-space-keyboard", app: app)
        username.typeText("\n")
        #else
        app.buttons["Invite"].tap()
        #endif
        let unchanged = XCTNSPredicateExpectation(predicate: NSPredicate(format: "%K == %@", headingProperty, "Members 2"),
            object: app.descendants(matching: .any)["space-members-heading"])
        XCTAssertEqual(XCTWaiter.wait(for: [unchanged], timeout: 5), .completed,
                       "Inviting must not grant immediate space membership")
        assertStaticText("Pending invitations  1", in: app)
        assertStaticText("Invitation sent. They must accept before joining.", in: app)
        assertStaticText("Sam", in: app)
        assertStaticText("@sam", in: app)
        XCTAssertTrue(username.value as? String == "" || username.value as? String == username.placeholderValue,
            "Successful invitation clears the editable field")
        capture("manage-space-pending-invitation", app: app)
        app.buttons["Close"].firstMatch.tap()
        XCTAssertFalse(app.textFields["Exact username"].exists)
        XCTAssertTrue(app.buttons["Back to Browse"].exists || app.buttons["account-profile"].isHittable)
    }

    func testManageSpaceCanDismissOnOutsideTap() {
        let app = launch(fixture: "manage-space")
        XCTAssertTrue(app.textFields["Exact username"].waitForExistence(timeout: 10))
        #if os(macOS)
        app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.02, dy: 0.5)).click()
        #else
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.02, dy: 0.5)).tap()
        #endif
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.textFields["Exact username"])
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 3), .completed)
    }

    func testPrivateChannelOverview() {
        let app = launch(fixture: "manage-channel")
        assertStaticText("Channel settings", in: app)
        assertStaticText("Private channel", in: app, timeout: 2)
        capture("private-channel-overview", app: app)
    }

    #if os(iOS)
    /// The conversation moves with the finger, so a swipe lifts on the control
    /// it started on: opening Browse from a mention or an author must not also
    /// open their card (as swiping from an image opened the media viewer).
    func testSwipeFromMessageControlsDoesNotTapThem() throws {
        let app = launch()
        let row = try require(app.descendants(matching: .any)["message-row-chan00000001m04"], timeout: 30,
                              "Missing the seeded message that mentions you")
        let pill = try require(row.links.matching(NSPredicate(format: "label CONTAINS %@", "fixture_owner")).firstMatch,
                               timeout: 5, "The @fixture_owner pill must be a link")
        let author = row.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Open profile for")).firstMatch
        let close = app.buttons["Close navigation"]
        let navigation = app.buttons["Back to Browse"]
        for control in [pill, author] where control.exists {
            let start = control.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            start.press(forDuration: 0.01, thenDragTo: start.withOffset(CGVector(dx: app.frame.width * 0.8, dy: 0)),
                        withVelocity: .fast, thenHoldForDuration: 0)
            XCTAssertTrue(close.waitForExistence(timeout: 3), "Swiping right from a message control opens Browse")
            XCTAssertFalse(app.descendants(matching: .any)["mention-card"].waitForExistence(timeout: 1),
                           "Lifting a swipe must not open the card under the finger")
            // Browse fills the phone; swiping left anywhere on it returns.
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.6, dy: 0.5)).press(forDuration: 0.01,
                thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: 0.5)), withVelocity: .fast, thenHoldForDuration: 0)
            XCTAssertTrue(navigation.waitForExistence(timeout: 3), "Swipe left from Browse's center returns to chat")
        }
        // A plain tap still opens it, once the swipe has lifted. By coordinate:
        // hosted simulators can fail the link's scroll-to-visible after swipes.
        Thread.sleep(forTimeInterval: 0.6)
        pill.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        _ = try require(app.descendants(matching: .any)["mention-card"], timeout: 5, "Tapping a pill must open the mention card")
    }

    /// The phone member list keeps its contents until it has slid away.
    private func assertMembersClosed(in app: XCUIApplication, _ message: String) {
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: staticTexts("Members", in: app).firstMatch)
        XCTAssertEqual(XCTWaiter().wait(for: [closed], timeout: 3), .completed, message)
        XCTAssertEqual(staticTexts("Members", in: app).count, 0)
    }

    /// Phones: an occupied channel you are not in offers Join in a strip under
    /// its header, and the room's card in Browse offers the same room.
    func testPhoneStripOffersJoinInAnOccupiedChannel() throws {
        let app = launch()
        try require(app.buttons["Back to Browse"], timeout: 10, "Phones open a conversation first").tap()
        // The room's card in Browse, in its row: touch sizes, with the room's Join.
        let roomJoin = try require(app.buttons["live-join-chan00000002"], timeout: 10, "The occupied room's row has its card")
        XCTAssertGreaterThanOrEqual(roomJoin.frame.height, 44)
        XCTAssertEqual(roomJoin.label, "Join voice in #design")
        app.buttons["channel-chan00000002"].tap()
        assertElement("channel-title-menu", label: "#design options", in: app)
        let strip = try require(app.descendants(matching: .any)["voice-strip"], timeout: 10, "The occupied channel shows a strip")
        XCTAssertEqual(strip.frame.height, 56, accuracy: 1)
        assertElement("voice-strip-count", label: "2 talking", in: app)
        let join = app.buttons["voice-strip-join"]
        XCTAssertTrue(join.exists)
        XCTAssertEqual(join.label, "Join voice in #design")
        XCTAssertGreaterThanOrEqual(join.frame.height, 44)
        XCTAssertFalse(app.buttons["voice-strip-mute"].exists, "Not in a call: the strip has no Mute")
        capture("voice-strip-join-test-fixture", app: app)
    }

    func testNarrowConversationAndBrowse() throws {
        let app = launch()
        let navigation = app.buttons["Back to Browse"]
        XCTAssertTrue(navigation.waitForExistence(timeout: 10))
        assertStaticText("Fixture Owner", in: app)
        let channel = app.descendants(matching: .any)["selected-channel-name"]
        XCTAssertGreaterThan(channel.frame.minX, navigation.frame.maxX, "Back arrow leads the channel menu")
        XCTAssertGreaterThanOrEqual(navigation.frame.width, 44, "Keep the back touch target accessible")
        // Phones: the name opens the conversation's menu, as in Slack; Pins and Members move there.
        XCTAssertFalse(app.buttons["channel-pins"].exists, "Pins lives in the phone title menu")
        XCTAssertFalse(app.buttons["Show member list"].exists, "Members lives in the phone title menu")
        XCTAssertFalse(app.descendants(matching: .any)["channel-more-menu"].exists, "There is no separate More button")
        let title = app.descendants(matching: .any)["channel-title-menu"]
        XCTAssertTrue(title.isHittable, "The phone header's title is the conversation menu")
        XCTAssertEqual(title.label, "#general options")
        XCTAssertGreaterThan(title.frame.minX, navigation.frame.maxX - 1, "Back arrow leads the title menu")
        XCTAssertGreaterThanOrEqual(title.frame.height, 44)
        // The phone header carries no voice: Browse's live cards and the strip beneath the header do.
        XCTAssertFalse(app.buttons["join-voice-button"].exists, "Phones do not join voice from the conversation header")
        XCTAssertFalse(app.buttons["header-mute"].exists, "The header has no microphone toggle")
        XCTAssertFalse(app.buttons["header-deafen"].exists, "The header has no deafen toggle")
        XCTAssertFalse(app.buttons["header-leave-voice"].exists, "The header has no Leave")
        XCTAssertFalse(app.descendants(matching: .any)["voice-strip"].exists, "#general is quiet and you are in no call: no strip")
        XCTAssertGreaterThan(channel.frame.width, 0, "The channel name keeps its room")
        // Below the header is only the conversation: the account bar stays in Browse.
        let accountProfile = app.buttons["account-profile"]
        XCTAssertFalse(accountProfile.exists && accountProfile.isHittable, "The account bar stays in Browse")
        let accountMicrophone = app.buttons["microphone-toggle"]
        XCTAssertFalse(accountMicrophone.exists && accountMicrophone.isHittable, "Account audio controls stay in Browse")
        let composer = app.descendants(matching: .any)["message-composer"]
        let timeline = app.descendants(matching: .any)["chat-timeline"]
        let send = app.buttons["send-message-button"]
        let restingBottom = send.frame.maxY
        let restingTimeline = timeline.frame
        var draft = "Draft survives Browse"
        composer.tap(); composer.typeText(draft)
        // Screen positions inside the timeline, above the keyboard: with the
        // software keyboard up, hosted simulators re-resolve the timeline's frame
        // during a drag, which moved these drags' end points.
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.3))
        start.press(forDuration: 0.01, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.16)), withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(navigation.isHittable, "A vertical scroll must stay in chat")
        XCTAssertEqual(staticTexts("Members", in: app).count, 0, "A vertical scroll must not open the member list")
        // The vertical scroll dismisses editing. Refocus explicitly so it cannot
        // mask a stale keyboard inset when the next horizontal swipe opens Browse.
        composer.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(try keyboardTop(in: app) - send.frame.maxY, 16, accuracy: 2)
        // A short, held header drag snaps back without scrolling the timeline.
        // It must neither dismiss editing nor disturb the keyboard-sized layout.
        let header = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: app.frame.width * 0.3, dy: title.frame.midY))
        header.press(forDuration: 0.01, thenDragTo: header.withOffset(CGVector(dx: 32, dy: 0)),
                     withVelocity: .slow, thenHoldForDuration: 0.2)
        XCTAssertTrue(navigation.isHittable, "A cancelled swipe stays in the conversation")
        XCTAssertTrue(app.keyboards.firstMatch.exists, "A cancelled header swipe keeps the keyboard")
        // Type without retapping to check retained focus through the public UI.
        app.typeText("!")
        draft += "!"
        XCTAssertEqual(composer.value as? String, draft)
        XCTAssertEqual(try keyboardTop(in: app) - send.frame.maxY, 16, accuracy: 2)
        start.press(forDuration: 0.01, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: 0.3)), withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(app.buttons["Close navigation"].waitForExistence(timeout: 3), "Swipe right from the timeline center opens Browse")
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.keyboards.firstMatch)
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed, "Committed Browse navigation dismisses the keyboard")
        // Browse fills the phone. Its list's "channel-browser" identifier is
        // intermittently missing from hosted simulators' snapshots, so swipe on the screen.
        func swipeBrowseLeft(at height: CGFloat) {
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.6, dy: height)).press(forDuration: 0.01,
                thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: height)), withVelocity: .fast, thenHoldForDuration: 0)
        }
        swipeBrowseLeft(at: 0.5)
        XCTAssertTrue(navigation.waitForExistence(timeout: 3), "Swipe left from Browse's center returns to chat")
        // Observe past the delayed focus-release window as well as the slide.
        XCTAssertFalse(app.keyboards.firstMatch.waitForExistence(timeout: 1.5), "Returning must not reopen the keyboard")
        XCTAssertEqual(send.frame.maxY, restingBottom, accuracy: 2, "Returning must restore the bottom edge without a keyboard-sized gap")
        XCTAssertEqual(timeline.frame.minY, restingTimeline.minY, accuracy: 2)
        XCTAssertEqual(timeline.frame.height, restingTimeline.height, accuracy: 2, "The timeline must recover its full height")
        XCTAssertEqual(composer.value as? String, draft)
        capture("narrow-chat-after-keyboard-swipe", app: app)
        composer.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(try keyboardTop(in: app) - send.frame.maxY, 16, accuracy: 2, "Tapping the returned composer must still dock above the keyboard")
        capture("narrow-chat-keyboard-reopened", app: app)
        // A swipe along a full-width Browse row lifts inside that row: it must
        // navigate without also tapping it (which opens New message).
        navigation.tap()
        let row = app.buttons["new-message"]
        XCTAssertTrue(row.waitForExistence(timeout: 3))
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).press(forDuration: 0.01,
            thenDragTo: row.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.5)), withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(navigation.waitForExistence(timeout: 3), "Swiping along a Browse row returns to chat")
        XCTAssertFalse(app.textFields["dm-username"].waitForExistence(timeout: 1), "Lifting a swipe must not tap the row under the finger")
        XCTAssertTrue(navigation.isHittable, "No dialog opens over the conversation after a swipe")
        navigation.tap()
        app.descendants(matching: .any)["selected-space-name"].tap()
        app.buttons["Browse channels"].tap()
        let search = app.textFields["Search channels"]
        XCTAssertTrue(search.waitForExistence(timeout: 3))
        search.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5)).press(forDuration: 0.01,
            thenDragTo: search.coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: 0.5)), withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(app.buttons["Close navigation"].isHittable, "The search editor keeps its horizontal gestures")
        // Below the search field, above the account bar.
        swipeBrowseLeft(at: 0.6)
        XCTAssertTrue(navigation.waitForExistence(timeout: 3), "Channel-search content can still swipe back to chat")
        XCTAssertEqual(composer.value as? String, draft)
        capture("narrow-conversation", app: app)
        chooseFromTitleMenu("Members", in: app)
        assertStaticText("Members", in: app, timeout: 2)
        capture("narrow-members", app: app)
        app.buttons["Close member list"].tap()
        // The list keeps its contents while it slides away.
        assertMembersClosed(in: app, "Close member list closes the member list")
        // Members uses the same center-start gestures as Browse.
        timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(forDuration: 0.01,
            thenDragTo: timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: 0.5)), withVelocity: .fast, thenHoldForDuration: 0)
        assertStaticText("Members", in: app, timeout: 3)
        XCTAssertTrue(navigation.isHittable, "Swiping in the member list keeps the conversation, not Browse")
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.5)).press(forDuration: 0.01,
            thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)), withVelocity: .fast, thenHoldForDuration: 0)
        assertMembersClosed(in: app, "Swiping the backdrop right closes the member list")
        chooseFromTitleMenu("Pins", in: app)
        XCTAssertTrue(app.descendants(matching: .any)["pinned-messages"].waitForExistence(timeout: 3))
        assertStaticText("No pinned messages", in: app, timeout: 2)
        capture("narrow-empty-pins", app: app)
        app.buttons["Close pins"].tap()
        XCTAssertTrue(navigation.waitForExistence(timeout: 3))
        XCTAssertTrue(title.waitForExistence(timeout: 3), "Dismissing Pins restores the header")
        XCTAssertEqual(composer.value as? String, draft)
        chooseFromTitleMenu("Members", in: app)
        assertStaticText("Members", in: app, timeout: 2)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.02, dy: 0.5)).tap()
        assertMembersClosed(in: app, "Tapping the backdrop closes the member list")
        navigation.tap()
        assertElement("selected-space-name", label: "Fixture Studio", in: app, timeout: 5)
        let profile = app.buttons["account-profile"]
        let settings = app.descendants(matching: .any)["account-settings-menu"]
        XCTAssertTrue(profile.isHittable)
        XCTAssertTrue(settings.isHittable)
        XCTAssertLessThan(profile.frame.minX, 60, "The mobile account bar spans beneath the rail")
        XCTAssertEqual(profile.frame.midY, settings.frame.midY, accuracy: 3)
        XCTAssertLessThan(settings.frame.maxY, app.frame.maxY - 16, "Account controls must clear the home indicator")
        capture("narrow-browse", app: app)
    }
    #endif
}
