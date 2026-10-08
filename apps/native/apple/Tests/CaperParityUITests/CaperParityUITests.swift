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
        #if arch(arm64)
        let desktop = size.width >= 1_400
        let layout = desktop ? "desktop" : "medium"
        XCTAssertGreaterThanOrEqual(size.width, desktop ? 1_400 : 980, "Hosted ARM medium capture requires at least 980 points of width")
        XCTAssertGreaterThanOrEqual(size.height, desktop ? 880 : 640, "Hosted ARM capture is too short for its declared layout")
        #else
        let layout = "desktop"
        XCTAssertGreaterThanOrEqual(size.width, 1_400, "Intel desktop parity capture requires a 1,400-point-wide app window")
        XCTAssertGreaterThanOrEqual(size.height, 880, "Intel desktop parity capture requires an app window close to the 1,440×900 reference")
        #endif
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

    private func assertElement(_ identifier: String, label: String, in app: XCUIApplication, timeout: TimeInterval = 10) {
        let element = app.descendants(matching: .any)[identifier]
        XCTAssertTrue(element.waitForExistence(timeout: timeout), "Missing accessibility identifier \(identifier)")
        #if os(macOS)
        if element.elementType == .staticText { XCTAssertEqual(element.value as? String, label) }
        else { XCTAssertEqual(element.label, label) }
        #else
        XCTAssertEqual(element.label, label)
        #endif
    }

    private func staticTexts(_ text: String, in app: XCUIApplication) -> XCUIElementQuery {
        #if os(macOS)
        app.staticTexts.matching(NSPredicate(format: "value == %@", text))
        #else
        app.staticTexts.matching(NSPredicate(format: "label == %@", text))
        #endif
    }

    private func dismissAudioMenu(in app: XCUIApplication) {
        #if os(macOS)
        app.typeKey(.escape, modifierFlags: [])
        #else
        // The popover blocks its anchor; tap outside it as a person would.
        let outside = app.otherElements["PopoverDismissRegion"]
        XCTAssertTrue(outside.exists)
        outside.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6)).tap()
        wait(for: [expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: outside)], timeout: 3)
        #endif
    }

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
        row.hover()
        let add = try require(app.buttons["add-reaction-\(messageID)"], timeout: 10,
                              "Hovering \(messageID) did not reveal Add reaction")
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
        let pill = try require(row.links.matching(NSPredicate(format: "label CONTAINS %@", "fixture_owner")).firstMatch,
                               timeout: 5, "The @fixture_owner pill must be a link")
        pill.tap()
        _ = try require(app.descendants(matching: .any)["mention-card"], timeout: 5, "Tapping a pill must open the mention card")
        XCTAssertTrue(app.descendants(matching: .any)["mention-card-you"].waitForExistence(timeout: 2), "Your own card says You")
        XCTAssertFalse(app.buttons["mention-card-message"].exists, "Your own card has no Message button")
        capture("mention-card-self", app: app)
    }
    #if os(iOS)
    func testIPhoneComposerUsesContentHeightAndShrinksAfterEditing() throws {
        let app = launch()
        let composer = try require(app.descendants(matching: .any)["message-composer"], timeout: 30,
                                   "Missing message composer")
        let send = app.buttons["send-message-button"]
        XCTAssertTrue(focus(composer))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 3))
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2)
        // The text view already measured 42 points before the fix. Its bottom
        // must align with Send too, or its background can still occupy 174.
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        capture("composer-empty-keyboard", app: app)

        let short = "Compact draft"
        composer.typeText(short)
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2)
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        XCTAssertLessThanOrEqual(send.frame.maxY, app.keyboards.firstMatch.frame.minY)
        capture("composer-single-line-keyboard", app: app)

        let multiline = "\nSecond line\nThird line"
        composer.typeText(multiline)
        XCTAssertEqual(composer.value as? String, short + multiline)
        XCTAssertGreaterThan(composer.frame.height, 62, "The fix must not freeze the composer at one line")
        XCTAssertLessThan(composer.frame.height, 174, "A three-line draft must not jump straight to the cap")
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        capture("composer-multiline-keyboard", app: app)

        let overflow = String(repeating: "\nmore", count: 16)
        composer.typeText(overflow)
        XCTAssertEqual(composer.value as? String, short + multiline + overflow,
                       "Scrolling must retain lines beyond the visible height")
        XCTAssertEqual(composer.frame.height, 174, accuracy: 2)
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        XCTAssertTrue(send.isHittable, "Send must remain available above the keyboard at the height cap")
        capture("composer-capped-keyboard", app: app)

        composer.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: multiline.count + overflow.count))
        XCTAssertEqual(composer.value as? String, short)
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2, "Removing extra lines must shrink the composer")
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
        composer.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: short.count))
        XCTAssertEqual(composer.value as? String, "")
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2)
        XCTAssertEqual(composer.frame.maxY, send.frame.maxY, accuracy: 2)
    }

    func testIPhoneThreadComposerDoesNotFillItsMaximumHeight() throws {
        let app = launch()
        let row = try require(app.descendants(matching: .any)["message-row-chan00000001m01"], timeout: 30,
                              "Missing fixture message")
        hold(row)
        try require(app.buttons["Reply in thread"], timeout: 5, "Missing thread action").tap()
        try require(app.buttons["Back to channel"], timeout: 5, "Thread did not open")
        let composer = try require(app.descendants(matching: .any)["message-composer"], timeout: 5,
                                   "Missing thread composer")
        XCTAssertTrue(focus(composer))
        composer.typeText("Short reply")
        let send = app.buttons["Send reply"]
        XCTAssertEqual(composer.frame.height, 42, accuracy: 2)
        // Keep the existing 72-point thread minimum, without the 66-point gap
        // below the text view that a 174-point background would introduce.
        XCTAssertLessThan(send.frame.minY - composer.frame.maxY, 32)
        XCTAssertTrue(send.isHittable)
        capture("thread-composer-single-line-keyboard", app: app)
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
        target.hover()
        XCTAssertTrue(app.buttons["add-reaction-\(hoverTargetID)"].waitForExistence(timeout: 2), "Message controls appear on hover")
        #else
        XCTAssertEqual(app.buttons.matching(NSPredicate(format: "label == %@", "Add reaction")).count, 0, "iPhone must not show an add-reaction button under each message")
        #endif
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
        try openReactionPicker(for: targetID, in: app)
        let search = app.textFields["reaction-picker-search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5))
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
        unrelatedRow.hover()
        let unrelatedAdd = try require(app.buttons["add-reaction-chan00000001m02"], timeout: 5,
                                       "Hovering another message did not reveal Add reaction")

        // Accessibility focus alone cannot detect the bug: the old modifier
        // painted inherited timeline focus around every otherwise valid button.
        let window = app.windows.firstMatch
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: window.screenshot().pngRepresentation))
        let scaleX = CGFloat(bitmap.pixelsWide) / window.frame.width
        let scaleY = CGFloat(bitmap.pixelsHigh) / window.frame.height
        func hasTerracottaOutline(_ element: XCUIElement) throws -> Bool {
            XCTAssertTrue(element.isHittable)
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
        // "rockets" appears while searching, so app.buttons["rockets"] matches
        // twice and tapping it fails.
        let rockets = try require(app.scrollViews["reaction-picker-grid"].buttons["rockets"], timeout: 5,
                                  "Searching rocket never showed the rockets emoji")

        // Push the presenting row out of the lazy timeline's viewport
        // while the picker is open and its keyboard has focus.
        var lastText = ""
        for index in 0..<12 {
            lastText = "Picker live scroll \(index)\n" + String(repeating: "TEST FIXTURE timeline layout change.\n", count: 8)
            try await Self.fixtureControl(["incomingMessage": ["channelId": "chan00000001", "text": lastText]])
            try require(search, timeout: 2, "Live delivery must not dismiss the reaction picker")
            XCTAssertEqual(search.value as? String, "rocket", "Live delivery must not reset the search")
            XCTAssertTrue(hasKeyboardFocus(search), "Live scrolling must not replace the sheet")
        }
        capture("reaction-picker-focused-after-live-scroll-fixture", app: app)
        try require(rockets, timeout: 2, "Live delivery must not drop the rockets search result")
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
        assertStaticText("Members", in: app, timeout: 2)
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
        XCTAssertTrue(generalVoice.exists, "An empty accessible channel keeps its quiet voice action")
        XCTAssertTrue(designVoice.exists, "An unselected accessible channel keeps its quiet voice action")
        XCTAssertEqual(generalVoice.label, "Join voice in #general")
        XCTAssertEqual(designVoice.label, "Join voice in #design")
        XCTAssertFalse(app.buttons["voice-stack-chan00000001"].exists, "Empty channels expose no voice count or status")
        XCTAssertGreaterThanOrEqual(generalVoice.frame.minY, general.frame.maxY, "Voice stays below the channel name")
        XCTAssertLessThanOrEqual(generalVoice.frame.minY - general.frame.maxY, 2, "No extra gap separates the voice action from its channel")
        XCTAssertEqual(generalVoice.frame.width, designVoice.frame.width, "Actions share one stable slot")
        XCTAssertEqual(generalVoice.frame.width, 108, "The whole reserved action slot is accessible, not just its text")
        #if os(iOS)
        XCTAssertGreaterThanOrEqual(general.frame.height, 44)
        XCTAssertGreaterThanOrEqual(design.frame.height, 44)
        XCTAssertGreaterThanOrEqual(generalVoice.frame.height, 44)
        XCTAssertGreaterThanOrEqual(designVoice.frame.height, 44)
        #else
        XCTAssertEqual(general.frame.height, 32)
        XCTAssertEqual(design.frame.height, 32)
        XCTAssertEqual(generalVoice.frame.height, 28)
        XCTAssertEqual(designVoice.frame.height, 28)
        #endif

        let generalFrame = general.frame
        let voiceFrame = generalVoice.frame
        let optionsFrame = app.descendants(matching: .any)["channel-options-chan00000001"].frame
        let stack = app.buttons["voice-stack-chan00000002"]
        XCTAssertEqual(stack.value as? String, "Collapsed", "Occupied rosters start collapsed")
        stack.tap()
        XCTAssertEqual(general.frame, generalFrame, "Expanding another roster must not move the channel name")
        XCTAssertEqual(generalVoice.frame, voiceFrame, "Expanding another roster must not move Join")
        XCTAssertEqual(app.descendants(matching: .any)["channel-options-chan00000001"].frame, optionsFrame,
                       "Expanding another roster must not move the channel menu")

        let options = app.descendants(matching: .any)["channel-options-chan00000001"]
        XCTAssertTrue(options.exists, "Owners have a permanent channel menu")
        options.tap()
        let settings = app.descendants(matching: .any)["Channel settings"].firstMatch
        XCTAssertTrue(settings.waitForExistence(timeout: 3))
        settings.tap()
        assertStaticText("Overview", in: app, timeout: 3)
        XCTAssertEqual(app.textFields["project-updates"].value as? String, "general", "The menu opens its own channel's editor")
        capture("channel-settings-from-menu", app: app)
    }

    func testLeaveChannelLivesInItsOwnOptionsMenu() {
        let app = launch()
        assertElement("selected-channel-name", label: "# general", in: app)
        XCTAssertFalse(app.buttons["Leave channel"].exists, "Leave must not appear in the chat header")
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
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

    func testMuteChannelFromItsOptionsMenu() async throws {
        try await Self.fixtureControl(["reset": true])
        addTeardownBlock { try await Self.fixtureControl(["reset": true]) }
        let app = launch()
        assertElement("selected-channel-name", label: "# general", in: app)
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        let design = app.buttons["channel-chan00000002"]
        XCTAssertTrue(design.waitForExistence(timeout: 10))
        XCTAssertEqual(design.value as? String ?? "", "", "An unselected, unmuted channel has no state")

        let options = app.descendants(matching: .any)["channel-options-chan00000002"]
        options.tap()
        XCTAssertTrue(app.descendants(matching: .any)["Notifications"].firstMatch.waitForExistence(timeout: 5))
        let mute = app.descendants(matching: .any)["Mute channel"].firstMatch
        XCTAssertTrue(mute.waitForExistence(timeout: 5), "The channel menu offers Mute channel")
        let enabled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "isEnabled == true"), object: mute)
        XCTAssertEqual(XCTWaiter.wait(for: [enabled], timeout: 5), .completed, "Mute is available once settings load")
        mute.tap()
        let forever = app.descendants(matching: .any)["Until I turn it back on"].firstMatch
        XCTAssertTrue(forever.waitForExistence(timeout: 3))
        forever.tap()
        waitForMuted(design, true, "A muted channel is marked in the sidebar")
        var overrides = try await Self.fixtureNotificationOverrides()
        XCTAssertEqual(overrides, ["chan00000002:forever"], "The fixture stored the channel mute")
        capture("channel-muted", app: app)

        options.tap()
        let unmute = app.descendants(matching: .any)["Unmute channel"].firstMatch
        XCTAssertTrue(unmute.waitForExistence(timeout: 3), "A muted channel's menu offers Unmute channel")
        XCTAssertTrue(app.descendants(matching: .any)["Muted"].firstMatch.exists, "The menu says it is muted")
        unmute.tap()
        waitForMuted(design, false, "Unmuting clears the sidebar mark")
        overrides = try await Self.fixtureNotificationOverrides()
        XCTAssertEqual(overrides, [], "Unmuting clears the override on the server")
    }

    func testSpectatorRosterCollapsesAndVoiceTargetDoesNotChangeChat() async throws {
        let app = launch()
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app)
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        let stack = app.buttons["voice-stack-chan00000002"]
        XCTAssertTrue(stack.waitForExistence(timeout: 10), "The fixture's design-channel occupants must be visible without joining")
        XCTAssertTrue(stack.label.contains("in voice in design"))
        XCTAssertEqual(stack.value as? String, "Collapsed")
        #if os(iOS)
        let selected = app.buttons["channel-chan00000001"]
        XCTAssertEqual(selected.value as? String, "Selected")
        #else
        let selected = app.descendants(matching: .any)["selected-channel-name"]
        XCTAssertEqual(selected.value as? String, "# general")
        #endif
        XCTAssertTrue(app.buttons["join-voice-chan00000002"].exists)
        stack.tap()
        XCTAssertEqual(stack.value as? String, "Expanded")
        #if os(iOS)
        XCTAssertEqual(selected.value as? String, "Selected", "Collapsing voice occupants must not navigate text chat")
        #else
        XCTAssertEqual(selected.value as? String, "# general", "Collapsing voice occupants must not navigate text chat")
        #endif
        stack.tap()
        XCTAssertEqual(stack.value as? String, "Collapsed")
        capture("spectator-voice-roster", app: app)

        var control = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        control.httpMethod = "POST"
        control.setValue("application/json", forHTTPHeaderField: "content-type")
        control.httpBody = Data(#"{"mediaAccessDenied":{"channelId":"chan00000002"}}"#.utf8)
        let (_, response) = try await URLSession.shared.data(for: control)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        let removed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: stack)
        XCTAssertEqual(XCTWaiter.wait(for: [removed], timeout: 10), .completed,
                       "Revoked private-channel occupancy must disappear without altering selected chat")
        let deniedAction = app.buttons["join-voice-chan00000002"]
        XCTAssertTrue(deniedAction.exists, "Revocation keeps the stable action slot")
        XCTAssertFalse(deniedAction.isEnabled, "A revoked channel cannot be joined")
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

    func testCompactActiveRosterAudioMenuAndCollapsedCallContextFixture() {
        let app = launch(fixture: "voice-roster")
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        let context = app.descendants(matching: .any)["active-voice-context"]
        XCTAssertTrue(context.waitForExistence(timeout: 10))
        // macOS folds a button's child text into the button label, so check the
        // combined label everywhere and the visible line itself where exposed.
        let contextLabel = app.buttons.matching(identifier: "active-voice-context")
            .matching(NSPredicate(format: "label == %@", "Voice connected, general / Fixture Studio"))
        XCTAssertTrue(contextLabel.firstMatch.exists, "Dock context reads channel / space")
        #if os(iOS)
        assertStaticText("general / Fixture Studio", in: app)
        #endif
        XCTAssertFalse(app.buttons["participant-audio-fixture-self"].exists, "Own row has no local playback menu")
        XCTAssertFalse(app.sliders["TEST FIXTURE Maya volume"].exists, "Volume stays in the remote-only Audio menu")
        XCTAssertFalse(app.buttons["participant-audio-fixture-remote"].exists, "Participant controls stay hidden while the roster starts collapsed")
        XCTAssertFalse(app.buttons["join-voice-chan00000001"].exists, "The connected channel has no redundant Leave action")
        XCTAssertEqual(app.buttons.matching(identifier: "Leave voice").count, 1, "Disconnect lives only in the dock")
        let stack = app.buttons["voice-stack-chan00000001"]
        XCTAssertTrue(stack.exists)
        XCTAssertEqual(stack.value as? String, "Collapsed")
        capture("active-voice-compact-test-fixture", app: app)
        stack.tap()
        XCTAssertEqual(stack.value as? String, "Expanded")
        assertStaticText("TEST FIXTURE You (you)", in: app)
        assertStaticText("TEST FIXTURE Maya", in: app)
        XCTAssertTrue(app.buttons["participant-audio-fixture-remote"].exists)
        XCTAssertTrue(context.exists, "Call context and Disconnect remain outside the collapsed participant roster")
        XCTAssertTrue(app.buttons["Leave voice"].exists)
        capture("active-voice-expanded-test-fixture", app: app)
        let audio = app.buttons["participant-audio-fixture-remote"]
        audio.tap()
        XCTAssertTrue(app.sliders["TEST FIXTURE Maya volume"].waitForExistence(timeout: 3))
        #if os(macOS)
        let mute = app.checkBoxes["Mute"]
        #else
        // The labelled row is a Switch; its centre is the label, so tap the inner control.
        let mute = app.switches["Mute"].switches.firstMatch
        #endif
        XCTAssertTrue(mute.exists)
        capture("active-voice-audio-menu-test-fixture", app: app)
        mute.tap()
        #if os(iOS)
        XCTAssertEqual(mute.value as? String, "1")
        #endif
        dismissAudioMenu(in: app)
        let localMute = app.descendants(matching: .any)["participant-local-muted-fixture-remote"]
        XCTAssertTrue(localMute.waitForExistence(timeout: 3))
        assertStaticText("You muted TEST FIXTURE Maya", in: app)
        capture("active-voice-locally-muted-test-fixture", app: app)
        audio.tap()
        XCTAssertTrue(app.sliders["TEST FIXTURE Maya volume"].waitForExistence(timeout: 3))
        mute.tap()
        #if os(iOS)
        XCTAssertEqual(mute.value as? String, "0")
        #endif
        dismissAudioMenu(in: app)
        XCTAssertFalse(localMute.waitForExistence(timeout: 1), "Local mute status disappears when remote playback is restored")
    }

    #if os(macOS)
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

    func testSeparateVoiceRowAndProfileBackdropDismissal() {
        let app = launch()
        let channel = app.buttons["channel-chan00000001"]
        let join = app.buttons["join-voice-chan00000001"]
        XCTAssertTrue(join.waitForExistence(timeout: 10))
        XCTAssertEqual(join.frame.minY, channel.frame.maxY, accuracy: 2, "Join stays directly below the channel's row")
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

    func testMembersCanBeHiddenWithoutChangingConversation() {
        let app = launch()
        let toggle = app.buttons["Hide member list"]
        XCTAssertTrue(toggle.waitForExistence(timeout: 10))
        toggle.tap()
        XCTAssertTrue(app.buttons["Show member list"].waitForExistence(timeout: 2))
        assertElement("selected-channel-name", label: "# general", in: app, timeout: 2)
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app, timeout: 2)
        XCTAssertEqual(staticTexts("Members", in: app).count, 0)
        capture("members-hidden", app: app)
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
        app.scrollViews["space-settings-scroll"].scroll(byDeltaX: 0, deltaY: -600)
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
        XCTAssertFalse(app.buttons["join-voice-button"].exists, "Web joins voice from the channel list, not the chat header")
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        let join = app.buttons["join-voice-chan00000001"]
        XCTAssertTrue(join.waitForExistence(timeout: 5))
        XCTAssertTrue(join.isEnabled, "Normal launches must expose voice without a test-only environment flag")
        capture("voice-ready", app: app)
        let microphone = app.buttons["microphone-toggle"]
        let headphones = app.buttons["deafen-toggle"]
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
        #if os(macOS)
        app.buttons["Input Options"].tap()
        XCTAssertTrue(app.sliders["Input volume"].waitForExistence(timeout: 2))
        XCTAssertFalse(app.sliders["Voice processing"].exists, "Web's input menu has only the device and input volume")
        capture("input-options", app: app)
        app.typeKey(.escape, modifierFlags: [])
        app.buttons["Output Options"].tap()
        XCTAssertTrue(app.sliders["Output volume"].waitForExistence(timeout: 2))
        capture("output-options", app: app)
        app.typeKey(.escape, modifierFlags: [])
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

    func testProfileEditRetainsRejectedValuesAndRetries() async throws {
        let app = launch()
        #if os(iOS)
        app.buttons["Back to Browse"].tap()
        #endif
        let account = app.buttons["account-profile"]
        XCTAssertTrue(account.waitForExistence(timeout: 10))
        account.tap()
        let username = app.textFields["Username"]
        let displayName = app.textFields["Display name"]
        let submit = app.buttons["profile-save"]
        XCTAssertTrue(username.waitForExistence(timeout: 5))
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
        XCTAssertEqual(submit.label, "Finish account")
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

    func testManageSpace() {
        let app = launch(fixture: "manage-space")
        assertStaticText("Manage space", in: app)
        assertElement("space-members-heading", label: "Members 3", in: app)
        capture("manage-space", app: app)
        let username = app.textFields["Exact username"]
        XCTAssertTrue(username.isEnabled, "The dialog must not inherit the disabled workspace")
        app.buttons["Remove"].firstMatch.tap()
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
        assertStaticText("Overview", in: app)
        assertStaticText("Private channel", in: app, timeout: 2)
        capture("private-channel-overview", app: app)
    }

    #if os(iOS)
    func testNarrowConversationAndBrowse() {
        let app = launch()
        let navigation = app.buttons["Back to Browse"]
        XCTAssertTrue(navigation.waitForExistence(timeout: 10))
        assertStaticText("Fixture Owner", in: app)
        let channel = app.descendants(matching: .any)["selected-channel-name"]
        XCTAssertGreaterThan(channel.frame.minX, navigation.frame.maxX, "Back arrow leads the channel menu")
        XCTAssertGreaterThanOrEqual(navigation.frame.width, 44, "Keep the back touch target accessible")
        XCTAssertFalse(app.buttons["Show member list"].exists, "Mobile Members belongs in the channel dropdown")
        XCTAssertFalse(app.buttons["channel-pins"].exists, "Mobile Pins belongs in the channel dropdown")
        let composer = app.descendants(matching: .any)["message-composer"]
        composer.tap(); composer.typeText("Draft survives Browse")
        let timeline = app.descendants(matching: .any)["chat-timeline"]
        let start = timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.03, dy: 0.5))
        start.press(forDuration: 0.01, thenDragTo: timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.03, dy: 0.2)), withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(navigation.isHittable, "A vertical scroll must stay in chat")
        start.press(forDuration: 0.01, thenDragTo: timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)), withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(app.buttons["Close navigation"].waitForExistence(timeout: 3), "Swipe right from the timeline edge opens Browse")
        let browser = app.descendants(matching: .any)["channel-browser"]
        browser.coordinate(withNormalizedOffset: CGVector(dx: 0.97, dy: 0.5)).press(forDuration: 0.01,
            thenDragTo: browser.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: 0.5)), withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(navigation.waitForExistence(timeout: 3), "Swipe left from Browse's edge returns to chat")
        XCTAssertEqual(composer.value as? String, "Draft survives Browse")
        capture("narrow-conversation", app: app)
        channel.tap()
        let members = app.buttons["Members"]
        XCTAssertTrue(members.waitForExistence(timeout: 2))
        capture("narrow-channel-menu", app: app)
        members.tap()
        assertStaticText("Members", in: app, timeout: 2)
        capture("narrow-members", app: app)
        app.buttons["Close member list"].tap()
        XCTAssertEqual(staticTexts("Members", in: app).count, 0)
        channel.tap()
        let pins = app.buttons["channel-pins"]
        XCTAssertTrue(pins.waitForExistence(timeout: 2))
        pins.tap()
        XCTAssertTrue(app.descendants(matching: .any)["pinned-messages"].waitForExistence(timeout: 3))
        assertStaticText("No pinned messages", in: app, timeout: 2)
        capture("narrow-empty-pins", app: app)
        app.buttons["Messages"].tap()
        XCTAssertTrue(navigation.waitForExistence(timeout: 3))
        XCTAssertFalse(app.buttons["channel-pins"].exists, "Dismissing Pins restores the clean header")
        XCTAssertEqual(composer.value as? String, "Draft survives Browse")
        channel.tap()
        members.tap()
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.02, dy: 0.5)).tap()
        XCTAssertEqual(staticTexts("Members", in: app).count, 0)
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
