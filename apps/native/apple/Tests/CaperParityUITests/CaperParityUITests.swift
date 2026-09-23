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
        if testRun?.hasSucceeded == false, let app = launchedApp {
            let hierarchy = XCTAttachment(string: app.debugDescription)
            hierarchy.name = "accessibility-hierarchy-\(name)"
            hierarchy.lifetime = .keepAlways
            add(hierarchy)
            let screenshot = XCTAttachment(screenshot: app.screenshot())
            screenshot.name = "failure-\(name)"
            screenshot.lifetime = .keepAlways
            add(screenshot)
        }
        launchedApp = nil
        super.tearDown()
    }

    private func capture(_ name: String, app: XCUIApplication) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func assertElement(_ identifier: String, label: String, in app: XCUIApplication, timeout: TimeInterval = 10) {
        let element = app.descendants(matching: .any)[identifier]
        XCTAssertTrue(element.waitForExistence(timeout: timeout), "Missing accessibility identifier \(identifier)")
        XCTAssertEqual(element.label, label)
    }

    func testPopulatedWorkspace() {
        let app = launch()
        assertElement("selected-channel-name", label: "# general", in: app)
        XCTAssertTrue(app.staticTexts["TEST FIXTURE — local sample data, not a live conversation."].exists)
        #if os(macOS)
        assertElement("selected-space-name", label: "Fixture Studio", in: app)
        XCTAssertTrue(app.buttons["Hide members"].exists)
        XCTAssertTrue(app.staticTexts["Members"].exists)
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
        XCTAssertTrue(app.staticTexts["TEST FIXTURE — local sample data, not a live conversation."].exists)
        XCTAssertFalse(app.staticTexts["Members"].exists)
        capture("members-hidden", app: app)
    }
    #endif

    func testLogin() {
        let app = launch(fixture: "login", signedIn: false)
        XCTAssertTrue(app.staticTexts["Come on in."].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["guest-general-button"].exists)
        capture("login", app: app)
    }

    func testAccountCanSendExactlyOneMessageAndComposerClears() {
        let app = launch()
        XCTAssertTrue(app.staticTexts["TEST FIXTURE — local sample data, not a live conversation."].waitForExistence(timeout: 10))
        let composer = app.descendants(matching: .any)["message-composer"]
        XCTAssertTrue(composer.exists)
        let message = "Native parity send \(UUID().uuidString)"
        composer.tap()
        composer.typeText(message)
        XCTAssertEqual(composer.value as? String, message)
        let send = app.buttons["send-message-button"]
        XCTAssertTrue(send.isEnabled)
        send.tap()

        let delivered = app.staticTexts.matching(NSPredicate(format: "label == %@", message))
        XCTAssertTrue(delivered.firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(delivered.count, 1, "HTTP confirmation and gateway delivery must merge into one message")
        let cleared = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == ''"), object: composer)
        XCTAssertEqual(XCTWaiter.wait(for: [cleared], timeout: 5), .completed)
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
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: email service unavailable."].waitForExistence(timeout: 5))
        capture("login-error", app: app)
    }

    func testManageSpace() {
        let app = launch(fixture: "manage-space")
        XCTAssertTrue(app.staticTexts["Manage space"].waitForExistence(timeout: 10))
        assertElement("space-members-heading", label: "Members 3", in: app)
        capture("manage-space", app: app)
    }

    func testPrivateChannelOverview() {
        let app = launch(fixture: "manage-channel")
        XCTAssertTrue(app.staticTexts["Channel Overview"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Private channel"].exists)
        capture("private-channel-overview", app: app)
    }

    #if os(iOS)
    func testNarrowConversationAndBrowse() {
        let app = launch()
        XCTAssertTrue(app.buttons["Browse"].waitForExistence(timeout: 10))
        capture("narrow-conversation", app: app)
        app.buttons["Browse"].tap()
        assertElement("selected-space-name", label: "Fixture Studio", in: app, timeout: 5)
        capture("narrow-browse", app: app)
    }
    #endif
}
