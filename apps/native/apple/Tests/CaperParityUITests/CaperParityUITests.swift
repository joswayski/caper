import Foundation
import XCTest

final class CaperParityUITests: XCTestCase {
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
        return app
    }

    private func capture(_ name: String, app: XCUIApplication) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testPopulatedWorkspace() {
        let app = launch()
        XCTAssertTrue(app.staticTexts["Fixture Studio"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["general"].exists)
        #if os(macOS)
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
        XCTAssertTrue(app.staticTexts["general"].exists)
        XCTAssertFalse(app.staticTexts["Members"].exists)
        capture("members-hidden", app: app)
    }
    #endif

    func testLogin() {
        let app = launch(fixture: "login", signedIn: false)
        XCTAssertTrue(app.staticTexts["Come on in."].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["Join general as a guest"].exists)
        capture("login", app: app)
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
        XCTAssertTrue(app.staticTexts["Members"].exists)
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
        XCTAssertTrue(app.staticTexts["Fixture Studio"].waitForExistence(timeout: 5))
        capture("narrow-browse", app: app)
    }
    #endif
}
