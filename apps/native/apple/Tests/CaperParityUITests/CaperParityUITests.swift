import Foundation
import XCTest

@MainActor
final class CaperParityUITests: XCTestCase {
    private var launchedApp: XCUIApplication?

    private static func configuredApp(fixture: String? = nil, signedIn: Bool = true) -> XCUIApplication {
        let app = XCUIApplication()
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
        if (testRun?.failureCount ?? 0) > 0, let app = launchedApp {
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
        let window = app.windows.firstMatch
        XCTAssertTrue(window.waitForExistence(timeout: 2))
        let size = window.frame.size
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

    func testReactionChipsPickerAndEmptySearchState() {
        let app = launch(fixture: "reaction-chips")
        let own = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "selected by you")).firstMatch
        XCTAssertTrue(own.waitForExistence(timeout: 10))
        let other = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "not selected by you")).firstMatch
        XCTAssertTrue(other.exists)
        XCTAssertGreaterThanOrEqual(app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "reaction,")).count, 20)
        XCTAssertGreaterThanOrEqual(app.buttons.matching(NSPredicate(format: "label == %@", "Add reaction")).count, 2, "fixture includes long and empty reaction rows")
        capture("reaction-chips-wrapped-fixture", app: app)
        let add = app.buttons.matching(NSPredicate(format: "label == %@", "Add reaction")).firstMatch
        XCTAssertTrue(add.exists); add.tap()
        #if os(iOS)
        let search = app.textFields["reaction-picker-search"]
        #else
        let search = app.searchFields.firstMatch
        #endif
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        capture("reaction-picker-open-fixture", app: app)
        type("definitely-no-such-emoji", into: search)
        XCTAssertTrue(app.descendants(matching: .any)["reaction-picker-empty"].waitForExistence(timeout: 5))
        capture("reaction-picker-empty-fixture", app: app)
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

    func testSpectatorRosterCollapsesAndVoiceTargetDoesNotChangeChat() async throws {
        let app = launch()
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app)
        #if os(iOS)
        app.buttons["Browse"].tap()
        #endif
        let stack = app.buttons["voice-stack-chan00000002"]
        XCTAssertTrue(stack.waitForExistence(timeout: 10), "The fixture's design-channel occupants must be visible without joining")
        XCTAssertTrue(stack.label.contains("in voice in design"))
        XCTAssertEqual(stack.value as? String, "Expanded")
        #if os(iOS)
        let selected = app.buttons["channel-chan00000001"]
        XCTAssertEqual(selected.value as? String, "Selected")
        #else
        let selected = app.descendants(matching: .any)["selected-channel-name"]
        XCTAssertEqual(selected.value as? String, "# general")
        #endif
        XCTAssertTrue(app.buttons["join-voice-chan00000002"].exists)
        stack.tap()
        XCTAssertEqual(stack.value as? String, "Collapsed")
        #if os(iOS)
        XCTAssertEqual(selected.value as? String, "Selected", "Collapsing voice occupants must not navigate text chat")
        #else
        XCTAssertEqual(selected.value as? String, "# general", "Collapsing voice occupants must not navigate text chat")
        #endif
        stack.tap()
        XCTAssertEqual(stack.value as? String, "Expanded")
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
        XCTAssertFalse(app.buttons["join-voice-chan00000002"].exists)
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
        app.buttons["Browse"].tap()
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
        assertStaticText("TEST FIXTURE You (you)", in: app)
        assertStaticText("TEST FIXTURE Maya", in: app)
        XCTAssertFalse(app.buttons["participant-audio-fixture-self"].exists, "Own row has no local playback menu")
        XCTAssertFalse(app.sliders["TEST FIXTURE Maya volume"].exists, "Volume stays in the remote-only Audio menu")
        let audio = app.buttons["participant-audio-fixture-remote"]
        XCTAssertTrue(audio.exists)
        let stack = app.buttons["voice-stack-chan00000001"]
        XCTAssertTrue(stack.exists)
        capture("active-voice-compact-test-fixture", app: app)
        stack.tap()
        XCTAssertEqual(stack.value as? String, "Collapsed")
        XCTAssertFalse(app.buttons["participant-audio-fixture-remote"].exists)
        XCTAssertTrue(context.exists, "Call context and Disconnect remain outside the collapsed participant roster")
        XCTAssertTrue(app.buttons["Leave voice"].exists)
        capture("active-voice-collapsed-test-fixture", app: app)
        stack.tap()
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

    func testInlineJoinAndProfileBackdropDismissal() {
        let app = launch()
        let channel = app.buttons["channel-chan00000001"]
        let join = app.buttons["join-voice-chan00000001"]
        XCTAssertTrue(join.waitForExistence(timeout: 10))
        XCTAssertEqual(join.frame.midY, channel.frame.midY, accuracy: 2, "Join stays on the channel's row")
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
        app.buttons["Browse"].tap()
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
        XCTAssertTrue(app.descendants(matching: .any)["sound-effects"].waitForExistence(timeout: 2), "Web keeps Caper sound effects in the settings menu")
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
        XCTAssertTrue((140...160).contains(displayedGain), "A 75% slider gesture should select approximately 150% of the 0–200% range")
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
        app.buttons["Browse"].tap()
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

    func testRejectedMessageActionsRenderWithoutSending() {
        let app = launch(fixture: "chat-rejected")
        assertStaticText("Fixture message that was rejected", in: app)
        let edit = app.buttons["Edit"]
        XCTAssertTrue(edit.waitForExistence(timeout: 5))
        XCTAssertTrue(edit.isEnabled)
        XCTAssertTrue(app.scrollViews["chat-timeline"].frame.contains(edit.frame), "Edit must be inside the visible chat viewport")
        XCTAssertTrue(app.buttons["Dismiss"].exists)
        XCTAssertFalse(app.buttons["send-message-button"].isEnabled)
        capture("chat-rejected-fixture", app: app)
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
            app.buttons["Browse"].tap()
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
        type("maya", into: username)
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
        assertStaticText("Maya", in: app)
        assertStaticText("@maya", in: app)
        XCTAssertTrue(username.value as? String == "" || username.value as? String == username.placeholderValue,
            "Successful invitation clears the editable field")
        app.buttons["Close"].firstMatch.tap()
        XCTAssertFalse(app.textFields["Exact username"].exists)
        XCTAssertTrue(app.buttons["Browse"].exists || app.buttons["account-profile"].isHittable)
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
        let navigation = app.buttons["Browse"]
        XCTAssertTrue(navigation.waitForExistence(timeout: 10))
        assertStaticText("Fixture Owner", in: app)
        let channel = app.descendants(matching: .any)["selected-channel-name"]
        XCTAssertGreaterThan(channel.frame.minX, navigation.frame.maxX, "Web's labelled Browse toggle leads the channel title")
        XCTAssertGreaterThanOrEqual(navigation.frame.width, 44, "Keep the menu touch target accessible")
        let members = app.buttons["Show member list"]
        XCTAssertTrue(members.exists)
        XCTAssertGreaterThan(members.frame.minX, app.frame.midX, "Members belongs on the right of the header")
        capture("narrow-conversation", app: app)
        members.tap()
        assertStaticText("Members", in: app, timeout: 2)
        capture("narrow-members", app: app)
        let hideMembers = app.buttons["Hide member list"]
        XCTAssertTrue(hideMembers.isHittable, "The open member panel must leave its toggle accessible")
        app.buttons["Close member list"].tap()
        XCTAssertEqual(staticTexts("Members", in: app).count, 0)
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
