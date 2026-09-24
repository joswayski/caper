import Foundation
import XCTest

@MainActor
final class CaperParityUITests: XCTestCase {
    private var launchedApp: XCUIApplication?

    private func launch(fixture: String? = nil, signedIn: Bool = true) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["CAPER_TEST_MODE"] = "parity"
        app.launchEnvironment["CAPER_API_BASE_URL"] = "http://127.0.0.1:3001"
        if signedIn {
            app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-owner-token"
            app.launchEnvironment["CAPER_TEST_SPACE_ID"] = "space0000001"
        }
        if let fixture { app.launchEnvironment["CAPER_UI_FIXTURE"] = fixture }
        app.launch()
        launchedApp = app
        return app
    }

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

    private func assertStaticText(_ text: String, in app: XCUIApplication, timeout: TimeInterval = 10) {
        XCTAssertTrue(staticTexts(text, in: app).firstMatch.waitForExistence(timeout: timeout), "Missing text: \(text)")
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
        XCTAssertTrue(app.buttons["Hide members"].exists)
        assertStaticText("Members", in: app, timeout: 2)
        #endif
        capture("populated", app: app)
    }

    #if os(macOS)
    func testMembersCanBeHiddenWithoutChangingConversation() {
        let app = launch()
        let toggle = app.buttons["Hide members"]
        XCTAssertTrue(toggle.waitForExistence(timeout: 10))
        toggle.tap()
        XCTAssertTrue(app.buttons["Show members"].waitForExistence(timeout: 2))
        assertElement("selected-channel-name", label: "# general", in: app, timeout: 2)
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app, timeout: 2)
        XCTAssertEqual(staticTexts("Members", in: app).count, 0)
        capture("members-hidden", app: app)
    }
    #endif

    func testLogin() {
        let app = launch(fixture: "login", signedIn: false)
        assertStaticText("Come on in.", in: app)
        XCTAssertTrue(app.buttons["guest-general-button"].exists)
        capture("login", app: app)
    }

    func testAccountCanSendExactlyOneMessageAndComposerClears() {
        let app = launch()
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app)
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
        send.tap()

        let delivered = staticTexts(message, in: app)
        XCTAssertTrue(delivered.firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(delivered.count, 1, "HTTP confirmation and gateway delivery must merge into one message")
        let cleared = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == ''"), object: composer)
        XCTAssertEqual(XCTWaiter.wait(for: [cleared], timeout: 5), .completed)
    }

    func testVoiceEntryAndAudioPreferencesWithoutFeatureFlag() {
        let app = launch()
        assertStaticText("TEST FIXTURE — local sample data, not a live conversation.", in: app)
        let join = app.buttons["join-voice-button"]
        XCTAssertTrue(join.waitForExistence(timeout: 5))
        XCTAssertTrue(join.isEnabled, "Normal launches must expose voice without a test-only environment flag")
        capture("voice-ready", app: app)
        #if os(iOS)
        app.buttons["Open navigation"].tap()
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
        let preferences = app.descendants(matching: .any)["Audio preferences"]
        XCTAssertTrue(preferences.waitForExistence(timeout: 2))
        preferences.tap()

        XCTAssertTrue(app.descendants(matching: .any)["audio-preferences-sheet"].waitForExistence(timeout: 5))
        assertStaticText("Audio preferences", in: app, timeout: 2)
        let gain = app.sliders["Output gain"]
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
        #else
        assertStaticText("Caper follows the input and output selected in macOS System Settings. The embedded WebRTC build does not expose safe per-device switching.", in: app, timeout: 2)
        #endif
        capture("audio-preferences", app: app)
    }

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
        assertStaticText("TEST FIXTURE: email service unavailable.", in: app, timeout: 5)
        capture("login-error", app: app)
    }

    func testManageSpace() {
        let app = launch(fixture: "manage-space")
        assertStaticText("Manage space", in: app)
        assertElement("space-members-heading", label: "Members 3", in: app)
        capture("manage-space", app: app)
    }

    func testPrivateChannelOverview() {
        let app = launch(fixture: "manage-channel")
        assertStaticText("Channel Overview", in: app)
        assertStaticText("Private channel", in: app, timeout: 2)
        capture("private-channel-overview", app: app)
    }

    #if os(iOS)
    func testNarrowConversationAndBrowse() {
        let app = launch()
        let navigation = app.buttons["Open navigation"]
        XCTAssertTrue(navigation.waitForExistence(timeout: 10))
        XCTAssertFalse(app.staticTexts["Browse"].exists)
        assertStaticText("Fixture Owner", in: app)
        let avatar = app.staticTexts["F"].firstMatch
        let author = staticTexts("Fixture Owner", in: app).firstMatch
        let channel = app.descendants(matching: .any)["selected-channel-name"]
        XCTAssertTrue(avatar.exists)
        XCTAssertEqual(navigation.frame.midX, avatar.frame.midX, accuracy: 1, "Menu must center over the message avatars")
        XCTAssertEqual(channel.frame.minX, author.frame.minX, accuracy: 1, "Channel title must align with message authors")
        XCTAssertGreaterThanOrEqual(navigation.frame.width, 44, "Keep the menu touch target accessible")
        let members = app.buttons["Show members"]
        XCTAssertTrue(members.exists)
        XCTAssertGreaterThan(members.frame.minX, app.frame.midX, "Members belongs on the right of the header")
        capture("narrow-conversation", app: app)
        members.tap()
        assertStaticText("Members", in: app, timeout: 2)
        capture("narrow-members", app: app)
        let hideMembers = app.buttons["Hide members"]
        XCTAssertTrue(hideMembers.isHittable, "The open member panel must leave its toggle accessible")
        hideMembers.tap()
        XCTAssertEqual(staticTexts("Members", in: app).count, 0)
        navigation.tap()
        assertElement("selected-space-name", label: "Fixture Studio", in: app, timeout: 5)
        capture("narrow-browse", app: app)
    }
    #endif
}
