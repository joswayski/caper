import AppKit
import XCTest

@MainActor final class CaperAppKitTests: XCTestCase {
    private func launch(signedIn: Bool = true, fixture: String? = nil, preservePanelWidths: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["CAPER_TEST_MODE"] = "parity"
        app.launchEnvironment["CAPER_API_BASE_URL"] = "http://127.0.0.1:3001"
        app.launchEnvironment["CAPER_UI_FIXTURE"] = fixture
        if signedIn {
            app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-owner-token"
            app.launchEnvironment["CAPER_TEST_SPACE_ID"] = "space0000001"
        }
        app.launchArguments = ["-ApplePersistenceIgnoreState", "YES", "-caper.space-landing.owner0000001", ""]
        // Normal flows start at the shipping defaults. Persistence flows omit
        // these argument-domain overrides on their second real app launch.
        if !preservePanelWidths { app.launchArguments += ["-caper.membersWidth", "220", "-caper.channelSidebarWidth", "280", "-caper.threadPanelWidth", "340"] }
        app.launch()
        return app
    }

    private func capture(_ name: String, app: XCUIApplication) {
        let screenshot = XCTAttachment(screenshot: app.windows["Caper — AppKit development (not feature complete)"].screenshot())
        screenshot.name = "AppKit \(name) — local fixture"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    /// Settings is a separate window; the main-window helper cannot capture it.
    private func capture(_ name: String, window: XCUIElement) {
        let screenshot = XCTAttachment(screenshot: window.screenshot())
        screenshot.name = "AppKit \(name) — local fixture"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    private func resize(_ app: XCUIApplication, width: CGFloat) {
        let window = app.windows["Caper — AppKit development (not feature complete)"]
        let frame = window.frame
        // Match the shipping parity harness: rounded corners are not reliable
        // resize targets, and the straight edge stays clear of the Dock.
        let edge = window.coordinate(withNormalizedOffset: .zero)
            .withOffset(CGVector(dx: frame.width - 1, dy: frame.height / 2))
        edge.press(forDuration: 0.1, thenDragTo: edge.withOffset(CGVector(dx: width - frame.width, dy: 0)))
        let resized = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in abs(window.frame.width - width) < 4 }, object: window)
        XCTAssertEqual(XCTWaiter.wait(for: [resized], timeout: 5), .completed)
    }

    /// Only scroll the section that owns the control; never resize it to make
    /// an existing action reachable or rely on XCTest clicking a clipped row.
    private func revealSidebar(_ element: XCUIElement, in scroll: XCUIElement) {
        XCTAssertTrue(scroll.waitForExistence(timeout: 5))
        XCTAssertTrue(element.waitForExistence(timeout: 5))
        for _ in 0..<16 {
            if element.isHittable && scroll.frame.contains(element.frame) { break }
            let step = min(CGFloat(100), scroll.frame.height / 2)
            scroll.scroll(byDeltaX: 0, deltaY: element.frame.minY < scroll.frame.minY ? step : -step)
        }
        XCTAssertTrue(element.isHittable)
        XCTAssertTrue(scroll.frame.contains(element.frame), "The requested sidebar control must be fully inside its own scrollport")
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

    private nonisolated static func fixtureGET(_ path: String, bearer: String = "fixture-owner-token") async throws -> [String: Any] {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001\(path)")!)
        request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
        let (data, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    /// Seed only API routes actually implemented by the disposable loopback
    /// fixture. In particular, it cannot manufacture an arbitrary list of DMs.
    private nonisolated static func fixturePOST(_ path: String, body: [String: Any], status: Int = 200) async throws -> [String: Any] {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001\(path)")!)
        request.httpMethod = "POST"
        request.setValue("Bearer fixture-owner-token", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, status)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    /// UI removal can be optimistic, and the reaction picker closes before its
    /// write finishes. Observe persistence independently, without fixed send delays.
    private static func fixtureEventually(_ path: String, matches: ([String: Any]) -> Bool) async throws -> [String: Any] {
        var snapshot = try await fixtureGET(path)
        for _ in 0..<50 {
            if matches(snapshot) { return snapshot }
            try await Task.sleep(for: .milliseconds(100))
            snapshot = try await fixtureGET(path)
        }
        XCTAssertTrue(matches(snapshot), "The HTTP fixture did not reach the expected state: \(path)")
        return snapshot
    }

    private func messageMenuItem(_ title: String, app: XCUIApplication, body: XCUIElement) -> XCUIElement {
        let item = app.menuItems[title]
        return item.exists ? item : body.menuItems[title]
    }

    private func inspectSettings(_ app: XCUIApplication, captureName: String) -> XCUIElement {
        let settings = app.windows["caper-appkit-settings"]
        XCTAssertTrue(settings.waitForExistence(timeout: 5))
        XCTAssertEqual(settings.label, "Caper Settings — AppKit development")
        XCTAssertEqual(app.windows.matching(identifier: "caper-appkit-settings").count, 1)
        XCTAssertEqual(app.windows.matching(identifier: "Caper Settings — AppKit development").count, 1)
        XCTAssertTrue(settings.descendants(matching: .any)["desktop-settings"].firstMatch.exists)
        let scroll = settings.scrollViews.firstMatch
        XCTAssertTrue(scroll.exists)
        scroll.scroll(byDeltaX: 0, deltaY: 10000)
        let startup = settings.descendants(matching: .any)["launch-at-login"].firstMatch
        let sounds = settings.descendants(matching: .any)["sound-effects"].firstMatch
        let voice = settings.descendants(matching: .any)["voice-messages-send-on-stop"].firstMatch
        XCTAssertTrue(startup.exists); XCTAssertFalse(startup.isEnabled, "Development fixtures must never change OS Login Items")
        XCTAssertTrue(sounds.exists); XCTAssertTrue(sounds.isEnabled); XCTAssertTrue(sounds.isHittable)
        startup.hover(); sounds.hover()
        capture("\(captureName) startup and sounds", window: settings)
        for _ in 0..<6 where !voice.isHittable { scroll.scroll(byDeltaX: 0, deltaY: -120) }
        XCTAssertTrue(voice.exists); XCTAssertTrue(voice.isEnabled); XCTAssertTrue(voice.isHittable)
        voice.hover()
        capture("\(captureName) voice send-on-stop", window: settings)
        // Inspect preferences without changing them: no defaults overrides,
        // recording, OS registration or updater actions.
        scroll.scroll(byDeltaX: 0, deltaY: -10000)
        let updates = settings.buttons["check-for-updates"]
        let notes = settings.buttons["open-whats-new"]
        XCTAssertTrue(updates.exists); XCTAssertFalse(updates.isEnabled)
        XCTAssertTrue(settings.staticTexts["Update checks are available in packaged release builds."].exists)
        XCTAssertTrue(notes.exists); XCTAssertTrue(notes.isEnabled); XCTAssertTrue(notes.isHittable)
        updates.hover(); notes.hover()
        capture("\(captureName) development updates and release notes", window: settings)
        return settings
    }

    func testSettingsSignedOutShortcutAccountAndCaperMenuReuseWindowWithoutLosingDraft() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let signedOut = launch(signedIn: false)
        defer { signedOut.terminate() }
        let email = signedOut.textFields["Email address"]
        XCTAssertTrue(email.waitForExistence(timeout: 10)); resize(signedOut, width: 1280)
        email.click(); email.typeText("settings-")
        signedOut.typeKey(",", modifierFlags: .command)
        let signedOutSettings = inspectSettings(signedOut, captureName: "signed-out Settings main 1280")
        signedOut.typeKey(",", modifierFlags: .command)
        XCTAssertEqual(signedOut.windows.matching(identifier: "caper-appkit-settings").count, 1)
        signedOut.typeKey("w", modifierFlags: .command)
        XCTAssertTrue(signedOutSettings.waitForNonExistence(timeout: 5))
        XCTAssertTrue(email.isHittable, "Command-W must close Settings, not the signed-out main window")
        signedOut.typeText("fixture@example.test")
        XCTAssertEqual(email.value as? String, "settings-fixture@example.test", "Closing restores native sign-in editing focus")
        capture("signed-out main after Settings Command-W 1280", app: signedOut)
        signedOut.terminate()

        let historyPath = "/api/chat/channels/chan00000001/messages"
        let historyBefore = try await Self.fixtureGET(historyPath)
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        var draft = "TEST FIXTURE — keep channel draft across Settings entrypoints"
        composer.click(); composer.typeText(draft)
        let account = app.buttons["account-settings-menu"]
        // AppKit supplies the application menu's title from the development
        // bundle, rather than the shipping app's exact "Caper" display name.
        let caperMenu = app.menuBars.menuBarItems.matching(NSPredicate(format: "label BEGINSWITH %@", "Caper")).firstMatch
        for width in [CGFloat(1280), CGFloat(900)] {
            resize(app, width: width)
            XCTAssertEqual(account.label, "Account settings")
            account.hover(); app.buttons["account-profile"].hover(); account.click()
            let item = app.menuItems["Settings…"]
            XCTAssertTrue(item.waitForExistence(timeout: 5)); XCTAssertTrue(item.isEnabled)
            item.hover(); app.menuItems["What’s new"].hover(); item.click()
            let settings = inspectSettings(app, captureName: "account Settings main \(Int(width))")
            app.typeKey(",", modifierFlags: .command)
            XCTAssertEqual(app.windows.matching(identifier: "caper-appkit-settings").count, 1)
            caperMenu.click()
            let menuSettings = app.menuBars.menuItems["Settings…"]
            XCTAssertTrue(menuSettings.isEnabled)
            menuSettings.hover(); app.menuBars.menuItems["What’s new"].hover(); menuSettings.click()
            XCTAssertEqual(app.windows.matching(identifier: "caper-appkit-settings").count, 1)
            XCTAssertEqual(app.windows.matching(identifier: "Caper Settings — AppKit development").count, 1)
            app.menuBars.menuBarItems["File"].click()
            XCTAssertTrue(app.menuBars.menuItems["Close Window"].isEnabled)
            app.typeKey(.escape, modifierFlags: [])
            app.typeKey("w", modifierFlags: .command)
            XCTAssertTrue(settings.waitForNonExistence(timeout: 5))
            XCTAssertTrue(composer.isHittable); XCTAssertEqual(composer.value as? String, draft)
            app.typeText(" after Settings"); draft += " after Settings"
            XCTAssertEqual(composer.value as? String, draft, "Closing restores editing focus, not just draft text")

            // Reopen through the main menu after Close, not only while visible.
            caperMenu.click(); app.menuBars.menuItems["Settings…"].click()
            XCTAssertTrue(settings.waitForExistence(timeout: 5))
            XCTAssertEqual(app.windows.matching(identifier: "caper-appkit-settings").count, 1)
            XCTAssertEqual(app.windows.matching(identifier: "Caper Settings — AppKit development").count, 1)
            capture("reopened singleton Settings main \(Int(width))", window: settings)
            app.menuBars.menuBarItems["File"].click(); app.menuBars.menuItems["Close Window"].click()
            XCTAssertTrue(settings.waitForNonExistence(timeout: 5))
            XCTAssertTrue(composer.isHittable); XCTAssertEqual(composer.value as? String, draft)
            app.typeText(" after reopening"); draft += " after reopening"
            XCTAssertEqual(composer.value as? String, draft)
            capture("channel draft after Settings File Close \(Int(width))", app: app)
        }
        let historyAfter = try await Self.fixtureGET(historyPath)
        XCTAssertEqual(historyAfter["cursor"] as? String, historyBefore["cursor"] as? String,
            "Settings entrypoints must not submit the channel draft")
    }

    func testWhatsNewFixtureDismissalAccountReopeningAndSettingsOwnershipPreserveDraft() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let historyPath = "/api/chat/channels/chan00000001/messages"
        let historyBefore = try await Self.fixtureGET(historyPath)
        let app = launch(fixture: "whats-new")
        defer { app.terminate() }
        let main = app.windows["Caper — AppKit development (not feature complete)"]
        let caperMenu = app.menuBars.menuBarItems.matching(NSPredicate(format: "label BEGINSWITH %@", "Caper")).firstMatch
        let close = app.buttons["Close What’s new"]
        let done = app.buttons["Done"]
        let preview = app.staticTexts["TEST FIXTURE — Message editing"]
        XCTAssertTrue(preview.waitForExistence(timeout: 15))
        XCTAssertTrue(app.descendants(matching: .any)["whats-new"].firstMatch.exists)
        XCTAssertEqual(app.buttons.matching(identifier: "Close What’s new").count, 1)
        XCTAssertTrue(close.isHittable); XCTAssertTrue(done.isHittable)
        app.typeKey(",", modifierFlags: .command)
        XCTAssertFalse(app.windows["caper-appkit-settings"].exists)
        XCTAssertTrue(preview.exists, "Opening Settings must not replace or acknowledge a presented release sheet")
        for width in [CGFloat(1280), CGFloat(900)] {
            resize(app, width: width)
            close.hover(); done.hover(); close.hover()
            capture("What's new labelled preview main \(Int(width))", app: app)
        }
        close.click()
        XCTAssertTrue(close.waitForNonExistence(timeout: 5)); XCTAssertFalse(preview.exists)
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        let draft = "TEST FIXTURE — keep channel draft while reading What's new"
        composer.click(); composer.typeText(draft)
        // native-project.yml does not stamp CaperSourceRevision, and project.yml
        // deliberately clears CAPER_SOURCE_REVISION for Debug. The one-shot
        // CaperRuntime preview is not release history: manual opens truthfully
        // show this existing state instead of invented published fixture data.
        let unavailable = "Release notes are unavailable in development builds."
        for width in [CGFloat(1280), CGFloat(900)] {
            resize(app, width: width)
            let account = app.buttons["account-settings-menu"]
            account.hover(); app.buttons["account-profile"].hover(); account.click()
            let item = app.menuItems["What’s new"]
            XCTAssertTrue(item.waitForExistence(timeout: 5)); XCTAssertTrue(item.isEnabled)
            item.hover(); app.menuItems["Settings…"].hover(); item.click()
            XCTAssertTrue(close.waitForExistence(timeout: 5))
            XCTAssertTrue(main.buttons["Close What’s new"].exists)
            XCTAssertTrue(app.staticTexts[unavailable].waitForExistence(timeout: 5))
            XCTAssertFalse(preview.exists)
            XCTAssertEqual(app.buttons.matching(identifier: "Close What’s new").count, 1)
            XCTAssertEqual(app.buttons.matching(identifier: "Done").count, 1)
            close.hover(); done.hover()
            capture("account What's new development unavailable main \(Int(width))", app: app)
            if width == 1280 { done.click() } else { app.typeKey(.escape, modifierFlags: []) }
            XCTAssertTrue(close.waitForNonExistence(timeout: 5))
            XCTAssertTrue(composer.waitForExistence(timeout: 5)); XCTAssertEqual(composer.value as? String, draft)

            account.click(); app.menuItems["Settings…"].click()
            let settings = app.windows["caper-appkit-settings"]
            XCTAssertTrue(settings.waitForExistence(timeout: 5))
            XCTAssertEqual(app.windows.matching(identifier: "caper-appkit-settings").count, 1)
            if width == 1280 {
                settings.scrollViews.firstMatch.scroll(byDeltaX: 0, deltaY: -10000)
                let notes = settings.buttons["open-whats-new"]
                XCTAssertTrue(notes.isHittable)
                settings.buttons["check-for-updates"].hover(); notes.hover(); notes.click()
            } else {
                // The global menu must also respect the open Settings owner.
                caperMenu.click()
                app.menuBars.menuItems["Settings…"].hover(); app.menuBars.menuItems["What’s new"].click()
            }
            let ownedClose = settings.buttons["Close What’s new"]
            XCTAssertTrue(ownedClose.waitForExistence(timeout: 5))
            XCTAssertTrue(settings.staticTexts[unavailable].waitForExistence(timeout: 5))
            // Repeated manual requests must foreground this same presentation.
            caperMenu.click(); app.menuBars.menuItems["What’s new"].click()
            XCTAssertFalse(main.buttons["Close What’s new"].exists, "Settings must not also present a workspace sheet")
            XCTAssertEqual(app.buttons.matching(identifier: "Close What’s new").count, 1)
            XCTAssertEqual(app.buttons.matching(identifier: "Done").count, 1)
            XCTAssertEqual(settings.sheets.count, 1)
            XCTAssertEqual(app.sheets.count, 1, "There must be one owner, not two concurrent sheets")
            ownedClose.hover(); settings.buttons["Done"].hover()
            capture("Settings owns What's new main \(Int(width))", window: settings)
            // The shared sheet can extend beyond the fixed-size Settings window.
            capture("Settings-owned What's new sheet main \(Int(width))", window: settings.sheets.firstMatch)
            if width == 1280 { ownedClose.click() } else { app.typeKey(.escape, modifierFlags: []) }
            XCTAssertTrue(close.waitForNonExistence(timeout: 5)); XCTAssertEqual(app.sheets.count, 0)
            XCTAssertTrue(settings.exists, "Dismissing release notes must leave their Settings window open")
            XCTAssertTrue(settings.descendants(matching: .any)["desktop-settings"].firstMatch.exists)
            app.typeKey("w", modifierFlags: .command)
            XCTAssertTrue(settings.waitForNonExistence(timeout: 5))
            XCTAssertTrue(composer.waitForExistence(timeout: 5)); XCTAssertTrue(composer.isHittable)
            XCTAssertEqual(composer.value as? String, draft)
            capture("channel draft after What's new and Settings dismissal \(Int(width))", app: app)
        }
        let historyAfter = try await Self.fixtureGET(historyPath)
        XCTAssertEqual(historyAfter["cursor"] as? String, historyBefore["cursor"] as? String,
            "Reading and dismissing What's new must not send the draft")
    }

    func testActivityInboxLoadRetryFiltersReadRollbackAndDismissalStopPolling() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let path = "/api/notifications/inbox"
        let historyPath = "/api/chat/channels/chan00000001/messages"
        let before = try await Self.fixtureGET(path)
        XCTAssertEqual(before["unreadCount"] as? Int, 2)
        XCTAssertEqual((before["items"] as? [[String: Any]])?.compactMap { $0["id"] as? String }, ["inbox0000002", "inbox0000001"])
        let historyBefore = try await Self.fixtureGET(historyPath)
        try await Self.control(["failure": ["path": path, "method": "GET", "status": 503,
            "persistent": true, "error": "TEST FIXTURE: notifications unavailable."]])
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15)); resize(app, width: 1280)
        let draft = "TEST FIXTURE — keep channel draft while reading notifications"
        composer.click(); composer.typeText(draft)
        let entry = app.buttons["activity-inbox"]
        let dialog = app.descendants(matching: .any)["activity-inbox-dialog"].firstMatch
        let done = app.buttons["activity-done"]
        let retry = app.buttons["Retry"]
        let mention = app.buttons["activity-open-inbox0000002"]
        let reaction = app.buttons["activity-open-inbox0000001"]
        let empty = app.staticTexts["No notifications match these filters."]
        XCTAssertTrue(entry.waitForExistence(timeout: 5)); XCTAssertEqual(entry.label, "Notifications")
        entry.hover(); app.buttons["selected-space-name"].hover(); entry.click()
        XCTAssertTrue(dialog.waitForExistence(timeout: 5)); XCTAssertTrue(retry.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: notifications unavailable."].exists)
        XCTAssertFalse(mention.exists); XCTAssertFalse(reaction.exists); XCTAssertFalse(empty.exists)
        XCTAssertFalse(composer.exists); XCTAssertFalse(app.scrollViews["message-timeline"].exists)
        XCTAssertFalse(app.buttons["channel-chan00000001"].exists)
        capture("activity inbox GET failure wide", app: app)
        try await Self.control(["clearFailures": true])
        retry.hover(); done.hover(); retry.click()
        XCTAssertTrue(mention.waitForExistence(timeout: 10)); XCTAssertTrue(reaction.waitForExistence(timeout: 5))
        XCTAssertTrue(retry.waitForNonExistence(timeout: 5))
        XCTAssertFalse(app.buttons["Load more"].exists, "The existing two-row seed has no next page")

        for (filter, visible, hidden) in [("mentions", mention, reaction), ("reactions", reaction, mention)] {
            let button = app.buttons["activity-filter-\(filter)"]
            button.hover(); app.buttons["activity-filter-all"].hover(); button.click()
            XCTAssertTrue(visible.waitForExistence(timeout: 10)); XCTAssertTrue(hidden.waitForNonExistence(timeout: 5))
            XCTAssertFalse(empty.exists)
        }
        for filter in ["dms", "threads", "invitations"] {
            // Return to a nonempty filter so each empty assertion observes a new load.
            app.buttons["activity-filter-all"].click()
            XCTAssertTrue(mention.waitForExistence(timeout: 10)); XCTAssertTrue(reaction.waitForExistence(timeout: 5))
            app.buttons["activity-filter-\(filter)"].click()
            XCTAssertTrue(empty.waitForExistence(timeout: 10))
            XCTAssertFalse(mention.exists); XCTAssertFalse(reaction.exists); XCTAssertFalse(retry.exists)
        }
        resize(app, width: 900)
        capture("activity inbox empty invitations filter narrow", app: app)
        app.buttons["activity-filter-mentions"].click()
        let read = app.buttons["activity-read-inbox0000002"]
        XCTAssertTrue(read.waitForExistence(timeout: 10))
        XCTAssertEqual(read.label, "Mark notification from Alex read")
        try await Self.control(["failure": ["path": path + "/read", "method": "POST", "status": 503,
            "error": "TEST FIXTURE: notification read unavailable."]])
        read.hover(); mention.hover(); app.buttons["activity-react-inbox0000002"].hover(); read.click()
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: notification read unavailable."].waitForExistence(timeout: 10))
        XCTAssertTrue(read.waitForExistence(timeout: 5)); XCTAssertTrue(mention.exists)
        XCTAssertTrue(app.staticTexts["Unread only (2)"].exists, "The optimistic count rolls back with the unread row")
        let rejected = try await Self.fixtureGET(path)
        XCTAssertEqual(rejected["unreadCount"] as? Int, 2)
        XCTAssertEqual((rejected["items"] as? [[String: Any]])?.filter { $0["unread"] as? Bool == true }.count, 2,
            "A failed read must restore both the row and count, not merely hide its error")
        capture("activity inbox mark read rollback narrow", app: app)
        // Retry reloads the inbox; it must not silently retry the failed write.
        retry.hover(); read.hover(); retry.click()
        XCTAssertTrue(retry.waitForNonExistence(timeout: 10)); XCTAssertTrue(read.exists)
        let afterReload = try await Self.fixtureGET(path)
        XCTAssertEqual(afterReload["unreadCount"] as? Int, 2)
        read.click()
        let accepted = try await Self.fixtureEventually(path) { $0["unreadCount"] as? Int == 1 }
        let items = try XCTUnwrap(accepted["items"] as? [[String: Any]])
        XCTAssertEqual(items.first { $0["id"] as? String == "inbox0000002" }?["unread"] as? Bool, false)
        XCTAssertEqual(items.first { $0["id"] as? String == "inbox0000001" }?["unread"] as? Bool, true)
        XCTAssertTrue(mention.waitForNonExistence(timeout: 5)); XCTAssertTrue(empty.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Unread only (1)"].exists)
        let unreadOnly = app.descendants(matching: .any)["activity-unread-only"].firstMatch
        XCTAssertTrue(unreadOnly.exists); unreadOnly.click()
        XCTAssertTrue(mention.waitForExistence(timeout: 10)); XCTAssertFalse(read.exists)
        XCTAssertFalse(reaction.exists, "Disabling unread-only must retain the Mentions filter")
        capture("activity inbox read notification included narrow", app: app)
        unreadOnly.click()
        XCTAssertTrue(mention.waitForNonExistence(timeout: 10)); XCTAssertTrue(empty.waitForExistence(timeout: 5))
        app.buttons["activity-filter-all"].click()
        XCTAssertTrue(reaction.waitForExistence(timeout: 10)); XCTAssertFalse(mention.exists)
        done.click()
        XCTAssertTrue(dialog.waitForNonExistence(timeout: 5)); XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(entry.value as? String, "1 unread")
        XCTAssertEqual(composer.value as? String, draft)
        app.typeText(" — after Done")
        XCTAssertEqual(composer.value as? String, draft + " — after Done", "Done must restore the native editor's focus")
        let historyAfter = try await Self.fixtureGET(historyPath)
        XCTAssertEqual(historyAfter["cursor"] as? String, historyBefore["cursor"] as? String,
            "Reading notifications must not submit the channel draft")

        // A one-shot GET failure is a sentinel: wait longer than the shipping
        // 15-second inbox poll while closed. Only reopening may consume it.
        try await Self.control(["failure": ["path": path, "method": "GET", "status": 503,
            "error": "TEST FIXTURE: first request after inbox dismissal."]])
        try await Task.sleep(for: .seconds(16))
        entry.click()
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: first request after inbox dismissal."].waitForExistence(timeout: 10),
            "A detached inbox poll must not consume requests after Done")
        capture("activity inbox closed polling sentinel on reopen narrow", app: app)
        retry.click()
        XCTAssertTrue(reaction.waitForExistence(timeout: 10)); XCTAssertTrue(retry.waitForNonExistence(timeout: 5))
        app.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(dialog.waitForNonExistence(timeout: 5)); XCTAssertTrue(composer.waitForExistence(timeout: 5))
        app.typeText(" — after Escape")
        XCTAssertEqual(composer.value as? String, draft + " — after Done — after Escape")
        XCTAssertEqual(entry.value as? String, "1 unread")
    }

    func testActivityInboxInlineReplyReactOriginalNavigationAndInvitationHandoff() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        // Existing control: an object with inbox:true, not a separate invitation ID.
        try await Self.control(["spaceInvitation": ["inbox": true]])
        let path = "/api/notifications/inbox"
        let source = "chan00000001", destination = "chan00000002", originalID = "chan00000001m04"
        let historyPath = "/api/chat/channels/\(source)/messages"
        let destinationPath = "/api/chat/channels/\(destination)/messages"
        for index in 1...55 {
            try await Self.control(["incomingMessage": ["channelId": source, "text": "TEST FIXTURE — notification context distance \(index)"]])
        }
        let newest = try await Self.fixtureGET(historyPath)
        XCTAssertFalse((newest["messages"] as? [[String: Any]] ?? []).contains { $0["id"] as? String == originalID },
            "The original must require a context fetch, not already be in the newest page")
        let destinationBefore = try await Self.fixtureGET(destinationPath)
        let initialInbox = try await Self.fixtureGET(path)
        XCTAssertEqual(initialInbox["unreadCount"] as? Int, 3)
        let invitation = try XCTUnwrap((initialInbox["items"] as? [[String: Any]])?.first { $0["id"] as? String == "inbox0000003" })
        XCTAssertEqual(invitation["kind"] as? String, "space.invitation"); XCTAssertEqual(invitation["spaceId"] as? String, "space0000008")
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15)); resize(app, width: 1280)
        let sourceDraft = "TEST FIXTURE — source draft survives notification navigation"
        let destinationDraft = "TEST FIXTURE — design draft is not an inline reply"
        composer.click(); composer.typeText(sourceDraft)
        app.buttons["channel-\(destination)"].click()
        XCTAssertTrue(app.textViews["message-\(destination)m01"].waitForExistence(timeout: 10))
        composer.click(); composer.typeText(destinationDraft)
        let entry = app.buttons["activity-inbox"]
        let dialog = app.descendants(matching: .any)["activity-inbox-dialog"].firstMatch
        entry.click()
        XCTAssertTrue(app.buttons["activity-open-inbox0000003"].waitForExistence(timeout: 10))
        app.buttons["activity-filter-mentions"].click()
        let row = app.descendants(matching: .any)["activity-row-inbox0000002"].firstMatch
        let reply = app.buttons["activity-reply-inbox0000002"]
        let react = app.buttons["activity-react-inbox0000002"]
        XCTAssertTrue(reply.waitForExistence(timeout: 10)); XCTAssertTrue(row.exists)
        reply.hover(); react.hover(); reply.click()
        let inlineComposer = row.textViews["mention-card-composer"]
        let send = row.buttons["mention-card-send"]
        XCTAssertTrue(inlineComposer.waitForExistence(timeout: 5)); XCTAssertTrue(app.staticTexts["Reply in #general"].exists)
        XCTAssertFalse(composer.exists); XCTAssertFalse(app.buttons["mention-card-message"].exists)
        let replyText = "TEST FIXTURE — reply through the shipping inbox composer"
        inlineComposer.click(); inlineComposer.typeText(replyText)
        reply.click(); XCTAssertTrue(inlineComposer.waitForNonExistence(timeout: 5))
        reply.click(); XCTAssertTrue(inlineComposer.waitForExistence(timeout: 5))
        XCTAssertEqual(inlineComposer.value as? String, replyText, "Collapsing a row must not discard its independent reply draft")
        send.hover(); row.buttons["mention-card-emoji"].hover(); send.hover()
        XCTAssertEqual(inlineComposer.value as? String, replyText)
        capture("activity inbox shared inline reply wide", app: app)
        send.click()
        let cleared = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", ""), object: inlineComposer)
        XCTAssertEqual(XCTWaiter.wait(for: [cleared], timeout: 10), .completed)
        let delivered = try await Self.fixtureEventually(historyPath) { snapshot in
            (snapshot["messages"] as? [[String: Any]] ?? []).contains { ($0["content"] as? [String: Any])?["text"] as? String == replyText }
        }
        let replies = try XCTUnwrap(delivered["messages"] as? [[String: Any]]).filter { ($0["content"] as? [String: Any])?["text"] as? String == replyText }
        XCTAssertEqual(replies.count, 1)
        XCTAssertEqual(replies.first?["channelId"] as? String, source)
        XCTAssertEqual((replies.first?["author"] as? [String: Any])?["id"] as? String, "owner0000001")
        XCTAssertEqual((replies.first?["replyTo"] as? [String: Any])?["id"] as? String, originalID)
        XCTAssertNil(replies.first?["threadRootId"])
        XCTAssertTrue(dialog.exists); XCTAssertFalse(composer.exists, "Inline Send must not open the channel or a DM")
        let followup = "TEST FIXTURE — unsent inline follow-up"
        inlineComposer.click(); inlineComposer.typeText(followup)
        react.hover(); reply.hover(); react.click()
        let search = app.textFields["reaction-picker-search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5)); search.click(); search.typeText("tomato")
        let tomato = app.scrollViews["reaction-picker-grid"].buttons["tomato"]
        XCTAssertTrue(tomato.waitForExistence(timeout: 5)); XCTAssertTrue(tomato.isHittable)
        capture("activity inbox shared reaction picker wide", app: app)
        tomato.hover(); app.buttons["emoji-category-food_drink"].hover(); tomato.click()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        let reactionPath = historyPath + "/\(originalID)/reactions"
        let stored = try await Self.fixtureEventually(reactionPath) { snapshot in
            let group = (snapshot["reactions"] as? [[String: Any]])?.first { $0["emoji"] as? String == "🍅" }
            return (group?["authors"] as? [[String: Any]] ?? []).contains { $0["id"] as? String == "owner0000001" }
        }
        let tomatoGroup = try XCTUnwrap((stored["reactions"] as? [[String: Any]])?.first { $0["emoji"] as? String == "🍅" })
        XCTAssertEqual((tomatoGroup["authors"] as? [[String: Any]])?.compactMap { $0["id"] as? String }, ["owner0000001"])
        XCTAssertTrue(dialog.exists); XCTAssertEqual(inlineComposer.value as? String, followup)
        let afterInline = try await Self.fixtureGET(path)
        XCTAssertEqual(afterInline["unreadCount"] as? Int, 3, "Inline reply/react do not acknowledge notifications")
        let directs = try await Self.fixtureGET("/api/dms")
        XCTAssertTrue(try XCTUnwrap(directs["conversations"] as? [[String: Any]]).isEmpty, "Replying to a mention must not create an actor DM")
        let destinationAfter = try await Self.fixtureGET(destinationPath)
        XCTAssertEqual(destinationAfter["cursor"] as? String, destinationBefore["cursor"] as? String)
        reply.click()
        app.buttons["activity-done"].click()
        XCTAssertTrue(dialog.waitForNonExistence(timeout: 5)); XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, destinationDraft); XCTAssertEqual(entry.value as? String, "3 unread")

        entry.click()
        let open = app.buttons["activity-open-inbox0000002"]
        XCTAssertTrue(open.waitForExistence(timeout: 10)); reply.click()
        XCTAssertTrue(inlineComposer.waitForExistence(timeout: 5)); XCTAssertEqual(inlineComposer.value as? String, followup)
        reply.click(); XCTAssertTrue(inlineComposer.waitForNonExistence(timeout: 5))
        open.hover(); reply.hover(); open.click()
        XCTAssertTrue(dialog.waitForNonExistence(timeout: 10))
        let timeline = app.scrollViews["message-timeline"]
        let original = timeline.textViews["message-\(originalID)"]
        XCTAssertTrue(original.waitForExistence(timeout: 10)); XCTAssertTrue(original.isHittable)
        XCTAssertTrue(timeline.frame.intersects(original.frame)); XCTAssertTrue(app.staticTexts["# general"].exists)
        XCTAssertEqual(composer.value as? String, sourceDraft)
        XCTAssertFalse(app.descendants(matching: .any)["thread-pane"].exists)
        let afterOpen = try await Self.fixtureGET(path)
        XCTAssertEqual(afterOpen["unreadCount"] as? Int, 2)
        XCTAssertEqual((afterOpen["items"] as? [[String: Any]])?.first { $0["id"] as? String == "inbox0000002" }?["unread"] as? Bool, false)
        capture("activity inbox old original channel navigation wide", app: app)

        resize(app, width: 900)
        entry.click(); app.buttons["activity-filter-invitations"].click()
        let openInvitation = app.buttons["activity-open-inbox0000003"]
        XCTAssertTrue(openInvitation.waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["activity-reply-inbox0000003"].exists); XCTAssertFalse(app.buttons["activity-react-inbox0000003"].exists)
        openInvitation.hover(); app.buttons["activity-read-inbox0000003"].hover(); openInvitation.click()
        let invitationTitle = app.staticTexts["Join Maya's Book Club?"]
        XCTAssertTrue(invitationTitle.waitForExistence(timeout: 10)); XCTAssertTrue(dialog.waitForNonExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Accept"].exists); XCTAssertTrue(app.buttons["Decline"].exists)
        XCTAssertFalse(composer.exists); XCTAssertFalse(app.buttons["channel-\(source)"].exists)
        XCTAssertFalse(app.buttons["activity-done"].exists, "The invitation replaces the inbox rather than stacking underneath it")
        let pending = try await Self.fixtureGET("/api/spaces")
        XCTAssertFalse(try XCTUnwrap(pending["spaces"] as? [[String: Any]]).contains { $0["id"] as? String == "space0000008" })
        XCTAssertTrue(try XCTUnwrap(pending["invitations"] as? [[String: Any]]).contains { $0["id"] as? String == "space0000008" },
            "Opening a notification must not accept or decline its pending invitation")
        capture("activity inbox invitation handoff before membership mutation narrow", app: app)
        app.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(invitationTitle.waitForNonExistence(timeout: 5)); XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, sourceDraft)
        let afterCancel = try await Self.fixtureGET("/api/spaces")
        XCTAssertFalse(try XCTUnwrap(afterCancel["spaces"] as? [[String: Any]]).contains { $0["id"] as? String == "space0000008" })
        XCTAssertTrue(try XCTUnwrap(afterCancel["invitations"] as? [[String: Any]]).contains { $0["id"] as? String == "space0000008" })
        entry.click()
        XCTAssertTrue(app.staticTexts["No notifications match these filters."].waitForExistence(timeout: 10))
        app.descendants(matching: .any)["activity-unread-only"].firstMatch.click()
        XCTAssertTrue(openInvitation.waitForExistence(timeout: 10)); openInvitation.click()
        XCTAssertTrue(invitationTitle.waitForExistence(timeout: 10))
        app.buttons["Accept"].hover(); app.buttons["Decline"].hover(); app.buttons["Accept"].click()
        XCTAssertTrue(invitationTitle.waitForNonExistence(timeout: 10))
        XCTAssertTrue(app.buttons["channel-chan00000091"].waitForExistence(timeout: 10))
        let joined = try await Self.fixtureGET("/api/spaces")
        XCTAssertTrue(try XCTUnwrap(joined["spaces"] as? [[String: Any]]).contains { $0["id"] as? String == "space0000008" })
        XCTAssertFalse(try XCTUnwrap(joined["invitations"] as? [[String: Any]]).contains { $0["id"] as? String == "space0000008" })
        let members = try await Self.fixtureGET("/api/spaces/space0000008")
        XCTAssertTrue(try XCTUnwrap(members["members"] as? [[String: Any]]).contains { $0["id"] as? String == "owner0000001" })
        XCTAssertEqual(entry.value as? String, "1 unread")
        capture("activity inbox invitation explicitly accepted narrow", app: app)
    }

    func testForwardPickerCancelLoadRetrySearchAndPartialDeliveryPreserveDrafts() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let sourceID = "chan00000001m02"
        let sourceChannel = "chan00000001", firstChannel = "chan00000002", secondChannel = "chan00000003"
        let channels = [sourceChannel, firstChannel, secondChannel]
        let drafts = ["TEST FIXTURE — source draft", "TEST FIXTURE — design draft", "TEST FIXTURE — planning draft"]
        var cursors: [String: String] = [:]
        for channel in channels {
            let history = try await Self.fixtureGET("/api/chat/channels/\(channel)/messages")
            cursors[channel] = try XCTUnwrap(history["cursor"] as? String)
        }
        let destinations = try await Self.fixtureGET("/api/chat/forward-destinations")
        let sorted = try XCTUnwrap(destinations["destinations"] as? [[String: Any]]).sorted {
            "\($0["spaceName"] as? String ?? "") \($0["name"] as? String ?? "")" < "\($1["spaceName"] as? String ?? "") \($1["name"] as? String ?? "")"
        }
        XCTAssertEqual(sorted.compactMap { $0["id"] as? String }.filter { [firstChannel, secondChannel].contains($0) },
            [firstChannel, secondChannel], "Fail the second selected destination in the picker's sorted submission order")
        func forwards(in channel: String) async throws -> [[String: Any]] {
            let history = try await Self.fixtureGET("/api/chat/channels/\(channel)/messages")
            return try XCTUnwrap(history["messages"] as? [[String: Any]]).filter { $0["forward"] != nil }
        }
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        for (channel, draft) in zip(channels, drafts) {
            app.buttons["channel-\(channel)"].click()
            XCTAssertTrue(app.textViews["message-\(channel)m01"].waitForExistence(timeout: 10))
            composer.click(); composer.typeText(draft)
            XCTAssertEqual(composer.value as? String, draft)
        }
        app.buttons["channel-\(sourceChannel)"].click()
        let timeline = app.scrollViews["message-timeline"]
        let body = timeline.textViews["message-\(sourceID)"]
        XCTAssertTrue(body.waitForExistence(timeout: 10)); XCTAssertEqual(composer.value as? String, drafts[0])
        let search = app.textFields["Find a space, channel or DM"]
        let note = app.textViews["forward-note"]
        let first = app.buttons["forward-destination-\(firstChannel)"]
        let second = app.buttons["forward-destination-\(secondChannel)"]
        func replace(_ field: XCUIElement, with text: String) {
            NSPasteboard.general.clearContents(); NSPasteboard.general.setString(text, forType: .string)
            field.click(); field.typeKey("a", modifierFlags: .command); field.typeKey("v", modifierFlags: .command)
            XCTAssertEqual(field.value as? String, text)
        }
        func openForward(options: Bool) {
            if options {
                timeline.descendants(matching: .any)["message-row-\(sourceID)"].hover()
                timeline.buttons["add-reaction-\(sourceID)"].hover()
                let button = timeline.buttons["message-options-\(sourceID)"]
                button.hover(); button.click()
            } else { body.rightClick() }
            let forward = messageMenuItem("Forward message", app: app, body: body)
            XCTAssertTrue(forward.waitForExistence(timeout: 5)); XCTAssertTrue(forward.isEnabled); forward.click()
            XCTAssertTrue(note.waitForExistence(timeout: 5))
            XCTAssertFalse(composer.exists); XCTAssertFalse(timeline.exists)
            XCTAssertFalse(app.buttons["channel-\(sourceChannel)"].exists, "The workspace modal must isolate navigation as well as the composer")
            let window = app.windows["Caper — AppKit development (not feature complete)"]
            XCTAssertTrue(window.frame.contains(search.frame)); XCTAssertTrue(window.frame.contains(app.buttons["Close"].frame))
            XCTAssertGreaterThan(search.frame.width, 300); XCTAssertLessThanOrEqual(search.frame.width, 460)
        }
        openForward(options: true)
        XCTAssertTrue(first.waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Forward (0)"].isEnabled)
        first.hover(); second.hover(); first.click()
        XCTAssertEqual(first.value as? String, "Selected")
        replace(note, with: "TEST FIXTURE — cancel this note, not the channel draft")
        XCTAssertTrue(app.buttons["Forward (1)"].isEnabled)
        capture("forward picker cancel wide", app: app)
        app.buttons["Close"].click()
        XCTAssertTrue(note.waitForNonExistence(timeout: 5)); XCTAssertEqual(composer.value as? String, drafts[0])
        for channel in channels {
            let history = try await Self.fixtureGET("/api/chat/channels/\(channel)/messages")
            XCTAssertEqual(history["cursor"] as? String, cursors[channel], "Closing the picker must not send to any destination")
            let rows = try await forwards(in: channel)
            XCTAssertTrue(rows.isEmpty)
        }

        try await Self.control(["failure": ["path": "/api/chat/forward-destinations", "method": "GET", "status": 503,
            "error": "TEST FIXTURE: destinations unavailable."]])
        openForward(options: false)
        let retryLoading = app.buttons["Retry loading"]
        XCTAssertTrue(retryLoading.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: destinations unavailable."].exists)
        XCTAssertFalse(first.exists); XCTAssertFalse(app.buttons["Forward (0)"].isEnabled)
        XCTAssertEqual(note.value as? String, "", "A cancelled note must not leak into a new forwarding intent")
        let rawNote = " \nTEST FIXTURE — shared note α\n "
        let trimmedNote = rawNote.trimmingCharacters(in: .whitespacesAndNewlines)
        replace(note, with: rawNote)
        capture("forward destinations load failure wide", app: app)
        retryLoading.hover(); app.buttons["Close"].hover(); retryLoading.click()
        XCTAssertTrue(first.waitForExistence(timeout: 10)); XCTAssertFalse(retryLoading.exists)
        XCTAssertEqual(note.value as? String, rawNote, "Destination loading must not reset the optional note")
        replace(search, with: "no-such-forward-destination")
        XCTAssertTrue(app.staticTexts["No matching destinations."].waitForExistence(timeout: 5))
        XCTAssertFalse(first.exists); XCTAssertFalse(second.exists)
        replace(search, with: " fixture #DeSiGn ")
        XCTAssertTrue(first.waitForExistence(timeout: 5)); XCTAssertFalse(second.exists); first.click()
        replace(search, with: "fixture #planning")
        XCTAssertTrue(second.waitForExistence(timeout: 5)); XCTAssertFalse(first.exists); second.click()
        XCTAssertTrue(app.buttons["Forward (2)"].exists, "Filtering must retain selections hidden by the search")
        replace(search, with: "")
        XCTAssertEqual(first.value as? String, "Selected"); XCTAssertEqual(second.value as? String, "Selected")
        replace(note, with: String(repeating: "e\u{301}", count: 2000) + "🚀")
        XCTAssertFalse(app.buttons["Forward (2)"].isEnabled, "Forward notes use the 4,000 Unicode-scalar limit")
        replace(note, with: rawNote)
        resize(app, width: 900)
        capture("forward filtered selection and note narrow", app: app)
        try await Self.control(["failure": ["path": "/api/chat/channels/\(secondChannel)/forwards", "method": "POST", "status": 503]])
        app.buttons["Forward (2)"].click()
        let retry = app.buttons["Retry forwards (1)"]
        XCTAssertTrue(retry.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Forwarded to 1 destination.")).firstMatch.exists)
        XCTAssertFalse(search.isEnabled); XCTAssertFalse(first.isEnabled); XCTAssertFalse(second.isEnabled)
        XCTAssertEqual(first.value as? String, "Not selected"); XCTAssertEqual(second.value as? String, "Selected")
        XCTAssertEqual(note.value as? String, rawNote)
        let accepted = try await forwards(in: firstChannel)
        let missing = try await forwards(in: secondChannel)
        XCTAssertEqual(accepted.count, 1); XCTAssertTrue(missing.isEmpty, "The second POST failed before persistence, independently of picker state")
        let acceptedID = try XCTUnwrap(accepted.first?["id"] as? String)
        let acceptedKey = try XCTUnwrap(accepted.first?["clientMessageId"] as? String)
        XCTAssertEqual((accepted.first?["content"] as? [String: Any])?["text"] as? String, trimmedNote)
        let acceptedOriginal = try XCTUnwrap((accepted.first?["forward"] as? [String: Any])?["message"] as? [String: Any])
        XCTAssertEqual(acceptedOriginal["id"] as? String, sourceID); XCTAssertEqual(acceptedOriginal["channelId"] as? String, sourceChannel)
        capture("forward partial delivery retains only failed destination narrow", app: app)
        retry.hover(); app.buttons["Close"].hover(); retry.click()
        XCTAssertTrue(note.waitForNonExistence(timeout: 10)); XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, drafts[0])
        let firstAfterRetry = try await forwards(in: firstChannel)
        XCTAssertEqual(firstAfterRetry.compactMap { $0["id"] as? String }, [acceptedID], "Retry must never duplicate the already confirmed destination")
        XCTAssertEqual(firstAfterRetry.first?["clientMessageId"] as? String, acceptedKey)
        for channel in [firstChannel, secondChannel] {
            let rows = try await forwards(in: channel)
            XCTAssertEqual(rows.count, 1)
            let wrapper = try XCTUnwrap(rows.first)
            XCTAssertEqual((wrapper["content"] as? [String: Any])?["text"] as? String, trimmedNote)
            let original = try XCTUnwrap((wrapper["forward"] as? [String: Any])?["message"] as? [String: Any])
            XCTAssertEqual(original["id"] as? String, sourceID); XCTAssertEqual(original["channelId"] as? String, sourceChannel)
        }
        let sourceAfter = try await Self.fixtureGET("/api/chat/channels/\(sourceChannel)/messages")
        XCTAssertEqual(sourceAfter["cursor"] as? String, cursors[sourceChannel], "Forwarding must not post the source draft")
        for (channel, draft) in zip(channels, drafts) {
            app.buttons["channel-\(channel)"].click()
            XCTAssertTrue(app.textViews["message-\(channel)m01"].waitForExistence(timeout: 10))
            XCTAssertEqual(composer.value as? String, draft, "Forwarding must not consume or replace destination drafts")
        }

        // Signed-in previews cannot react or compose, but can share a readable original.
        app.buttons["channel-options-\(firstChannel)"].click(); app.menuItems["Leave channel"].click()
        XCTAssertTrue(app.staticTexts["Leave #design?"].waitForExistence(timeout: 5))
        app.windows.firstMatch.buttons["Leave channel"].click()
        XCTAssertTrue(app.buttons["channel-\(firstChannel)"].waitForNonExistence(timeout: 10))
        app.buttons["selected-space-name"].click(); app.menuItems["Browse channels"].click()
        let browse = app.searchFields["browse-channels-search"]
        XCTAssertTrue(browse.waitForExistence(timeout: 5)); browse.click(); browse.typeText("design")
        app.buttons["browse-channel-\(firstChannel)"].click()
        XCTAssertTrue(app.buttons["join-channel"].waitForExistence(timeout: 10)); XCTAssertFalse(composer.exists)
        let previewBody = app.textViews["message-\(firstChannel)m01"]
        XCTAssertTrue(previewBody.waitForExistence(timeout: 5)); previewBody.rightClick()
        XCTAssertFalse(messageMenuItem("Add reaction…", app: app, body: previewBody).isEnabled)
        let previewForward = messageMenuItem("Forward message", app: app, body: previewBody)
        XCTAssertTrue(previewForward.waitForExistence(timeout: 5)); XCTAssertTrue(previewForward.isEnabled); previewForward.click()
        XCTAssertTrue(note.waitForExistence(timeout: 5)); XCTAssertFalse(app.buttons["join-channel"].exists)
        XCTAssertTrue(second.waitForExistence(timeout: 10)); XCTAssertFalse(first.exists, "An unjoined source is not a writable destination")
        capture("forward from read-only preview narrow", app: app)
        app.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(note.waitForNonExistence(timeout: 5)); XCTAssertTrue(app.buttons["join-channel"].waitForExistence(timeout: 5))
        XCTAssertFalse(composer.exists)
        let afterPreview = try await forwards(in: firstChannel)
        XCTAssertEqual(afterPreview.compactMap { $0["id"] as? String }, [acceptedID], "ESC must dismiss without another forward")
    }

    func testForwardCardsFollowOriginalUpdatesNavigateReforwardAndHandoffFromPins() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let sourceChannel = "chan00000003", destinationChannel = "chan00000002", nextChannel = "chan00000001"
        let sourceID = "chan00000003m02"
        let originalText = "TEST FIXTURE — original shared conversation"
        try await Self.control(["incomingEdit": ["channelId": sourceChannel, "messageId": sourceID, "text": originalText]])
        try await Self.control(["incomingPin": ["channelId": sourceChannel, "messageId": sourceID, "userId": "owner0000001"]])
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15)); resize(app, width: 1280)
        let sourceDraft = "TEST FIXTURE — keep source draft during original navigation"
        let destinationDraft = "TEST FIXTURE — keep destination draft during re-forward"
        app.buttons["channel-\(sourceChannel)"].click()
        XCTAssertTrue(app.textViews["message-\(sourceID)"].waitForExistence(timeout: 10))
        composer.click(); composer.typeText(sourceDraft)
        app.buttons["channel-\(destinationChannel)"].click()
        XCTAssertTrue(app.textViews["message-\(destinationChannel)m01"].waitForExistence(timeout: 10))
        composer.click(); composer.typeText(destinationDraft)
        app.buttons["channel-\(sourceChannel)"].click()
        XCTAssertTrue(app.textViews["message-\(sourceID)"].waitForExistence(timeout: 10))
        app.buttons["toggle-members"].hover(); app.buttons["channel-pins"].hover(); app.buttons["channel-pins"].click()
        let pins = app.descendants(matching: .any)["pinned-messages"]
        let pinned = pins.descendants(matching: .any)["pinned-message-\(sourceID)"]
        XCTAssertTrue(pinned.waitForExistence(timeout: 5)); XCTAssertEqual(composer.value as? String, sourceDraft)
        let pinBody = pinned.staticTexts.matching(NSPredicate(format: "label == %@ OR value == %@", originalText, originalText)).firstMatch
        XCTAssertTrue(pinBody.waitForExistence(timeout: 5))
        pinBody.hover(); pins.buttons["go-to-message-\(sourceID)"].hover(); pinBody.rightClick()
        let pinForward = app.menuItems["Forward message"]
        XCTAssertTrue(pinForward.waitForExistence(timeout: 5)); XCTAssertTrue(pinForward.isEnabled); pinForward.click()
        let note = app.textViews["forward-note"]
        XCTAssertTrue(note.waitForExistence(timeout: 5))
        XCTAssertTrue(pins.waitForNonExistence(timeout: 5), "Pins must hand off to a workspace-owned modal, not retain a picker in a closing popover")
        XCTAssertFalse(composer.exists); XCTAssertFalse(app.buttons["channel-pins"].exists)
        let destination = app.buttons["forward-destination-\(destinationChannel)"]
        XCTAssertTrue(destination.waitForExistence(timeout: 10)); destination.click()
        let wrapperNote = "TEST FIXTURE — first forwarder's note, not the original"
        note.click(); note.typeText(wrapperNote)
        capture("pins to workspace forwarding handoff wide", app: app)
        app.buttons["Forward (1)"].click()
        XCTAssertTrue(note.waitForNonExistence(timeout: 10)); XCTAssertFalse(pins.exists)
        XCTAssertEqual(composer.value as? String, sourceDraft)
        let delivered = try await Self.fixtureGET("/api/chat/channels/\(destinationChannel)/messages")
        let wrappers = try XCTUnwrap(delivered["messages"] as? [[String: Any]]).filter { $0["forward"] != nil }
        XCTAssertEqual(wrappers.count, 1)
        let wrapper = try XCTUnwrap(wrappers.first)
        let wrapperID = try XCTUnwrap(wrapper["id"] as? String)
        XCTAssertEqual((wrapper["content"] as? [String: Any])?["text"] as? String, wrapperNote)
        let original = try XCTUnwrap((wrapper["forward"] as? [String: Any])?["message"] as? [String: Any])
        XCTAssertEqual(original["id"] as? String, sourceID); XCTAssertEqual(original["channelId"] as? String, sourceChannel)
        app.buttons["channel-\(destinationChannel)"].click()
        let timeline = app.scrollViews["message-timeline"]
        let wrapperBody = timeline.textViews["message-\(wrapperID)"]
        XCTAssertTrue(wrapperBody.waitForExistence(timeout: 10)); XCTAssertEqual(wrapperBody.value as? String, wrapperNote)
        XCTAssertEqual(composer.value as? String, destinationDraft)
        let card = timeline.descendants(matching: .any)["forward-card-\(wrapperID)"]
        let open = card.buttons["open-forward-\(wrapperID)"]
        XCTAssertTrue(card.waitForExistence(timeout: 5)); XCTAssertTrue(open.waitForExistence(timeout: 5))
        XCTAssertTrue(card.staticTexts.matching(NSPredicate(format: "label == %@ OR value == %@", originalText, originalText)).firstMatch.exists)
        XCTAssertFalse(card.staticTexts[wrapperNote].exists, "The wrapper note is rendered outside the original card")
        capture("delivered forward original and separate note wide", app: app)

        let editedText = "TEST FIXTURE — the original changed live\nThe forwarder's note must stay untouched."
        try await Self.control(["incomingEdit": ["channelId": sourceChannel, "messageId": sourceID, "text": editedText]])
        let edited = card.staticTexts.matching(NSPredicate(format: "label == %@ OR value == %@", editedText, editedText)).firstMatch
        XCTAssertTrue(edited.waitForExistence(timeout: 10)); XCTAssertEqual(wrapperBody.value as? String, wrapperNote)
        try await Self.control(["incomingReaction": ["channelId": sourceChannel, "messageId": sourceID, "emoji": "👍", "userId": "member000002"]])
        XCTAssertTrue(card.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "👍, 1 reactions")).firstMatch.waitForExistence(timeout: 10))
        try await Self.control(["incomingReply": ["rootId": sourceID, "text": "A live reply to the forwarded original"]])
        let replyCount = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "1 reply · View conversation"), object: open)
        XCTAssertEqual(XCTWaiter.wait(for: [replyCount], timeout: 10), .completed)
        XCTAssertEqual(wrapperBody.value as? String, wrapperNote); XCTAssertEqual(composer.value as? String, destinationDraft)
        let projected = try await Self.fixtureGET("/api/chat/channels/\(destinationChannel)/messages")
        let live = try XCTUnwrap((projected["messages"] as? [[String: Any]])?.first { $0["id"] as? String == wrapperID })
        let liveOriginal = try XCTUnwrap((live["forward"] as? [String: Any])?["message"] as? [String: Any])
        XCTAssertEqual((liveOriginal["content"] as? [String: Any])?["text"] as? String, editedText)
        XCTAssertEqual((liveOriginal["thread"] as? [String: Any])?["replyCount"] as? Int, 1)
        let reactions = try XCTUnwrap(liveOriginal["reactions"] as? [[String: Any]])
        XCTAssertEqual(reactions.first?["emoji"] as? String, "👍"); XCTAssertEqual(reactions.first?["authorIds"] as? [String], ["member000002"])
        XCTAssertEqual((live["content"] as? [String: Any])?["text"] as? String, wrapperNote)
        capture("forward card live edit reaction and reply wide", app: app)

        let conversationPath = "/api/chat/channels/\(destinationChannel)/forwards/\(wrapperID)/thread"
        let conversation = try await Self.fixtureGET(conversationPath)
        let reply = try XCTUnwrap((conversation["messages"] as? [[String: Any]])?.first)
        let replyID = try XCTUnwrap(reply["id"] as? String)
        XCTAssertEqual((conversation["root"] as? [String: Any])?["id"] as? String, sourceID)
        XCTAssertNotNil(conversation["source"] as? [String: Any], "This fixture account can navigate to the original, rather than needing the read-only reader")
        try await Self.control(["failure": ["path": conversationPath, "method": "GET", "status": 503]])
        wrapperBody.hover(); open.hover(); open.click()
        let navigationError = app.staticTexts["conversation-error"]
        XCTAssertTrue(navigationError.waitForExistence(timeout: 10))
        XCTAssertEqual(navigationError.value as? String, "Couldn’t open the conversation. Try again.")
        XCTAssertTrue(card.exists); XCTAssertEqual(composer.value as? String, destinationDraft)
        capture("forward original navigation failure wide", app: app)
        open.click()
        let thread = app.scrollViews["thread-timeline"]
        let root = thread.textViews["message-\(sourceID)"]
        XCTAssertTrue(root.waitForExistence(timeout: 10)); XCTAssertEqual(root.value as? String, editedText)
        XCTAssertTrue(thread.textViews["message-\(replyID)"].waitForExistence(timeout: 10))
        XCTAssertEqual(thread.textViews["message-\(replyID)"].value as? String, (reply["content"] as? [String: Any])?["text"] as? String)
        XCTAssertTrue(root.isHittable); XCTAssertTrue(thread.frame.intersects(root.frame))
        XCTAssertTrue(app.staticTexts["# planning"].exists)
        XCTAssertEqual(composer.value as? String, sourceDraft); XCTAssertEqual(app.textViews["thread-composer"].value as? String, "")
        capture("forward opened original conversation and live reply wide", app: app)
        app.buttons["close-thread"].click()
        XCTAssertTrue(thread.waitForNonExistence(timeout: 5))
        app.buttons["channel-\(destinationChannel)"].click()
        XCTAssertTrue(wrapperBody.waitForExistence(timeout: 10)); XCTAssertEqual(composer.value as? String, destinationDraft)
        resize(app, width: 900)
        wrapperBody.rightClick()
        let reforward = messageMenuItem("Forward message", app: app, body: wrapperBody)
        XCTAssertTrue(reforward.waitForExistence(timeout: 5)); XCTAssertTrue(reforward.isEnabled); reforward.click()
        XCTAssertTrue(note.waitForExistence(timeout: 5)); XCTAssertEqual(note.value as? String, "")
        XCTAssertFalse(composer.exists)
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label == %@ OR value == %@", editedText, editedText)).firstMatch.exists)
        XCTAssertFalse(app.staticTexts[wrapperNote].exists, "Re-forward previews the original, never the previous wrapper's note")
        let next = app.buttons["forward-destination-\(nextChannel)"]
        XCTAssertTrue(next.waitForExistence(timeout: 10)); next.click()
        let nextNote = "TEST FIXTURE — a fresh note on the same original"
        note.click(); note.typeText(nextNote)
        capture("re-forward original without previous wrapper note narrow", app: app)
        app.buttons["Forward (1)"].click()
        XCTAssertTrue(note.waitForNonExistence(timeout: 10)); XCTAssertEqual(composer.value as? String, destinationDraft)
        let nextHistory = try await Self.fixtureGET("/api/chat/channels/\(nextChannel)/messages")
        let nextWrappers = try XCTUnwrap(nextHistory["messages"] as? [[String: Any]]).filter { $0["forward"] != nil }
        XCTAssertEqual(nextWrappers.count, 1)
        let nextWrapper = try XCTUnwrap(nextWrappers.first)
        let nextID = try XCTUnwrap(nextWrapper["id"] as? String)
        XCTAssertNotEqual(nextID, wrapperID)
        XCTAssertEqual((nextWrapper["content"] as? [String: Any])?["text"] as? String, nextNote)
        let nextOriginal = try XCTUnwrap((nextWrapper["forward"] as? [String: Any])?["message"] as? [String: Any])
        XCTAssertEqual(nextOriginal["id"] as? String, sourceID); XCTAssertNotEqual(nextOriginal["id"] as? String, wrapperID)
        XCTAssertEqual(nextOriginal["channelId"] as? String, sourceChannel)
        XCTAssertEqual((nextOriginal["content"] as? [String: Any])?["text"] as? String, editedText)
        XCTAssertNil(nextOriginal["forward"], "Re-forwarding must flatten to the original conversation")
        app.buttons["channel-\(nextChannel)"].click()
        XCTAssertTrue(timeline.textViews["message-\(nextID)"].waitForExistence(timeout: 10))
        XCTAssertEqual(timeline.textViews["message-\(nextID)"].value as? String, nextNote)
        XCTAssertTrue(timeline.descendants(matching: .any)["forward-card-\(nextID)"].waitForExistence(timeout: 5))
        XCTAssertTrue(timeline.buttons["open-forward-\(nextID)"].exists)
        capture("delivered re-forward card narrow", app: app)

        // Alex can read #general, but has no grant to private #planning. Existing
        // fixture identities and reply controls exercise the real reader, not a
        // fabricated response with its source pointer removed.
        for index in 1...50 {
            try await Self.control(["incomingReply": ["rootId": sourceID, "text": "Forward reader page distance \(index)"]])
        }
        let readerPath = "/api/chat/channels/\(nextChannel)/forwards/\(nextID)/thread"
        let readerHistory = try await Self.fixtureGET(readerPath, bearer: "fixture-other-token")
        XCTAssertTrue(readerHistory["source"] is NSNull)
        XCTAssertEqual((readerHistory["root"] as? [String: Any])?["id"] as? String, sourceID)
        XCTAssertEqual(readerHistory["hasMore"] as? Bool, true)
        let newestReplies = try XCTUnwrap(readerHistory["messages"] as? [[String: Any]])
        XCTAssertEqual(newestReplies.count, 50); XCTAssertFalse(newestReplies.contains { $0["id"] as? String == replyID })
        let olderHistory = try await Self.fixtureGET("\(readerPath)?before=\(try XCTUnwrap(newestReplies.first?["seq"] as? String))", bearer: "fixture-other-token")
        XCTAssertEqual((olderHistory["messages"] as? [[String: Any]])?.compactMap { $0["id"] as? String }, [replyID])
        app.terminate()
        app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-other-token"
        app.launchArguments += ["-caper.space-landing.member000002", ""]
        app.launch()
        XCTAssertTrue(composer.waitForExistence(timeout: 15)); resize(app, width: 900)
        XCTAssertFalse(app.buttons["channel-\(sourceChannel)"].exists, "The shared original does not grant membership in its private channel")
        let readerDraft = "TEST FIXTURE — retain the reader's destination draft"
        composer.click(); composer.typeText(readerDraft)
        let readerOpen = timeline.buttons["open-forward-\(nextID)"]
        XCTAssertTrue(readerOpen.waitForExistence(timeout: 10)); readerOpen.hover(); readerOpen.click()
        let readerTitle = app.staticTexts["Forwarded conversation"]
        XCTAssertTrue(readerTitle.waitForExistence(timeout: 10))
        XCTAssertFalse(composer.exists); XCTAssertFalse(timeline.exists); XCTAssertFalse(app.textViews["thread-composer"].exists)
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label == %@ OR value == %@", editedText, editedText)).firstMatch.exists)
        let oldestText = try XCTUnwrap((reply["content"] as? [String: Any])?["text"] as? String)
        let oldestReply = app.staticTexts.matching(NSPredicate(format: "label == %@ OR value == %@", oldestText, oldestText)).firstMatch
        XCTAssertFalse(oldestReply.exists, "The oldest reply starts outside the initial reader page")
        let loadOlder = app.buttons["Load older replies"]
        XCTAssertTrue(loadOlder.waitForExistence(timeout: 10))
        capture("private original shared through read-only reader narrow", app: app)
        try await Self.control(["failure": ["path": readerPath, "method": "GET", "status": 503,
            "error": "TEST FIXTURE: older forwarded replies unavailable."]])
        loadOlder.hover(); app.buttons["Close"].hover(); loadOlder.click()
        let readerRetry = app.buttons["Retry"]
        XCTAssertTrue(readerRetry.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: older forwarded replies unavailable."].exists)
        XCTAssertFalse(composer.exists); XCTAssertFalse(oldestReply.exists)
        capture("forward reader page failure narrow", app: app)
        readerRetry.click()
        XCTAssertTrue(oldestReply.waitForExistence(timeout: 10)); XCTAssertFalse(readerRetry.exists)
        XCTAssertFalse(loadOlder.exists, "Retry must finish both requested pages, not silently reset to the newest 50 replies")
        capture("forward reader older page recovered narrow", app: app)
        app.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(readerTitle.waitForNonExistence(timeout: 5)); XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, readerDraft)
        XCTAssertTrue(timeline.buttons["open-forward-\(nextID)"].exists)
    }

    func testNativePinsRollbackRetryDismissAndLiveAuthorIdentityPreserveDraft() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let path = "/api/chat/channels/chan00000001/messages"
        let id = "chan00000001m02", peerPinID = "chan00000001m01"
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        let draft = "TEST FIXTURE — keep this draft while managing pins"
        composer.click(); composer.typeText(draft)
        let timeline = app.scrollViews["message-timeline"]
        let body = timeline.textViews["message-\(id)"]
        let badge = timeline.buttons["pinned-by-\(id)"]
        let error = timeline.staticTexts["pin-error-\(id)"]
        let retry = timeline.buttons["pin-retry-\(id)"]
        let dismiss = timeline.buttons["pin-dismiss-\(id)"]
        let pins = app.descendants(matching: .any)["pinned-messages"]
        func options() {
            timeline.descendants(matching: .any)["message-row-\(id)"].hover()
            timeline.buttons["add-reaction-\(id)"].hover()
            let button = timeline.buttons["message-options-\(id)"]
            button.hover(); button.click()
            XCTAssertTrue(messageMenuItem("Copy message ID", app: app, body: body).waitForExistence(timeout: 5))
        }
        func openPins() {
            app.buttons["toggle-members"].hover()
            let button = app.buttons["channel-pins"]
            button.hover(); button.click()
            XCTAssertTrue(pins.waitForExistence(timeout: 5))
            XCTAssertEqual(pins.frame.width, 470, accuracy: 3)
            XCTAssertEqual(pins.frame.height, 360, accuracy: 3)
            XCTAssertTrue(composer.exists, "A native popover must not hide the underlying composer from accessibility")
            XCTAssertEqual(composer.value as? String, draft)
        }
        func closePins() {
            app.buttons["close-pins"].click()
            XCTAssertTrue(pins.waitForNonExistence(timeout: 5))
            XCTAssertEqual(composer.value as? String, draft)
        }
        func storedPin(_ messageID: String, authorID: String?) async throws {
            let history = try await Self.fixtureGET(path)
            let message = try XCTUnwrap((history["messages"] as? [[String: Any]])?.first { $0["id"] as? String == messageID })
            let pinned = try XCTUnwrap(history["pinnedMessages"] as? [[String: Any]])
            let pin = message["pin"] as? [String: Any]
            XCTAssertEqual((pin?["author"] as? [String: Any])?["id"] as? String, authorID)
            XCTAssertEqual(pinned.contains { $0["id"] as? String == messageID }, authorID != nil,
                "The HTTP pin list must agree with the timeline, not only its optimistic badge")
        }
        openPins()
        XCTAssertTrue(pins.staticTexts["No pinned messages"].exists)
        capture("pins empty wide with channel draft", app: app)
        closePins()
        try await Self.control(["failure": ["path": "\(path)/\(id)/pin", "method": "PUT", "status": 503]])
        options()
        let pin = messageMenuItem("Pin message", app: app, body: body)
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: pin)
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 10), .completed); pin.click()
        XCTAssertTrue(error.waitForExistence(timeout: 10))
        XCTAssertEqual(error.value as? String, "Couldn’t pin message. Try again.")
        XCTAssertFalse(badge.exists, "A failed pin rolls back its optimistic identity")
        try await storedPin(id, authorID: nil)
        capture("pin failed and rolled back wide", app: app)
        openPins()
        XCTAssertTrue(pins.staticTexts["No pinned messages"].exists, "Rollback must also remove the optimistic pinned-list entry")
        closePins()
        retry.hover(); dismiss.hover(); retry.click()
        XCTAssertTrue(error.waitForNonExistence(timeout: 10))
        // The native button's AX label describes its profile action; its visible title says "Pinned by you".
        XCTAssertTrue(badge.waitForExistence(timeout: 10)); XCTAssertEqual(badge.label, "Open profile for Fixture Owner")
        body.rightClick()
        let unpin = messageMenuItem("Unpin message", app: app, body: body)
        XCTAssertTrue(unpin.waitForExistence(timeout: 5))
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: unpin)
        XCTAssertEqual(XCTWaiter.wait(for: [settled], timeout: 10), .completed)
        app.typeKey(.escape, modifierFlags: [])
        try await storedPin(id, authorID: "owner0000001")
        timeline.buttons["profile-author-\(id)"].hover(); badge.hover(); badge.click()
        XCTAssertTrue(app.buttons["mention-card-edit-profile"].waitForExistence(timeout: 5),
            "The pin author is you even though Maya wrote the message")
        XCTAssertTrue(app.descendants(matching: .any)["mention-card-you"].exists)
        XCTAssertFalse(app.buttons["mention-card-message"].exists)
        capture("own pin author on Maya message", app: app)
        app.buttons["mention-card-close"].click()
        openPins()
        XCTAssertTrue(pins.descendants(matching: .any)["pinned-message-\(id)"].waitForExistence(timeout: 5))
        XCTAssertFalse(pins.staticTexts["No pinned messages"].exists)
        capture("pins populated wide with channel draft", app: app)
        closePins()

        try await Self.control(["failure": ["path": "\(path)/\(id)/pin", "method": "PUT", "status": 503]])
        options(); messageMenuItem("Unpin message", app: app, body: body).click()
        XCTAssertTrue(error.waitForExistence(timeout: 10))
        XCTAssertEqual(error.value as? String, "Couldn’t unpin message. Try again.")
        XCTAssertTrue(badge.exists); XCTAssertEqual(badge.label, "Open profile for Fixture Owner")
        try await storedPin(id, authorID: "owner0000001")
        let beforeDismiss = try await Self.fixtureGET(path)
        capture("unpin failed and restored wide", app: app)
        retry.hover(); dismiss.hover(); dismiss.click()
        XCTAssertTrue(error.waitForNonExistence(timeout: 5)); XCTAssertFalse(retry.exists); XCTAssertFalse(dismiss.exists)
        XCTAssertTrue(badge.exists, "Dismiss clears the error, not the restored pin")
        let afterDismiss = try await Self.fixtureGET(path)
        XCTAssertEqual(afterDismiss["cursor"] as? String, beforeDismiss["cursor"] as? String,
            "Dismiss must not retry the failed unpin")
        body.rightClick()
        XCTAssertTrue(messageMenuItem("Unpin message", app: app, body: body).waitForExistence(timeout: 5))
        messageMenuItem("Unpin message", app: app, body: body).click()
        XCTAssertTrue(badge.waitForNonExistence(timeout: 10))
        body.rightClick()
        let restoredPin = messageMenuItem("Pin message", app: app, body: body)
        XCTAssertTrue(restoredPin.waitForExistence(timeout: 5))
        let unpinned = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: restoredPin)
        XCTAssertEqual(XCTWaiter.wait(for: [unpinned], timeout: 10), .completed)
        app.typeKey(.escape, modifierFlags: [])
        try await storedPin(id, authorID: nil)

        // Reverse authorship to catch a badge opening the message author instead of the pinner.
        try await Self.control(["incomingPin": ["channelId": "chan00000001", "messageId": peerPinID, "userId": "member000001"]])
        let peerBadge = timeline.buttons["pinned-by-\(peerPinID)"]
        XCTAssertTrue(peerBadge.waitForExistence(timeout: 10)); XCTAssertEqual(peerBadge.label, "Open profile for Maya")
        timeline.buttons["profile-author-\(peerPinID)"].hover(); peerBadge.hover(); peerBadge.click()
        let username = app.staticTexts["mention-card-username"]
        XCTAssertTrue(username.waitForExistence(timeout: 5)); XCTAssertEqual(username.label, "@maya")
        XCTAssertFalse(app.descendants(matching: .any)["mention-card-you"].exists)
        XCTAssertTrue(app.textViews["mention-card-composer"].exists)
        capture("Maya pin author on own message", app: app)
        app.buttons["mention-card-close"].click()
        try await storedPin(peerPinID, authorID: "member000001")
        resize(app, width: 900)
        openPins()
        let peerRow = pins.descendants(matching: .any)["pinned-message-\(peerPinID)"]
        XCTAssertTrue(peerRow.waitForExistence(timeout: 5))
        XCTAssertFalse(pins.descendants(matching: .any)["pinned-message-\(id)"].exists)
        capture("pins populated narrow after native unpin", app: app)
        try await Self.control(["incomingPin": ["channelId": "chan00000001", "messageId": peerPinID, "active": false]])
        XCTAssertTrue(peerRow.waitForNonExistence(timeout: 10))
        XCTAssertTrue(pins.staticTexts["No pinned messages"].waitForExistence(timeout: 10))
        XCTAssertFalse(peerBadge.exists, "Live unpin updates the native timeline beneath the still-open popover")
        XCTAssertTrue(composer.exists); XCTAssertEqual(composer.value as? String, draft)
        try await storedPin(peerPinID, authorID: nil)
        capture("pins live unpin to empty narrow", app: app)
        closePins()
    }

    func testPinnedOldOriginalAndPrivateReplyJumpToTheirOwnTimelines() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let path = "/api/chat/channels/chan00000001/messages"
        let oldID = "chan00000001m01", rootID = "chan00000001m02"
        let initial = try await Self.fixtureGET(path)
        let old = try XCTUnwrap((initial["messages"] as? [[String: Any]])?.first { $0["id"] as? String == oldID })
        let oldText = try XCTUnwrap((old["content"] as? [String: Any])?["text"] as? String)
        try await Self.control(["incomingReply": ["rootId": rootID, "text": "Private pinned thread reply"]])
        let threadPath = "\(path)/\(rootID)/thread"
        let thread = try await Self.fixtureGET(threadPath)
        let reply = try XCTUnwrap((thread["messages"] as? [[String: Any]])?.first)
        let replyID = try XCTUnwrap(reply["id"] as? String)
        let replyText = try XCTUnwrap((reply["content"] as? [String: Any])?["text"] as? String)
        XCTAssertEqual(reply["threadRootId"] as? String, rootID); XCTAssertEqual(reply["broadcast"] as? Bool, false)
        try await Self.control(["incomingPin": ["channelId": "chan00000001", "messageId": oldID, "userId": "owner0000001"]])
        // Seed before launch: the original must not be in the app's loaded history already.
        for index in 1...55 {
            try await Self.control(["incomingMessage": ["channelId": "chan00000001", "text": "After old pinned original \(index)"]])
        }
        let newest = try await Self.fixtureGET(path)
        let page = try XCTUnwrap(newest["messages"] as? [[String: Any]])
        XCTAssertEqual(page.count, 50)
        XCTAssertFalse(page.contains { [oldID, rootID, replyID].contains($0["id"] as? String ?? "") })
        XCTAssertEqual((newest["pinnedMessages"] as? [[String: Any]])?.compactMap { $0["id"] as? String }, [oldID])
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        let draft = "TEST FIXTURE — retain draft across pin context jumps"
        composer.click(); composer.typeText(draft)
        let channel = app.scrollViews["message-timeline"]
        let oldBody = channel.textViews["message-\(oldID)"]
        XCTAssertFalse(oldBody.exists)
        app.buttons["toggle-members"].hover(); app.buttons["channel-pins"].hover(); app.buttons["channel-pins"].click()
        let pins = app.descendants(matching: .any)["pinned-messages"]
        XCTAssertTrue(pins.descendants(matching: .any)["pinned-message-\(oldID)"].waitForExistence(timeout: 5))
        XCTAssertTrue(composer.exists); XCTAssertEqual(composer.value as? String, draft)
        capture("old pin outside latest history page wide", app: app)
        let original = pins.buttons["go-to-message-\(oldID)"]
        original.hover(); app.buttons["close-pins"].hover(); original.click()
        XCTAssertTrue(pins.waitForNonExistence(timeout: 10))
        XCTAssertTrue(oldBody.waitForExistence(timeout: 10)); XCTAssertEqual(oldBody.value as? String, oldText)
        XCTAssertTrue(oldBody.isHittable); XCTAssertTrue(channel.frame.intersects(oldBody.frame))
        XCTAssertFalse(app.descendants(matching: .any)["thread-pane"].exists)
        XCTAssertEqual(composer.value as? String, draft)
        capture("pin jumped to old channel original wide", app: app)

        // Replace the list through real gateway events, keeping the next target unambiguous.
        try await Self.control(["incomingPin": ["channelId": "chan00000001", "messageId": oldID, "active": false]])
        try await Self.control(["incomingPin": ["channelId": "chan00000001", "messageId": replyID, "userId": "member000001"]])
        let persisted = try await Self.fixtureGET(path)
        let pinned = try XCTUnwrap((persisted["pinnedMessages"] as? [[String: Any]])?.first)
        XCTAssertEqual(pinned["id"] as? String, replyID); XCTAssertEqual(pinned["threadRootId"] as? String, rootID)
        XCTAssertEqual(pinned["broadcast"] as? Bool, false)
        resize(app, width: 900)
        app.buttons["channel-pins"].click()
        XCTAssertTrue(pins.descendants(matching: .any)["pinned-message-\(replyID)"].waitForExistence(timeout: 10))
        XCTAssertFalse(pins.descendants(matching: .any)["pinned-message-\(oldID)"].exists)
        XCTAssertFalse(channel.textViews["message-\(replyID)"].exists, "A private reply appears in pins, never in channel history")
        XCTAssertTrue(composer.exists); XCTAssertEqual(composer.value as? String, draft)
        capture("private thread reply in channel pins narrow", app: app)
        pins.buttons["go-to-message-\(replyID)"].click()
        XCTAssertTrue(pins.waitForNonExistence(timeout: 10))
        let threadTimeline = app.scrollViews["thread-timeline"]
        let replyBody = threadTimeline.textViews["message-\(replyID)"]
        XCTAssertTrue(replyBody.waitForExistence(timeout: 10)); XCTAssertEqual(replyBody.value as? String, replyText)
        XCTAssertTrue(replyBody.isHittable); XCTAssertTrue(threadTimeline.frame.intersects(replyBody.frame))
        XCTAssertFalse(channel.exists); XCTAssertFalse(composer.exists, "Narrow navigation opens the thread instead of exposing the covered channel")
        let editor = app.textViews["thread-composer"]
        XCTAssertEqual(editor.value as? String, "", "A pin jump must not transfer the channel draft to its thread")
        capture("pin jumped to private reply narrow", app: app)
        resize(app, width: 1280)
        XCTAssertTrue(threadTimeline.textViews["message-\(rootID)"].waitForExistence(timeout: 5), "The pane must belong to the reply's root, not the previously pinned message")
        XCTAssertFalse(threadTimeline.textViews["message-\(oldID)"].exists)
        XCTAssertFalse(channel.textViews["message-\(replyID)"].exists)
        XCTAssertEqual(composer.value as? String, draft)
        app.buttons["close-thread"].click()
        XCTAssertTrue(app.descendants(matching: .any)["thread-pane"].waitForNonExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, draft)
    }

    func testWhoReactedRetriesSwitchesEmojiAndFollowsLiveChangesWithoutToggling() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let id = "chan00000001m01"
        let path = "/api/chat/channels/chan00000001/messages/\(id)/reactions"
        try await Self.control(["incomingReaction": ["channelId": "chan00000001", "messageId": id,
            "emoji": "👍", "userId": "member000001"]])
        try await Self.control(["incomingReaction": ["channelId": "chan00000001", "messageId": id,
            "emoji": "❤️", "userId": "member000002"]])
        try await Self.control(["failure": ["path": path, "method": "GET", "status": 503,
            "persistent": true, "error": "TEST FIXTURE: reaction names unavailable."]])
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        let draft = "TEST FIXTURE — keep my draft while reading reactions"
        composer.click(); composer.typeText(draft)
        let thumb = app.buttons["reaction-\(id)-👍"]
        let heart = app.buttons["reaction-\(id)-❤️"]
        XCTAssertTrue(thumb.waitForExistence(timeout: 5)); heart.hover(); thumb.hover(); thumb.rightClick()
        let show = app.menuItems["Show who reacted"]
        XCTAssertTrue(show.waitForExistence(timeout: 5)); show.click()
        let sheet = app.descendants(matching: .any)["reactors-sheet"]
        let retry = app.buttons["reactors-retry"]
        XCTAssertTrue(sheet.waitForExistence(timeout: 5)); XCTAssertTrue(retry.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Couldn’t load reactions"].exists)
        XCTAssertFalse(composer.exists, "Only the modal's controls belong in accessibility while it is open")
        capture("who reacted load failure wide", app: app)
        // Hover now shares this request/cache; it must not consume a one-shot
        // failure before the details sheet reaches its own error state.
        try await Self.control(["clearFailures": true])
        retry.click()
        let maya = sheet.descendants(matching: .any)["reactor-row-member000001"]
        let alex = sheet.descendants(matching: .any)["reactor-row-member000002"]
        let own = sheet.descendants(matching: .any)["reactor-row-owner0000001"]
        XCTAssertTrue(maya.waitForExistence(timeout: 10)); XCTAssertTrue(maya.label.contains("Maya"))
        XCTAssertFalse(alex.exists); XCTAssertFalse(own.exists, "Opening details must not add the current account's reaction")
        capture("who reacted names wide", app: app)
        sheet.buttons["reactors-tab-❤️"].click()
        XCTAssertTrue(alex.waitForExistence(timeout: 5)); XCTAssertTrue(alex.label.contains("Alex"))
        XCTAssertFalse(maya.exists)
        resize(app, width: 900)
        capture("who reacted second emoji narrow", app: app)
        app.buttons["Done"].click()
        XCTAssertTrue(sheet.waitForNonExistence(timeout: 5))
        XCTAssertTrue(composer.waitForExistence(timeout: 5)); XCTAssertEqual(composer.value as? String, draft)
        XCTAssertEqual(thumb.label, "👍 1"); XCTAssertEqual(heart.label, "❤️ 1")

        thumb.click()
        let toggled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "👍 2"), object: thumb)
        XCTAssertEqual(XCTWaiter.wait(for: [toggled], timeout: 10), .completed, "Ordinary click still toggles the reaction")
        let body = app.textViews["message-\(id)"]
        body.rightClick()
        let view = messageMenuItem("View reactions", app: app, body: body)
        XCTAssertTrue(view.waitForExistence(timeout: 5)); view.click()
        XCTAssertTrue(maya.waitForExistence(timeout: 10)); XCTAssertTrue(own.waitForExistence(timeout: 10))
        let stored = try await Self.fixtureGET(path)
        let groups = try XCTUnwrap(stored["reactions"] as? [[String: Any]])
        let thumbs = try XCTUnwrap(groups.first { $0["emoji"] as? String == "👍" })
        XCTAssertEqual(Set(try XCTUnwrap(thumbs["authors"] as? [[String: Any]]).compactMap { $0["id"] as? String }),
            ["member000001", "owner0000001"])
        for user in ["member000001", "owner0000001"] {
            try await Self.control(["incomingReaction": ["channelId": "chan00000001", "messageId": id,
                "emoji": "👍", "userId": user, "active": false]])
        }
        XCTAssertTrue(sheet.buttons["reactors-tab-👍"].waitForNonExistence(timeout: 10))
        XCTAssertTrue(alex.waitForExistence(timeout: 10), "Removing the selected emoji switches to the surviving tab")
        XCTAssertFalse(own.exists)
        capture("who reacted live tab removal narrow", app: app)
        try await Self.control(["incomingReaction": ["channelId": "chan00000001", "messageId": id,
            "emoji": "❤️", "userId": "member000002", "active": false]])
        XCTAssertTrue(sheet.waitForNonExistence(timeout: 10), "Removing the final reaction closes the native-hosted dialog")
        XCTAssertTrue(composer.waitForExistence(timeout: 5)); XCTAssertEqual(composer.value as? String, draft)
        XCTAssertFalse(thumb.exists); XCTAssertFalse(heart.exists)
    }

    func testReactionHoverRetryLiveNamesAndPinnerProfileRetainFocusAndDrafts() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let channel = "chan00000001", id = "chan00000001m01"
        // The API names this resource /reactions, not /reactors. Seed self last
        // so a self-first summary cannot pass by simply echoing fixture order.
        let path = "/api/chat/channels/\(channel)/messages/\(id)/reactions"
        for person in ["member000001", "owner0000001"] {
            try await Self.control(["incomingReaction": ["channelId": channel, "messageId": id, "emoji": "👍", "userId": person]])
        }
        try await Self.control(["incomingPin": ["channelId": channel, "messageId": id, "userId": "member000001"]])
        try await Self.control(["failure": ["path": path, "method": "GET", "status": 503, "persistent": true]])
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15)); resize(app, width: 1280)
        let draft = "TEST FIXTURE — retain composer focus under hover"
        composer.click(); composer.typeText(draft)
        let timeline = app.scrollViews["message-timeline"]
        let thumb = timeline.buttons["reaction-\(id)-👍"]
        let tooltip = app.descendants(matching: .any)["reaction-tooltip"]
        let profileClose = app.buttons["mention-card-close"]
        let quick = timeline.buttons["quick-reaction-\(id)-❤️"]
        func summary(_ prefix: String) {
            let named = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label BEGINSWITH %@", prefix), object: tooltip)
            XCTAssertEqual(XCTWaiter.wait(for: [named], timeout: 10), .completed)
        }
        XCTAssertTrue(thumb.waitForExistence(timeout: 5)); thumb.hover()
        try await Task.sleep(for: .milliseconds(500)) // Dwell beyond the shared 400ms delay with the real pointer.
        XCTAssertTrue(tooltip.waitForExistence(timeout: 5)); summary("2 people reacted with ")
        XCTAssertTrue(composer.exists); XCTAssertFalse(profileClose.exists)
        app.typeText(" — still focused")
        let retainedDraft = draft + " — still focused"
        XCTAssertEqual(composer.value as? String, retainedDraft, "The bubble must not become a key window or steal the native first responder")
        capture("reaction hover count fallback and composer focus wide", app: app)
        quick.hover()
        XCTAssertTrue(tooltip.waitForNonExistence(timeout: 5))
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertFalse(tooltip.exists); XCTAssertFalse(profileClose.exists, "Neighbor quick reactions do not open a profile or names bubble")
        try await Self.control(["clearFailures": true])
        thumb.hover(); XCTAssertTrue(tooltip.waitForExistence(timeout: 5)); summary("You and Maya reacted with ")
        capture("reaction hover reentry retries names self first wide", app: app)

        // Exercise real profile.updated delivery while the pointer stays on the
        // chip; changing the reaction is not needed to refresh a cached name.
        var rename = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/account/profile")!)
        rename.httpMethod = "POST"
        rename.setValue("Bearer fixture-member-token", forHTTPHeaderField: "Authorization")
        rename.setValue("application/json", forHTTPHeaderField: "Content-Type")
        rename.httpBody = try JSONSerialization.data(withJSONObject: ["username": "maya", "displayName": "Maya Live"])
        let (_, renamed) = try await URLSession.shared.data(for: rename)
        XCTAssertEqual((renamed as? HTTPURLResponse)?.statusCode, 200)
        summary("You and Maya Live reacted with ")
        try await Self.control(["incomingReaction": ["channelId": channel, "messageId": id, "emoji": "👍", "userId": "member000002"]])
        summary("You, Maya Live and Alex reacted with ")
        XCTAssertEqual(thumb.label, "👍 3")
        try await Self.control(["incomingReaction": ["channelId": channel, "messageId": id, "emoji": "👍", "userId": "member000002", "active": false]])
        summary("You and Maya Live reacted with ")
        XCTAssertEqual(thumb.label, "👍 2")
        capture("reaction hover live names and membership wide", app: app)
        composer.hover(); XCTAssertTrue(tooltip.waitForNonExistence(timeout: 5))
        resize(app, width: 900)
        thumb.hover(); XCTAssertTrue(tooltip.waitForExistence(timeout: 5)); summary("You and Maya Live reacted with ")
        XCTAssertGreaterThanOrEqual(tooltip.frame.minX, timeline.frame.minX)
        XCTAssertLessThanOrEqual(tooltip.frame.maxX, timeline.frame.maxX)
        XCTAssertLessThanOrEqual(tooltip.frame.width, 280)
        XCTAssertEqual(composer.value as? String, retainedDraft)
        capture("reaction hover bounded inside narrow conversation", app: app)
        let beforeDetails = try await Self.fixtureGET("/api/chat/channels/\(channel)/messages")
        thumb.rightClick()
        let show = app.menuItems["Show who reacted"]
        XCTAssertTrue(show.waitForExistence(timeout: 5)); show.click()
        let sheet = app.descendants(matching: .any)["reactors-sheet"]
        XCTAssertTrue(sheet.waitForExistence(timeout: 5)); XCTAssertTrue(tooltip.waitForNonExistence(timeout: 5))
        XCTAssertTrue(sheet.descendants(matching: .any)["reactor-row-member000001"].waitForExistence(timeout: 10))
        XCTAssertTrue(sheet.descendants(matching: .any)["reactor-row-owner0000001"].exists)
        let afterDetails = try await Self.fixtureGET("/api/chat/channels/\(channel)/messages")
        XCTAssertEqual(afterDetails["cursor"] as? String, beforeDetails["cursor"] as? String, "Right-click details must not toggle a reaction under the hover overlay")
        let listed = try await Self.fixtureGET(path)
        let thumbs = try XCTUnwrap((listed["reactions"] as? [[String: Any]])?.first { $0["emoji"] as? String == "👍" })
        XCTAssertEqual(Set(try XCTUnwrap(thumbs["authors"] as? [[String: Any]]).compactMap { $0["id"] as? String }),
            ["member000001", "owner0000001"])
        app.buttons["Done"].click(); XCTAssertTrue(sheet.waitForNonExistence(timeout: 5))
        thumb.hover(); XCTAssertTrue(tooltip.waitForExistence(timeout: 5)); thumb.click()
        let toggled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "👍 1"), object: thumb)
        XCTAssertEqual(XCTWaiter.wait(for: [toggled], timeout: 10), .completed, "The bubble must not intercept an ordinary chip click")
        composer.hover(); XCTAssertTrue(tooltip.waitForNonExistence(timeout: 5))

        resize(app, width: 1280)
        // Only the pinner auto-opens. Author, avatar and member controls keep
        // click-to-open behavior; these captures are for Mac hover-wash review.
        for identifier in ["profile-author-chan00000001m02", "profile-avatar-chan00000001m02", "profile-member-member000001"] {
            let person = app.buttons[identifier]
            XCTAssertTrue(person.waitForExistence(timeout: 5)); person.hover()
            try await Task.sleep(for: .milliseconds(500))
            XCTAssertFalse(profileClose.exists); XCTAssertFalse(tooltip.exists)
            capture("native person hover wash \(identifier) wide", app: app)
        }
        let badge = timeline.buttons["pinned-by-\(id)"]
        XCTAssertTrue(badge.waitForExistence(timeout: 5)); XCTAssertEqual(badge.label, "Open profile for Maya Live")
        quick.hover(); badge.hover()
        let profileDraft = app.textViews["mention-card-composer"]
        XCTAssertTrue(profileDraft.waitForExistence(timeout: 5))
        XCTAssertEqual(app.staticTexts["mention-card-username"].label, "@maya", "Hover must open the pinner, not the message's owner")
        let privateDraft = "TEST FIXTURE — retain the pinner's profile draft"
        profileDraft.click(); profileDraft.typeText(privateDraft)
        capture("pinner hover opens profile with independent draft wide", app: app)
        timeline.buttons["add-reaction-\(id)"].hover(); timeline.buttons["message-options-\(id)"].hover()
        app.buttons["profile-author-chan00000001m04"].hover()
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertEqual(app.staticTexts["mention-card-username"].label, "@maya", "Neighbor controls and ordinary authors must not replace the open pinner profile")
        XCTAssertEqual(profileDraft.value as? String, privateDraft)
        badge.hover(); badge.click()
        XCTAssertEqual(profileDraft.value as? String, privateDraft, "Leaving/reentering and clicking the same pinner must not recreate its presenter draft")
        XCTAssertEqual(composer.value as? String, retainedDraft)
        capture("same pinner reentry and click retain both drafts wide", app: app)
        profileClose.click(); XCTAssertTrue(profileDraft.waitForNonExistence(timeout: 5))
        quick.hover(); try await Task.sleep(for: .milliseconds(500))
        XCTAssertFalse(profileClose.exists); XCTAssertFalse(tooltip.exists)
        badge.hover(); XCTAssertTrue(profileDraft.waitForExistence(timeout: 5))
        XCTAssertEqual(profileDraft.value as? String, privateDraft, "An explicit close also preserves the existing account-scoped profile draft")
        profileClose.click(); XCTAssertTrue(profileDraft.waitForNonExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, retainedDraft)
    }

    func testMessageReplyQuoteCancelSendAndOriginalNavigation() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        try await Self.control(["uploadsEnabled": true])
        let path = "/api/chat/channels/chan00000001/messages"
        let targetID = "chan00000001m02"
        let initial = try await Self.fixtureGET(path)
        let target = try XCTUnwrap((initial["messages"] as? [[String: Any]])?.first { $0["id"] as? String == targetID })
        let author = try XCTUnwrap((target["author"] as? [String: Any])?["name"] as? String)
        let original = try XCTUnwrap((target["content"] as? [String: Any])?["text"] as? String)
        let source = try imageFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        let timeline = app.scrollViews["message-timeline"]
        let body = timeline.textViews["message-\(targetID)"]
        XCTAssertTrue(body.waitForExistence(timeout: 5))
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: app.buttons["attach-files-button"])
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 10), .completed)
        let draft = "TEST FIXTURE — quoted reply α 👩🏽‍💻 café"
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(draft, forType: .string)
        composer.click(); composer.typeKey("v", modifierFlags: .command)
        NSPasteboard.general.clearContents(); NSPasteboard.general.writeObjects([source as NSURL])
        composer.typeKey("v", modifierFlags: .command)
        let staged = app.buttons["staged-file-remove"]
        XCTAssertTrue(staged.waitForExistence(timeout: 10))
        timeline.scroll(byDeltaX: 0, deltaY: 1000)
        let row = timeline.descendants(matching: .any)["message-row-\(targetID)"]
        XCTAssertTrue(row.waitForExistence(timeout: 5)); row.hover()
        let options = timeline.buttons["message-options-\(targetID)"]
        XCTAssertTrue(options.waitForExistence(timeout: 5)); options.hover(); options.click()
        for title in ["Reply", "Reply in thread"] {
            let item = messageMenuItem(title, app: app, body: body)
            XCTAssertTrue(item.waitForExistence(timeout: 5)); XCTAssertTrue(item.isEnabled)
        }
        messageMenuItem("Reply", app: app, body: body).click()
        let cancel = app.buttons["Cancel reply"]
        XCTAssertTrue(cancel.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Replying to \(author)"].exists)
        XCTAssertEqual(composer.value as? String, draft)
        XCTAssertTrue(staged.exists, "Selecting Reply must not discard staged files")
        XCTAssertFalse(app.descendants(matching: .any)["thread-pane"].exists, "Reply belongs to the channel, not a new thread")
        capture("channel reply draft and staged file wide", app: app)
        cancel.click()
        XCTAssertTrue(cancel.waitForNonExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, draft)
        XCTAssertTrue(staged.exists, "Cancelling only clears the quote")
        body.rightClick()
        for title in ["Reply", "Reply in thread"] {
            let item = messageMenuItem(title, app: app, body: body)
            XCTAssertTrue(item.waitForExistence(timeout: 5)); XCTAssertTrue(item.isEnabled)
        }
        messageMenuItem("Reply in thread", app: app, body: body).click()
        XCTAssertTrue(app.textViews["thread-composer"].waitForExistence(timeout: 5))
        XCTAssertEqual(app.textViews["thread-composer"].value as? String, "")
        XCTAssertFalse(cancel.exists, "Reply in thread must not implicitly quote the root")
        XCTAssertEqual(composer.value as? String, draft)
        XCTAssertTrue(staged.exists, "Opening a thread leaves the channel's staged file in its original composer")
        app.buttons["close-thread"].click()
        XCTAssertTrue(app.descendants(matching: .any)["thread-pane"].waitForNonExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, draft); XCTAssertTrue(staged.exists)
        timeline.scroll(byDeltaX: 0, deltaY: 1000)
        XCTAssertTrue(body.waitForExistence(timeout: 5)); body.rightClick()
        XCTAssertTrue(messageMenuItem("Reply", app: app, body: body).waitForExistence(timeout: 5))
        messageMenuItem("Reply", app: app, body: body).click()
        XCTAssertTrue(cancel.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, draft)
        XCTAssertTrue(staged.exists)

        // Push the original outside the newest HTTP page, not merely behind
        // another view. View original must fetch context and reveal its target.
        for index in 1...55 {
            try await Self.control(["incomingMessage": ["channelId": "chan00000001", "text": "Reply context distance \(index)"]])
        }
        app.buttons["send-message"].click()
        timeline.scroll(byDeltaX: 0, deltaY: -10000)
        let delivered = timeline.textViews.matching(NSPredicate(format: "value == %@", draft)).firstMatch
        XCTAssertTrue(delivered.waitForExistence(timeout: 20), "The quoted image message must reach the HTTP fixture and gateway")
        let sentID = String(delivered.identifier.dropFirst("message-".count))
        XCTAssertEqual(sentID.count, 15)
        XCTAssertTrue(cancel.waitForNonExistence(timeout: 5)); XCTAssertFalse(staged.exists)
        XCTAssertEqual(composer.value as? String, "")
        let history = try await Self.fixtureGET(path)
        let stored = try XCTUnwrap((history["messages"] as? [[String: Any]])?.first { $0["id"] as? String == sentID })
        let quote = try XCTUnwrap(stored["replyTo"] as? [String: Any])
        XCTAssertEqual(quote["id"] as? String, targetID)
        XCTAssertEqual(quote["text"] as? String, original)
        XCTAssertNil(stored["threadRootId"])
        XCTAssertNil(stored["broadcast"])
        XCTAssertEqual(((stored["content"] as? [String: Any])?["attachments"] as? [[String: Any]])?.count, 1)
        XCTAssertFalse(body.exists, "The old original must not already be realized at the live end")
        let sentRow = timeline.descendants(matching: .any)["message-row-\(sentID)"]
        let viewOriginal = sentRow.buttons["View original message from \(author)"]
        XCTAssertTrue(viewOriginal.waitForExistence(timeout: 5))
        capture("delivered reply context wide", app: app)
        viewOriginal.click()
        XCTAssertTrue(body.waitForExistence(timeout: 10))
        XCTAssertEqual(body.value as? String, original)
        XCTAssertTrue(body.isHittable); XCTAssertTrue(timeline.frame.intersects(body.frame))
        XCTAssertFalse(app.descendants(matching: .any)["thread-pane"].exists)
        capture("reply jumped to original channel message", app: app)

        // A cancelled quote must also be absent from the next server payload.
        body.rightClick()
        let reply = messageMenuItem("Reply", app: app, body: body)
        XCTAssertTrue(reply.waitForExistence(timeout: 5)); reply.click()
        XCTAssertTrue(cancel.waitForExistence(timeout: 5)); cancel.click()
        XCTAssertTrue(cancel.waitForNonExistence(timeout: 5))
        let unquoted = "TEST FIXTURE — cancelled quote sends normally"
        composer.click(); composer.typeText(unquoted); app.buttons["send-message"].click()
        XCTAssertTrue(timeline.textViews.matching(NSPredicate(format: "value == %@", unquoted)).firstMatch.waitForExistence(timeout: 15))
        let afterCancel = try await Self.fixtureGET(path)
        let plain = try XCTUnwrap((afterCancel["messages"] as? [[String: Any]])?.first {
            ($0["content"] as? [String: Any])?["text"] as? String == unquoted
        })
        XCTAssertNil(plain["replyTo"]); XCTAssertNil(plain["threadRootId"])
    }

    func testThreadDraftsNarrowBackSendIsolationRetryAndRootNavigation() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let path = "/api/chat/channels/chan00000001/messages"
        let rootID = "chan00000001m02", otherRootID = "chan00000001m01"
        let threadPath = "\(path)/\(rootID)/thread"
        try await Self.control(["incomingReply": ["rootId": rootID, "text": "Original thread reply"]])
        try await Self.control(["incomingReply": ["rootId": otherRootID, "text": "Second root reply"]])
        let initialChannel = try await Self.fixtureGET(path)
        let initialChannelCount = try XCTUnwrap(initialChannel["messages"] as? [[String: Any]]).count
        let initial = try await Self.fixtureGET(threadPath)
        let seed = try XCTUnwrap((initial["messages"] as? [[String: Any]])?.first)
        let seedID = try XCTUnwrap(seed["id"] as? String)
        let seedText = try XCTUnwrap((seed["content"] as? [String: Any])?["text"] as? String)
        let seedAuthor = try XCTUnwrap((seed["author"] as? [String: Any])?["name"] as? String)
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        let channel = app.scrollViews["message-timeline"]
        let pane = app.descendants(matching: .any)["thread-pane"]
        let timeline = app.scrollViews["thread-timeline"]
        let editor = app.textViews["thread-composer"]
        let send = app.buttons["thread-send"]
        let back = app.buttons["close-thread"]
        let broadcast = app.checkBoxes["thread-broadcast"]
        let window = app.windows["Caper — AppKit development (not feature complete)"]
        let members = app.scrollViews["member-list"]
        let membersToggle = app.buttons["toggle-members"]
        let divider = app.sliders["thread-panel-resize"]
        let summary = channel.buttons["thread-summary-\(rootID)"]
        let channelDraft = "TEST FIXTURE — keep the channel draft"
        let threadDraft = "TEST FIXTURE — private reply to the first root"
        let otherDraft = "TEST FIXTURE — second root keeps its own draft"
        composer.click(); composer.typeText(channelDraft)
        func openOtherRoot() {
            XCTAssertTrue(channel.waitForExistence(timeout: 5))
            channel.scroll(byDeltaX: 0, deltaY: 10000)
            let link = channel.buttons["thread-summary-\(otherRootID)"]
            XCTAssertTrue(link.waitForExistence(timeout: 5)); link.click()
            XCTAssertTrue(editor.waitForExistence(timeout: 5))
        }
        try await Self.control(["failure": ["path": threadPath, "method": "GET", "status": 503,
            "error": "TEST FIXTURE: thread history unavailable."]])
        let root = channel.textViews["message-\(rootID)"]
        XCTAssertTrue(root.waitForExistence(timeout: 5)); root.rightClick()
        let open = messageMenuItem("Reply in thread", app: app, body: root)
        XCTAssertTrue(open.waitForExistence(timeout: 5)); open.click()
        XCTAssertTrue(pane.waitForExistence(timeout: 5))
        let loadError = pane.staticTexts["TEST FIXTURE: thread history unavailable."]
        XCTAssertTrue(loadError.waitForExistence(timeout: 10))
        let retryLoad = pane.buttons["Retry"]
        XCTAssertTrue(retryLoad.exists)
        XCTAssertEqual(composer.value as? String, channelDraft)
        capture("thread history failure wide", app: app)
        retryLoad.click()
        let seedBody = timeline.textViews["message-\(seedID)"]
        XCTAssertTrue(seedBody.waitForExistence(timeout: 10))
        XCTAssertTrue(loadError.waitForNonExistence(timeout: 5))
        XCTAssertEqual(seedBody.value as? String, seedText)
        XCTAssertTrue(composer.isHittable); XCTAssertTrue(editor.isHittable)
        let channelMicrophone = app.buttons["composer-voice-record"]
        let threadMicrophone = app.buttons["thread-voice-record"]
        XCTAssertTrue(channelMicrophone.waitForExistence(timeout: 5))
        XCTAssertTrue(threadMicrophone.waitForExistence(timeout: 5))
        XCTAssertEqual(channelMicrophone.label, "Dictate")
        XCTAssertEqual(threadMicrophone.label, "Dictate")
        XCTAssertTrue(channelMicrophone.isHittable); XCTAssertTrue(threadMicrophone.isHittable)
        // The two controllers intentionally share attach/emoji/GIF identifiers.
        // Scope thread controls to its real AX pane, and require exactly one
        // channel control inside the channel's horizontal bounds, not firstMatch.
        func channelAccessory(_ identifier: String) throws -> XCUIElement {
            let matches = app.buttons.matching(identifier: identifier)
            XCTAssertTrue(matches.firstMatch.waitForExistence(timeout: 5))
            let candidates = matches.allElementsBoundByIndex.filter {
                $0.frame.minX >= channel.frame.minX && $0.frame.maxX <= channel.frame.maxX
            }
            XCTAssertEqual(candidates.count, 1, "Exactly one \(identifier) must fit in the channel pane")
            return try XCTUnwrap(candidates.first)
        }
        func assertComposerGeometry(inThread: Bool) throws {
            let input = inThread ? editor : composer
            let bounds = inThread ? pane.frame : CGRect(x: channel.frame.minX, y: window.frame.minY,
                width: channel.frame.width, height: window.frame.height)
            // A long draft's NSTextView can exceed its clipped viewport. Test
            // the native scrollport rather than confusing hidden text with overlap.
            let viewports = app.scrollViews.containing(.textView, identifier: input.identifier)
            XCTAssertEqual(viewports.count, 1)
            let viewport = viewports.firstMatch
            XCTAssertTrue(viewport.exists); XCTAssertTrue(input.isHittable)
            var controls = [viewport]
            for identifier in ["attach-files-button", "composer-gif", "composer-emoji"] {
                let control: XCUIElement
                if inThread { control = pane.buttons[identifier] }
                else { control = try channelAccessory(identifier) }
                XCTAssertTrue(control.waitForExistence(timeout: 5))
                controls.append(control)
                control.hover()
            }
            let microphone = inThread ? threadMicrophone : channelMicrophone
            let submit = inThread ? send : app.buttons["send-message"]
            controls += [microphone, submit]
            if inThread {
                controls.append(broadcast); broadcast.hover()
                let cancelReply = pane.buttons["Cancel reply"]
                if cancelReply.exists { controls.append(cancelReply); cancelReply.hover() }
            }
            microphone.hover(); submit.hover()
            for (index, control) in controls.enumerated() {
                XCTAssertTrue(control.exists)
                XCTAssertGreaterThan(control.frame.width, 0); XCTAssertGreaterThan(control.frame.height, 0)
                XCTAssertTrue(bounds.contains(control.frame), "\(control.identifier) must stay within its own composer pane")
                for neighbor in controls.dropFirst(index + 1) {
                    let overlap = control.frame.intersection(neighbor.frame)
                    XCTAssertTrue(overlap.isNull || overlap.isEmpty, "\(control.identifier) must not overlap \(neighbor.identifier)")
                }
            }
            if bounds.width < 250 {
                for control in controls.dropFirst().prefix(3) {
                    XCTAssertLessThanOrEqual(control.frame.maxY, microphone.frame.minY,
                        "Attach, GIF and emoji must wrap above microphone/send in the very narrow split")
                }
            }
        }
        func assertSplit(threadWidth: CGFloat, showingMembers: Bool) throws {
            XCTAssertTrue(channel.exists); XCTAssertTrue(composer.exists); XCTAssertTrue(divider.exists)
            XCTAssertEqual(members.exists, showingMembers)
            XCTAssertEqual(app.sliders["members-panel-resize"].exists, showingMembers)
            XCTAssertEqual(timeline.frame.width, threadWidth, accuracy: 1)
            XCTAssertEqual(pane.frame.width, threadWidth, accuracy: 1)
            XCTAssertEqual(channel.frame.maxX, pane.frame.minX, accuracy: 1)
            XCTAssertLessThanOrEqual(channel.frame.maxX, timeline.frame.minX)
            let available = window.frame.maxX - channel.frame.minX - (showingMembers ? members.frame.width : 0)
            let maximum = min(CGFloat(600), available - min(CGFloat(300), available / 2))
            XCTAssertEqual(channel.frame.width + pane.frame.width, available, accuracy: 1)
            XCTAssertGreaterThanOrEqual(channel.frame.width, min(CGFloat(300), available / 2) - 1)
            XCTAssertGreaterThanOrEqual(pane.frame.width, min(CGFloat(300), maximum) - 1)
            XCTAssertLessThanOrEqual(pane.frame.width, maximum + 1)
            if showingMembers {
                XCTAssertEqual(members.frame.width, 220, accuracy: 1)
                XCTAssertEqual(pane.frame.maxX, members.frame.minX, accuracy: 1)
                XCTAssertLessThanOrEqual(timeline.frame.maxX, members.frame.minX)
                XCTAssertEqual(members.frame.maxX, window.frame.maxX, accuracy: 1)
                app.buttons["Search members"].hover(); app.buttons["Close member list"].hover()
            } else { XCTAssertEqual(pane.frame.maxX, window.frame.maxX, accuracy: 1) }
            try assertComposerGeometry(inThread: false); try assertComposerGeometry(inThread: true)
            app.buttons["channel-pins"].hover(); membersToggle.hover(); divider.hover(); back.hover()
        }
        // Inspect the native entrypoints without requesting microphone access
        // or recording the Mac's surroundings during fixture UI flows.
        try channelAccessory("composer-emoji").hover(); channelMicrophone.hover()
        app.buttons["send-message"].hover(); threadMicrophone.hover(); send.hover()
        XCTAssertLessThanOrEqual(channelMicrophone.frame.maxX, app.buttons["send-message"].frame.minX)
        XCTAssertLessThanOrEqual(threadMicrophone.frame.maxX, send.frame.minX)
        XCTAssertEqual(composer.value as? String, channelDraft)
        capture("dictation entrypoints and independent composers wide", app: app)
        XCTAssertLessThanOrEqual(channel.frame.maxX, timeline.frame.minX, "Wide windows show both timelines without overlap")
        XCTAssertTrue(divider.exists); XCTAssertEqual(divider.value as? String, "340 pixels")
        app.buttons["channel-pins"].hover(); back.hover(); divider.hover()
        let handle = divider.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        handle.press(forDuration: 0.15, thenDragTo: handle.withOffset(CGVector(dx: timeline.frame.width - 380, dy: 0)))
        XCTAssertEqual(timeline.frame.width, 380, accuracy: 3, "Dragging the divider resizes the native thread pane")
        divider.click(); divider.typeKey(.rightArrow, modifierFlags: [])
        XCTAssertEqual(timeline.frame.width, 370, accuracy: 3, "Like shipping SwiftUI, Right narrows by ten")
        divider.typeKey(.home, modifierFlags: [])
        XCTAssertEqual(timeline.frame.width, 300, accuracy: 1)
        capture("thread minimum width and focused handle", app: app)
        divider.typeKey(.end, modifierFlags: [])
        XCTAssertEqual(timeline.frame.width, 420, accuracy: 1, "1280 minus 340 navigation and 220 members leaves a 720pt split")
        XCTAssertGreaterThanOrEqual(channel.frame.width, 300)
        try assertSplit(threadWidth: 420, showingMembers: true)
        capture("thread maximum width and adjacent channel", app: app)
        divider.doubleClick()
        XCTAssertEqual(timeline.frame.width, 340, accuracy: 1)
        divider.typeKey(.leftArrow, modifierFlags: [])
        divider.typeKey(.leftArrow, modifierFlags: [])
        divider.typeKey(.leftArrow, modifierFlags: [])
        XCTAssertEqual(timeline.frame.width, 370, accuracy: 1)
        XCTAssertLessThanOrEqual(channel.frame.maxX, timeline.frame.minX)
        XCTAssertEqual(editor.value as? String, "")
        XCTAssertFalse(pane.buttons["Cancel reply"].exists, "Reply in thread opens the root without quoting it")
        XCTAssertFalse(send.isEnabled); XCTAssertEqual(broadcast.value as? String, "0")
        editor.click(); editor.typeText(threadDraft)
        seedBody.rightClick()
        let quoteAction = messageMenuItem("Reply", app: app, body: seedBody)
        XCTAssertTrue(quoteAction.waitForExistence(timeout: 5)); quoteAction.click()
        XCTAssertTrue(pane.buttons["Cancel reply"].waitForExistence(timeout: 5))
        XCTAssertTrue(pane.staticTexts["Replying to \(seedAuthor)"].exists)
        XCTAssertEqual(editor.value as? String, threadDraft)
        XCTAssertEqual(composer.value as? String, channelDraft, "Reply inside a thread must not target the channel composer")
        capture("thread quote and independent channel draft wide", app: app)

        let sidebar = app.sliders["channel-sidebar-resize"]
        sidebar.click(); sidebar.typeKey(.end, modifierFlags: [])
        XCTAssertTrue(channel.exists, "Above 760, even maximum navigation must keep both conversations accessible")
        XCTAssertTrue(divider.exists, "Sidebar width never folds the thread split")
        XCTAssertTrue(editor.isHittable); XCTAssertEqual(editor.value as? String, threadDraft)
        XCTAssertTrue(app.sliders["members-panel-resize"].exists)
        try assertSplit(threadWidth: 280, showingMembers: true)
        XCTAssertEqual(composer.value as? String, channelDraft)
        capture("1280 maximum sidebar keeps two bounded 280pt conversation panes", app: app)
        sidebar.typeKey(.home, modifierFlags: [])
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(timeline.frame.width, 370, accuracy: 1, "A temporary layout clamp never replaces the preferred thread width")
        XCTAssertEqual(composer.value as? String, channelDraft)
        XCTAssertEqual(editor.value as? String, threadDraft)
        XCTAssertLessThanOrEqual(channel.frame.maxX, timeline.frame.minX)
        try assertSplit(threadWidth: 370, showingMembers: true)
        sidebar.doubleClick()

        // Check automatic member docking before any explicit show/hide choice.
        // Exact window widths distinguish the two sides of the 760/761 boundary.
        for width in [CGFloat(900), 761, 760, 390] {
            resize(app, width: width)
            XCTAssertEqual(window.frame.width, width, accuracy: 0.5)
            XCTAssertTrue(editor.isHittable)
            XCTAssertEqual(editor.value as? String, threadDraft)
            XCTAssertTrue(pane.buttons["Cancel reply"].exists)
            if width > 760 {
                try assertSplit(threadWidth: width == 900 ? 170 : 210.5, showingMembers: width == 900)
                XCTAssertEqual(channel.frame.width, timeline.frame.width, accuracy: 1)
                XCTAssertEqual(composer.value as? String, channelDraft)
                capture("\(Int(width)) split thread and channel with wrapped accessories and real neighbor hover", app: app)
            } else {
                XCTAssertFalse(composer.exists); XCTAssertFalse(channel.exists, "At or below 760 only the open thread is accessible")
                XCTAssertFalse(channelMicrophone.exists, "The folded channel's microphone must leave accessibility too")
                XCTAssertFalse(divider.exists); XCTAssertFalse(sidebar.exists); XCTAssertFalse(members.exists)
                XCTAssertFalse(app.sliders["members-panel-resize"].exists)
                XCTAssertEqual(pane.frame.minX, window.frame.minX, accuracy: 1)
                XCTAssertEqual(pane.frame.width, width, accuracy: 1)
                XCTAssertEqual(timeline.frame.width, width, accuracy: 1)
                XCTAssertTrue(threadMicrophone.isHittable)
                XCTAssertGreaterThanOrEqual(threadMicrophone.frame.minX, pane.frame.minX)
                XCTAssertLessThanOrEqual(threadMicrophone.frame.maxX, send.frame.minX)
                XCTAssertEqual(back.label, "Back to channel")
                try assertComposerGeometry(inThread: true)
                back.hover(); pane.buttons["Cancel reply"].hover()
                capture("\(Int(width)) folded fullwidth thread quote and composer", app: app)
            }
        }
        back.click()
        XCTAssertTrue(pane.waitForNonExistence(timeout: 5))
        XCTAssertTrue(composer.waitForExistence(timeout: 5)); XCTAssertEqual(composer.value as? String, channelDraft)
        XCTAssertTrue(channelMicrophone.waitForExistence(timeout: 5)); XCTAssertTrue(channelMicrophone.isHittable)
        XCTAssertFalse(threadMicrophone.exists)
        XCTAssertEqual(channel.frame.minX, window.frame.minX, accuracy: 1)
        XCTAssertEqual(channel.frame.width, 390, accuracy: 1)
        try assertComposerGeometry(inThread: false)
        capture("390 close thread returns fullwidth channel and exact unsent draft", app: app)
        let browse = app.buttons["open-navigation"]
        let closeBrowse = app.buttons["close-navigation"]
        for cycle in 1...2 {
            composer.click()
            let focused = XCTNSPredicateExpectation(predicate: NSPredicate(format: "hasKeyboardFocus == true"), object: composer)
            XCTAssertEqual(XCTWaiter.wait(for: [focused], timeout: 5), .completed)
            XCTAssertTrue(browse.isHittable); browse.hover(); browse.click()
            XCTAssertTrue(closeBrowse.waitForExistence(timeout: 5))
            XCTAssertFalse(channel.exists); XCTAssertFalse(composer.exists); XCTAssertFalse(channelMicrophone.exists)
            XCTAssertFalse(pane.exists); XCTAssertFalse(divider.exists)
            let channelsScroll = app.scrollViews["sidebar-channels-scroll"]
            let directScroll = app.scrollViews["sidebar-direct-scroll"]
            XCTAssertTrue(channelsScroll.exists); XCTAssertTrue(directScroll.exists)
            // The fullwidth navigation includes its existing 60pt space rail.
            XCTAssertEqual(channelsScroll.frame.minX - window.frame.minX, 60, accuracy: 1)
            XCTAssertEqual(channelsScroll.frame.maxX, window.frame.maxX, accuracy: 1)
            XCTAssertEqual(directScroll.frame.width, channelsScroll.frame.width, accuracy: 1)
            XCTAssertTrue(closeBrowse.isHittable); closeBrowse.hover()
            capture("390 fullwidth Browse cycle \(cycle) hides channel accessibility", app: app)
            closeBrowse.click()
            XCTAssertTrue(closeBrowse.waitForNonExistence(timeout: 5))
            XCTAssertTrue(composer.waitForExistence(timeout: 5))
            XCTAssertEqual(composer.value as? String, channelDraft)
            XCTAssertEqual(channel.frame.width, 390, accuracy: 1)
            XCTAssertFalse(NSPredicate(format: "hasKeyboardFocus == true").evaluate(with: composer),
                "Closing Browse must not steal keyboard focus back into the retained composer")
            try assertComposerGeometry(inThread: false)
            capture("390 Browse closed cycle \(cycle) preserves exact draft without autofocus", app: app)
        }
        openOtherRoot()
        XCTAssertEqual(editor.value as? String, ""); XCTAssertFalse(pane.buttons["Cancel reply"].exists)
        editor.click(); editor.typeText(otherDraft)
        back.click()
        XCTAssertTrue(summary.waitForExistence(timeout: 5)); summary.click()
        XCTAssertTrue(editor.waitForExistence(timeout: 5)); XCTAssertEqual(editor.value as? String, threadDraft)
        XCTAssertTrue(pane.buttons["Cancel reply"].exists, "Returning to a root restores its quote as well as its text")

        resize(app, width: 1280)
        XCTAssertEqual(window.frame.width, 1280, accuracy: 0.5)
        try assertSplit(threadWidth: 370, showingMembers: true)
        XCTAssertEqual(divider.value as? String, "370 pixels", "390/760/761/900 clamps must not save over the preferred width")
        XCTAssertEqual(composer.value as? String, channelDraft); XCTAssertEqual(editor.value as? String, threadDraft)
        membersToggle.click()
        XCTAssertTrue(members.waitForNonExistence(timeout: 5))
        try assertSplit(threadWidth: 370, showingMembers: false)
        capture("1280 thread with members hidden and real neighboring hover", app: app)
        membersToggle.click()
        XCTAssertTrue(members.waitForExistence(timeout: 5))
        try assertSplit(threadWidth: 370, showingMembers: true)
        XCTAssertEqual(composer.value as? String, channelDraft); XCTAssertEqual(editor.value as? String, threadDraft)
        XCTAssertTrue(pane.buttons["Cancel reply"].exists)
        capture("1280 restored preferred thread width with members shown and no overlap", app: app)
        // An explicitly opened member overlay is separate from automatic docking;
        // close it before continuing the existing folded send/retry lifecycle.
        app.buttons["Close member list"].click()
        XCTAssertTrue(members.waitForNonExistence(timeout: 5))
        resize(app, width: 760)
        XCTAssertEqual(window.frame.width, 760, accuracy: 0.5)
        XCTAssertFalse(composer.exists); XCTAssertFalse(channel.exists); XCTAssertFalse(divider.exists)
        XCTAssertFalse(members.exists); XCTAssertEqual(pane.frame.width, 760, accuracy: 1)
        XCTAssertEqual(editor.value as? String, threadDraft)

        try await Self.control(["failure": ["path": path, "method": "POST", "status": 503,
            "error": "TEST FIXTURE: reply send unavailable."]])
        send.click()
        let pending = pane.groups.matching(NSPredicate(format: "identifier BEGINSWITH %@", "pending-message-")).firstMatch
        XCTAssertTrue(pending.waitForExistence(timeout: 10))
        let retrySend = pending.buttons["Retry send"]
        XCTAssertTrue(retrySend.waitForExistence(timeout: 10))
        XCTAssertTrue(pending.staticTexts.matching(NSPredicate(format: "label CONTAINS %@ OR value CONTAINS %@",
            "TEST FIXTURE: reply send unavailable.", "TEST FIXTURE: reply send unavailable.")).firstMatch.exists)
        let pendingID = String(pending.identifier.dropFirst("pending-message-".count))
        let failed = try await Self.fixtureGET(threadPath)
        XCTAssertEqual((failed["messages"] as? [[String: Any]])?.count, 1, "A rejected transport must not invent a confirmed reply")
        capture("thread send failure narrow", app: app)
        back.click(); openOtherRoot()
        XCTAssertEqual(editor.value as? String, otherDraft)
        XCTAssertFalse(pending.exists, "Another root must not show the first root's failed message")
        XCTAssertFalse(send.isEnabled, "An unresolved send cannot be resubmitted as a different root's draft")
        back.click()
        XCTAssertTrue(composer.waitForExistence(timeout: 5)); XCTAssertEqual(composer.value as? String, channelDraft)
        XCTAssertFalse(channel.groups.matching(NSPredicate(format: "identifier BEGINSWITH %@", "pending-message-")).firstMatch.exists)
        summary.click()
        XCTAssertTrue(retrySend.waitForExistence(timeout: 5)); retrySend.click()
        XCTAssertTrue(pending.waitForNonExistence(timeout: 10))
        let privateBody = timeline.textViews.matching(NSPredicate(format: "value == %@", threadDraft)).firstMatch
        XCTAssertTrue(privateBody.waitForExistence(timeout: 10))
        let privateID = String(privateBody.identifier.dropFirst("message-".count))
        let recovered = try await Self.fixtureGET(threadPath)
        let privateRows = try XCTUnwrap(recovered["messages"] as? [[String: Any]])
        XCTAssertEqual(privateRows.count, 2, "Retry confirms exactly one reply")
        let privateReply = try XCTUnwrap(privateRows.first { $0["id"] as? String == privateID })
        XCTAssertEqual(privateReply["clientMessageId"] as? String, pendingID.lowercased(), "Retry keeps the original idempotency key")
        XCTAssertEqual(privateReply["threadRootId"] as? String, rootID)
        XCTAssertEqual(privateReply["broadcast"] as? Bool, false)
        XCTAssertEqual((privateReply["replyTo"] as? [String: Any])?["id"] as? String, seedID)
        let privateChannel = try await Self.fixtureGET(path)
        XCTAssertEqual((privateChannel["messages"] as? [[String: Any]])?.count, initialChannelCount)
        XCTAssertFalse(try XCTUnwrap(privateChannel["messages"] as? [[String: Any]]).contains { $0["id"] as? String == privateID })
        XCTAssertEqual(editor.value as? String, ""); XCTAssertFalse(pane.buttons["Cancel reply"].exists)
        XCTAssertEqual(broadcast.value as? String, "0")

        let sharedText = "TEST FIXTURE — one reply shared with the channel"
        editor.click(); editor.typeText(sharedText)
        timeline.scroll(byDeltaX: 0, deltaY: 1000)
        XCTAssertTrue(seedBody.waitForExistence(timeout: 5)); seedBody.rightClick()
        let quoteAgain = messageMenuItem("Reply", app: app, body: seedBody)
        XCTAssertTrue(quoteAgain.waitForExistence(timeout: 5)); quoteAgain.click()
        broadcast.click(); XCTAssertEqual(broadcast.value as? String, "1")
        send.click()
        timeline.scroll(byDeltaX: 0, deltaY: -10000)
        let sharedBody = timeline.textViews.matching(NSPredicate(format: "value == %@", sharedText)).firstMatch
        XCTAssertTrue(sharedBody.waitForExistence(timeout: 10))
        let sharedID = String(sharedBody.identifier.dropFirst("message-".count))
        let sharedThread = try await Self.fixtureGET(threadPath)
        let sharedRows = try XCTUnwrap(sharedThread["messages"] as? [[String: Any]])
        let sharedReply = try XCTUnwrap(sharedRows.first { $0["id"] as? String == sharedID })
        XCTAssertEqual(sharedRows.count, 3)
        XCTAssertEqual(sharedReply["threadRootId"] as? String, rootID)
        XCTAssertEqual(sharedReply["broadcast"] as? Bool, true)
        XCTAssertEqual((sharedReply["replyTo"] as? [String: Any])?["id"] as? String, seedID)
        let sharedChannel = try await Self.fixtureGET(path)
        let channelRows = try XCTUnwrap(sharedChannel["messages"] as? [[String: Any]])
        XCTAssertEqual(channelRows.count, initialChannelCount + 1, "Broadcast must not create a separate channel copy")
        XCTAssertEqual(channelRows.filter { $0["id"] as? String == sharedID }.count, 1, "Broadcast is the same server message, not a second send")
        XCTAssertFalse(channelRows.contains { $0["id"] as? String == privateID })
        XCTAssertEqual(broadcast.value as? String, "0", "Broadcast applies to one send only")
        resize(app, width: 1280)
        XCTAssertTrue(composer.waitForExistence(timeout: 5)); XCTAssertEqual(composer.value as? String, channelDraft)
        XCTAssertEqual(timeline.frame.width, 370, accuracy: 1, "Reopening roots at narrow widths retains the saved split width")
        channel.scroll(byDeltaX: 0, deltaY: -10000)
        let channelShared = channel.textViews["message-\(sharedID)"]
        XCTAssertTrue(channelShared.waitForExistence(timeout: 5))
        XCTAssertTrue(timeline.textViews["message-\(sharedID)"].exists)
        XCTAssertFalse(channel.textViews["message-\(privateID)"].exists)
        let sharedLink = channel.buttons["thread-summary-\(sharedID)"]
        XCTAssertEqual(sharedLink.label, "View thread")
        capture("broadcast shared between native timelines wide", app: app)
        channel.scroll(byDeltaX: 0, deltaY: 10000)
        let rootRecord = try XCTUnwrap(sharedThread["root"] as? [String: Any])
        let count = try XCTUnwrap((rootRecord["thread"] as? [String: Any])?["replyCount"] as? Int)
        let summaryUpdated = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "\(count) replies"), object: summary)
        XCTAssertEqual(XCTWaiter.wait(for: [summaryUpdated], timeout: 5), .completed)

        channel.scroll(byDeltaX: 0, deltaY: -10000)
        XCTAssertTrue(channelShared.waitForExistence(timeout: 5))
        channelShared.rightClick()
        let channelReply = messageMenuItem("Reply", app: app, body: channelShared)
        XCTAssertTrue(channelReply.waitForExistence(timeout: 5)); channelReply.click()
        XCTAssertTrue(pane.waitForNonExistence(timeout: 5), "Reply on a channel broadcast still targets the channel")
        XCTAssertTrue(app.buttons["Cancel reply"].waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, channelDraft)
        app.buttons["Cancel reply"].click()
        sharedLink.click()
        XCTAssertTrue(editor.waitForExistence(timeout: 5)); XCTAssertEqual(editor.value as? String, "")
        XCTAssertTrue(timeline.textViews["message-\(sharedID)"].waitForExistence(timeout: 5), "View thread opens the original root, not a nested thread")
        back.click()

        // The broadcast's quote is a real deep link into an older thread page.
        for index in 1...55 {
            try await Self.control(["incomingReply": ["rootId": rootID, "text": "Newer thread reply \(index)"]])
        }
        let newestPage = try await Self.fixtureGET(threadPath)
        let newest = try XCTUnwrap((newestPage["messages"] as? [[String: Any]])?.last)
        let newestID = try XCTUnwrap(newest["id"] as? String)
        let sharedRow = channel.descendants(matching: .any)["message-row-\(sharedID)"]
        let original = sharedRow.buttons["View original message from \(seedAuthor)"]
        XCTAssertTrue(original.waitForExistence(timeout: 5)); original.click()
        XCTAssertTrue(seedBody.waitForExistence(timeout: 10)); XCTAssertTrue(seedBody.isHittable)
        XCTAssertEqual(seedBody.value as? String, seedText)
        let newer = app.buttons["load-newer-replies"]
        XCTAssertTrue(newer.waitForExistence(timeout: 5))
        capture("quoted original thread reply with newer page available", app: app)
        newer.click()
        XCTAssertTrue(newer.waitForNonExistence(timeout: 10))
        timeline.scroll(byDeltaX: 0, deltaY: -10000)
        XCTAssertTrue(timeline.textViews["message-\(newestID)"].waitForExistence(timeout: 10))
        XCTAssertEqual(composer.value as? String, channelDraft)
        XCTAssertFalse(channel.textViews["message-\(newestID)"].exists, "Live thread-only gateway rows never enter the channel timeline")
        capture("thread newer replies loaded without changing channel draft", app: app)
        back.click(); openOtherRoot()
        XCTAssertEqual(editor.value as? String, otherDraft, "Sending and navigating the first root never consume the second root's draft")
        back.click()
        channel.scroll(byDeltaX: 0, deltaY: 10000)
        let ownRoot = channel.textViews["message-\(otherRootID)"]
        XCTAssertTrue(ownRoot.waitForExistence(timeout: 5)); ownRoot.rightClick()
        messageMenuItem("Delete message", app: app, body: ownRoot).click()
        XCTAssertTrue(app.staticTexts["Delete message?"].waitForExistence(timeout: 5)); app.buttons["Delete"].click()
        let tombstone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "This message was deleted."), object: ownRoot)
        XCTAssertEqual(XCTWaiter.wait(for: [tombstone], timeout: 10), .completed)
        let survivingThread = channel.buttons["thread-summary-\(otherRootID)"]
        XCTAssertTrue(survivingThread.exists, "Deleting a root preserves access to its replies")
        ownRoot.rightClick()
        let viewThread = messageMenuItem("View thread", app: app, body: ownRoot)
        XCTAssertTrue(viewThread.waitForExistence(timeout: 5))
        XCTAssertFalse(messageMenuItem("Reply", app: app, body: ownRoot).exists)
        viewThread.click()
        XCTAssertTrue(timeline.textViews.matching(NSPredicate(format: "value == %@", "Second root reply")).firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(editor.value as? String, otherDraft)
        capture("surviving replies beneath a deleted root", app: app)
        back.click(); survivingThread.click()
        XCTAssertTrue(editor.waitForExistence(timeout: 5)); XCTAssertEqual(editor.value as? String, otherDraft)
        XCTAssertEqual(divider.value as? String, "370 pixels")
        app.terminate()
        let relaunched = launch(preservePanelWidths: true)
        defer { relaunched.terminate() }
        XCTAssertTrue(relaunched.textViews["message-composer"].waitForExistence(timeout: 15))
        resize(relaunched, width: 1280)
        let history = relaunched.scrollViews["message-timeline"]
        history.scroll(byDeltaX: 0, deltaY: 10000)
        let restoredRoot = history.buttons["thread-summary-\(rootID)"]
        XCTAssertTrue(restoredRoot.waitForExistence(timeout: 5)); restoredRoot.click()
        let restored = relaunched.sliders["thread-panel-resize"]
        XCTAssertTrue(restored.waitForExistence(timeout: 5))
        XCTAssertEqual(restored.value as? String, "370 pixels", "A real relaunch reads the committed width, not the launch override")
        XCTAssertEqual(relaunched.scrollViews["thread-timeline"].frame.width, 370, accuracy: 1)
        capture("thread preference restored after relaunch", app: relaunched)
        restored.doubleClick()
    }

    func testOwnMessageActionsEditConflictHistoryAndDeleteConfirmation() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        func replace(_ field: XCUIElement, with text: String) {
            NSPasteboard.general.clearContents(); NSPasteboard.general.setString(text, forType: .string)
            field.click(); field.typeKey("a", modifierFlags: .command); field.typeKey("v", modifierFlags: .command)
            XCTAssertEqual(field.value as? String, text)
        }
        let link = "https://example.test/left?q=one#tail"
        let original = "TEST FIXTURE — α-left 👩🏽‍💻 café e\u{301}\n東京 → \(link)\nيمين — END-7"
        replace(composer, with: original)
        app.buttons["send-message"].click()
        let delivered = app.textViews.matching(NSPredicate(format: "value == %@ AND identifier != %@", original, "message-composer")).firstMatch
        XCTAssertTrue(delivered.waitForExistence(timeout: 10), "Send must reach the fixture API and gateway")
        XCTAssertTrue(delivered.identifier.hasPrefix("message-"))
        let id = String(delivered.identifier.dropFirst("message-".count))
        XCTAssertEqual(id.count, 15, "Actions must target the confirmed server ID, not a pending row")
        let body = app.textViews["message-\(id)"]
        let row = app.descendants(matching: .any)["message-row-\(id)"]
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, "")
        func item(_ title: String) -> XCUIElement { messageMenuItem(title, app: app, body: body) }
        func options() {
            row.hover()
            let button = app.buttons["message-options-\(id)"]
            XCTAssertTrue(button.waitForExistence(timeout: 5)); XCTAssertTrue(button.isHittable)
            button.hover(); button.click()
            XCTAssertTrue(item("Copy message ID").waitForExistence(timeout: 5))
        }
        options()
        for title in ["Add reaction…", "Edit message", "Copy text", "Open link", "Copy link", "Copy message ID", "Delete message"] {
            XCTAssertTrue(item(title).exists, "Missing own-message action: \(title)")
            XCTAssertTrue(item(title).isEnabled)
        }
        XCTAssertFalse(item("Message history").exists, "An unedited message has no history action")
        XCTAssertFalse(app.buttons["message-history-\(id)"].exists)
        for title in ["Forward…"] {
            XCTAssertFalse(item(title).exists, "Unimplemented actions must not appear as placeholders")
        }
        capture("own message options wide", app: app)
        NSPasteboard.general.clearContents(); item("Copy text").click()
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), original, "Copy text preserves asymmetric Unicode and line breaks")
        options(); NSPasteboard.general.clearContents(); item("Copy message ID").click()
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), id)
        body.rightClick()
        XCTAssertTrue(item("Copy link").waitForExistence(timeout: 5))
        // Check the real Open target's availability, but never launch an external browser.
        XCTAssertTrue(item("Open link").isEnabled)
        NSPasteboard.general.clearContents(); item("Copy link").click()
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), link, "Copy link must exclude surrounding non-ASCII text")

        options(); item("Edit message").click()
        let editor = app.textViews["message-edit-text"]
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        XCTAssertEqual(editor.value as? String, original)
        XCTAssertFalse(composer.exists, "The on-demand editor hides background controls from accessibility")
        let save = app.buttons["Save changes"]
        replace(editor, with: " \n\t")
        XCTAssertFalse(save.isEnabled, "Whitespace-only edits cannot be saved")
        replace(editor, with: String(repeating: "e\u{301}", count: 2000) + "🚀")
        XCTAssertFalse(save.isEnabled, "The 4,000-character limit counts Unicode scalars, not graphemes")
        capture("message editor invalid draft wide", app: app)
        app.buttons["Cancel"].click()
        XCTAssertTrue(editor.waitForNonExistence(timeout: 5))
        XCTAssertEqual(body.value as? String, original, "Cancelling an edit must retain the original")
        XCTAssertFalse(app.buttons["message-history-\(id)"].exists)

        options(); item("Edit message").click()
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        let edited = "TEST FIXTURE — β-right 🥑\n\(link)\n終わり — END-29"
        replace(editor, with: edited)
        XCTAssertTrue(save.isEnabled)
        let latest = original + "\nTEST FIXTURE — changed in another client"
        try await Self.control(["incomingEdit": ["channelId": "chan00000001", "messageId": id, "text": latest]])
        save.click()
        let conflict = app.staticTexts["message changed; reload it before editing again"]
        XCTAssertTrue(conflict.waitForExistence(timeout: 10), "The fixture must reject the stale expectedRevision")
        XCTAssertEqual(editor.value as? String, edited, "A conflict must not discard the local draft")
        capture("message edit conflict wide", app: app)
        app.buttons["Discard draft and load latest"].click()
        let reloaded = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", latest), object: editor)
        XCTAssertEqual(XCTWaiter.wait(for: [reloaded], timeout: 10), .completed)
        XCTAssertFalse(conflict.exists)
        replace(editor, with: edited); save.click()
        XCTAssertTrue(editor.waitForNonExistence(timeout: 10))
        let saved = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", edited), object: body)
        XCTAssertEqual(XCTWaiter.wait(for: [saved], timeout: 10), .completed)
        let history = app.buttons["message-history-\(id)"]
        XCTAssertTrue(history.waitForExistence(timeout: 5)); history.click()
        XCTAssertTrue(app.staticTexts["Message history"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Previous version"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Current version"].exists)
        for text in [latest, edited] {
            XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label == %@ OR value == %@", text, text)).firstMatch.exists,
                "History must show both the previous and current text, not just revision labels")
        }
        capture("message history old and new wide", app: app)
        app.buttons["Close"].click()
        XCTAssertTrue(body.waitForExistence(timeout: 5))
        resize(app, width: 900)
        options(); XCTAssertTrue(item("Message history").exists)
        capture("edited message options narrow", app: app)
        item("Message history").click()
        XCTAssertTrue(app.staticTexts["Previous version"].waitForExistence(timeout: 10))
        capture("message history narrow", app: app)
        app.buttons["Close"].click()

        options(); item("Delete message").click()
        XCTAssertTrue(app.staticTexts["Delete message?"].waitForExistence(timeout: 5))
        capture("delete message confirmation narrow", app: app)
        app.buttons["Cancel"].click()
        XCTAssertTrue(body.waitForExistence(timeout: 5))
        XCTAssertEqual(body.value as? String, edited, "Cancel leaves the saved message intact")
        try await Self.control(["failure": ["path": "/api/chat/channels/chan00000001/messages/\(id)", "method": "DELETE",
            "status": 503, "error": "TEST FIXTURE: deletion unavailable."]])
        options(); item("Delete message").click(); app.buttons["Delete"].click()
        let error = app.staticTexts["conversation-error"]
        XCTAssertTrue(error.waitForExistence(timeout: 10))
        XCTAssertEqual(error.value as? String, "Couldn’t delete the message. Try again.")
        XCTAssertEqual(body.value as? String, edited, "A rejected deletion must not manufacture a tombstone")
        capture("delete request failed narrow", app: app)
        options(); item("Delete message").click(); app.buttons["Delete"].click()
        let deleted = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "This message was deleted."), object: body)
        XCTAssertEqual(XCTWaiter.wait(for: [deleted], timeout: 10), .completed)
        XCTAssertTrue(row.exists, "Deletion keeps the server row as a tombstone")
        XCTAssertFalse(history.exists)
        options()
        XCTAssertTrue(item("Copy message ID").waitForExistence(timeout: 5))
        let tombstoneMenu = app.menus.containing(.menuItem, identifier: "Copy message ID").firstMatch
        XCTAssertEqual(tombstoneMenu.menuItems.allElementsBoundByIndex.map(\.label), ["Copy message ID"])
        for title in ["Add reaction…", "Forward message", "Edit message", "Message history", "Copy text", "Open link", "Copy link", "Delete message", "Pin message", "Unpin message"] {
            XCTAssertFalse(item(title).exists, "A tombstone only offers Copy message ID")
        }
        capture("tombstone options menu narrow", app: app)
        NSPasteboard.general.clearContents(); item("Copy message ID").click()
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), id)
        body.rightClick()
        XCTAssertTrue(item("Copy message ID").waitForExistence(timeout: 5))
        // Native text-selection commands may remain for the placeholder. Only
        // message actions are restricted; they must not resurrect deleted text.
        for title in ["Add reaction…", "Forward message", "Edit message", "Message history", "Copy text", "Open link", "Copy link", "Delete message", "Pin message", "Unpin message"] {
            XCTAssertFalse(item(title).exists, "Right-click must not restore actions on deleted content")
        }
        app.typeKey(.escape, modifierFlags: [])
        let peer = app.textViews["message-chan00000001m02"] // Maya, not the signed-in owner.
        XCTAssertTrue(peer.waitForExistence(timeout: 5)); peer.rightClick()
        XCTAssertTrue(messageMenuItem("Copy text", app: app, body: peer).waitForExistence(timeout: 5))
        XCTAssertTrue(messageMenuItem("Add reaction…", app: app, body: peer).exists)
        XCTAssertTrue(messageMenuItem("Block Maya…", app: app, body: peer).exists)
        for title in ["Edit message", "Delete message", "Message history"] {
            XCTAssertFalse(messageMenuItem(title, app: app, body: peer).exists, "Ownership must be checked for the menu's target")
        }
        capture("peer context menu narrow", app: app)
        app.typeKey(.escape, modifierFlags: [])
    }

    func testMessageQuickReactionsPickerAndTextSelectionStayIndependent() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let id = "chan00000001m01" // Labelled owner message in the HTTP fixture, not a local reaction overlay.
        try await Self.control(["incomingReaction": ["channelId": "chan00000001", "messageId": id,
            "emoji": "👍", "userId": "member000001", "active": true]])
        let app = launch()
        defer { app.terminate() }
        let body = app.textViews["message-\(id)"]
        XCTAssertTrue(body.waitForExistence(timeout: 15))
        let row = app.descendants(matching: .any)["message-row-\(id)"]
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        let composer = app.textViews["message-composer"]
        composer.click(); composer.typeText("TEST FIXTURE — leave this draft alone")
        row.hover()
        let quick = ["👍", "❤️", "😂"].map { app.buttons["quick-reaction-\(id)-\($0)"] }
        for button in quick {
            XCTAssertTrue(button.waitForExistence(timeout: 5)); XCTAssertTrue(button.isHittable)
            XCTAssertEqual(button.frame.midY, quick[0].frame.midY, accuracy: 1)
            // A live pixel check catches missing/blank artwork without an OS-dependent golden image.
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: button.screenshot().pngRepresentation))
            var coloured = 0
            for y in 0..<bitmap.pixelsHigh {
                for x in 0..<bitmap.pixelsWide {
                    if let colour = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB),
                       colour.saturationComponent > 0.4, colour.brightnessComponent > 0.5 { coloured += 1 }
                }
            }
            XCTAssertGreaterThan(coloured, 10, "Quick reactions must render colourful emoji artwork, not empty controls")
        }
        let add = app.buttons["add-reaction-\(id)"]
        let options = app.buttons["message-options-\(id)"]
        XCTAssertLessThanOrEqual(quick[2].frame.maxX, add.frame.minX)
        XCTAssertLessThanOrEqual(add.frame.maxX, options.frame.minX)
        let thumbBefore = quick[0].screenshot().pngRepresentation
        quick[1].hover()
        XCTAssertEqual(quick[0].screenshot().pngRepresentation, thumbBefore, "Hover must not highlight its neighbor")
        capture("quick reaction artwork and neighbor hover wide", app: app)
        let thumb = app.buttons["reaction-\(id)-👍"]
        XCTAssertEqual(thumb.label, "👍 1")
        for count in [2, 1] {
            row.hover(); quick[0].hover(); quick[0].click()
            let toggled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "👍 \(count)"), object: thumb)
            XCTAssertEqual(XCTWaiter.wait(for: [toggled], timeout: 10), .completed, "Quick reaction toggles only the current account")
        }
        row.hover(); add.hover(); options.hover(); add.click()
        let search = app.textFields["reaction-picker-search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.click(); search.typeText("no-such-emoji-zzzz")
        XCTAssertTrue(app.descendants(matching: .any)["reaction-picker-empty"].waitForExistence(timeout: 5))
        capture("message reaction picker empty wide", app: app)
        app.buttons["Cancel"].click()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        XCTAssertEqual(thumb.label, "👍 1")
        XCTAssertFalse(app.buttons["reaction-\(id)-🍅"].exists, "Cancel must not add a reaction")
        resize(app, width: 900)
        body.rightClick()
        let menuAdd = messageMenuItem("Add reaction…", app: app, body: body)
        XCTAssertTrue(menuAdd.waitForExistence(timeout: 5)); menuAdd.click()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        XCTAssertEqual(search.value as? String, "", "A new picker clears the cancelled query")
        search.click(); search.typeText("tomato")
        // The Touch Bar can also expose the catalog name; choose the actual grid button.
        let tomato = app.scrollViews["reaction-picker-grid"].buttons["tomato"]
        XCTAssertTrue(tomato.waitForExistence(timeout: 5)); XCTAssertTrue(tomato.isHittable)
        capture("message reaction tomato search narrow", app: app)
        tomato.click()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        let chip = app.buttons["reaction-\(id)-🍅"]
        XCTAssertTrue(chip.waitForExistence(timeout: 10)); XCTAssertEqual(chip.label, "🍅 1")
        // Re-enter through HTTP history so an optimistic-only chip cannot pass.
        app.buttons["channel-chan00000002"].click()
        XCTAssertTrue(app.staticTexts["# design"].waitForExistence(timeout: 10))
        app.buttons["channel-chan00000001"].click()
        XCTAssertTrue(chip.waitForExistence(timeout: 10)); XCTAssertEqual(chip.label, "🍅 1")
        XCTAssertEqual(thumb.label, "👍 1")

        let start = body.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 2, dy: 10))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 90, dy: 0)))
        NSPasteboard.general.clearContents(); app.typeKey("c", modifierFlags: .command)
        let selection = try XCTUnwrap(NSPasteboard.general.string(forType: .string))
        let text = try XCTUnwrap(body.value as? String)
        XCTAssertFalse(selection.isEmpty); XCTAssertLessThan(selection.count, text.count)
        XCTAssertTrue(text.contains(selection), "Dragging must select text, not invoke Copy text for the entire row")
        row.hover(); quick[0].hover(); quick[1].hover(); quick[2].hover(); add.hover(); options.hover()
        app.buttons["profile-author-chan00000001m02"].hover()
        NSPasteboard.general.clearContents(); app.typeKey("c", modifierFlags: .command)
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), selection, "Neighbor hovers must not steal the text selection or responder")
        try await Self.control(["incomingReaction": ["channelId": "chan00000001", "messageId": id,
            "emoji": "🍅", "userId": "member000002", "active": true]])
        let incoming = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "🍅 2"), object: chip)
        XCTAssertEqual(XCTWaiter.wait(for: [incoming], timeout: 10), .completed, "A peer reaction must arrive over the real fixture WebSocket")
        NSPasteboard.general.clearContents(); app.typeKey("c", modifierFlags: .command)
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), selection, "Live reaction updates preserve the native text selection")
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — leave this draft alone")
        XCTAssertFalse(search.exists); XCTAssertFalse(app.textViews["message-edit-text"].exists)
        XCTAssertFalse(app.buttons["reaction-chan00000001m02-🍅"].exists, "Reactions belong to the original target, not a hovered neighbor")
        capture("live reaction with selected message text narrow", app: app)
        chip.click()
        let removed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "🍅 1"), object: chip)
        XCTAssertEqual(XCTWaiter.wait(for: [removed], timeout: 10), .completed, "Clicking the chip removes only your own reaction")

        let failure: [String: Any] = ["path": "/api/chat/channels/chan00000001/messages/\(id)/reactions", "method": "PUT",
            "status": 503, "error": "TEST FIXTURE: reaction unavailable."]
        try await Self.control(["failure": failure])
        row.hover(); quick[0].click()
        let retry = app.buttons["reaction-retry-\(id)"]
        let dismiss = app.buttons["reaction-dismiss-\(id)"]
        XCTAssertTrue(retry.waitForExistence(timeout: 10)); XCTAssertTrue(dismiss.exists)
        XCTAssertEqual(thumb.label, "👍 1", "A failed mutation must roll back the optimistic count")
        capture("reaction failure retry narrow", app: app)
        retry.click()
        XCTAssertTrue(retry.waitForNonExistence(timeout: 10))
        let retried = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "👍 2"), object: thumb)
        XCTAssertEqual(XCTWaiter.wait(for: [retried], timeout: 10), .completed)
        try await Self.control(["failure": failure])
        row.hover(); quick[0].click()
        XCTAssertTrue(dismiss.waitForExistence(timeout: 10))
        dismiss.click()
        XCTAssertTrue(dismiss.waitForNonExistence(timeout: 5))
        XCTAssertEqual(thumb.label, "👍 2", "Dismiss clears the error, not the stored reaction")
    }

    func testComposerEmojiPickerSelectionUndoCancelAndScalarLimit() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch(fixture: "klipy")
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        let emoji = app.buttons["composer-emoji"]
        XCTAssertTrue(app.buttons["composer-gif"].waitForExistence(timeout: 10))
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: emoji)
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 10), .completed)
        let microphone = app.buttons["composer-voice-record"]
        XCTAssertTrue(microphone.waitForExistence(timeout: 5)); XCTAssertTrue(microphone.isHittable)
        XCTAssertLessThanOrEqual(emoji.frame.maxX, microphone.frame.minX)
        XCTAssertLessThanOrEqual(microphone.frame.maxX, app.buttons["send-message"].frame.minX)
        capture("composer accessories", app: app)
        func replaceDraft(_ text: String) {
            NSPasteboard.general.clearContents(); NSPasteboard.general.setString(text, forType: .string)
            composer.click(); composer.typeKey("a", modifierFlags: .command); composer.typeKey("v", modifierFlags: .command)
            XCTAssertEqual(composer.value as? String, text)
        }
        let original = "🚀 replace tail"
        replaceDraft(original)
        composer.typeKey(.leftArrow, modifierFlags: .command)
        composer.typeKey(.rightArrow, modifierFlags: []); composer.typeKey(.rightArrow, modifierFlags: [])
        for _ in 0..<7 { composer.typeKey(.rightArrow, modifierFlags: .shift) }
        app.buttons["composer-gif"].hover(); emoji.hover(); emoji.click()
        let search = app.textFields["reaction-picker-search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.click(); search.typeText("tomato")
        XCTAssertTrue(app.buttons["tomato"].waitForExistence(timeout: 5))
        capture("emoji search with saved selection", app: app)
        app.buttons["tomato"].click()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, "🚀 🍅 tail", "Insert at the saved UTF-16 selection, not the end")
        app.typeText("!")
        XCTAssertEqual(composer.value as? String, "🚀 🍅! tail", "Selection restores the composer and caret")
        app.typeKey("z", modifierFlags: .command)
        XCTAssertEqual(composer.value as? String, "🚀 🍅 tail")
        app.typeKey("z", modifierFlags: .command)
        XCTAssertEqual(composer.value as? String, original, "Emoji insertion must be one native undo step")
        emoji.click()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.click(); search.typeText("no-such-emoji-zzzz")
        XCTAssertTrue(app.descendants(matching: .any)["reaction-picker-empty"].waitForExistence(timeout: 5))
        capture("emoji empty search", app: app)
        app.buttons["Cancel"].click()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, original)
        emoji.click()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        app.buttons["channel-chan00000002"].click()
        XCTAssertTrue(app.staticTexts["# design"].waitForExistence(timeout: 10))
        XCTAssertFalse(search.exists)
        XCTAssertEqual(composer.value as? String, "", "Closing a picker during navigation must not change the new draft")
        app.buttons["channel-chan00000001"].click()
        XCTAssertTrue(app.staticTexts["# general"].waitForExistence(timeout: 10))
        XCTAssertEqual(composer.value as? String, original)
        // A combining sequence is two Unicode scalars, but one grapheme.
        replaceDraft(String(repeating: "e\u{301}", count: 1499) + "🚀")
        XCTAssertFalse(app.staticTexts["composer-counter"].exists)
        composer.typeText("x")
        XCTAssertTrue(app.staticTexts["3,000 / 4,000"].waitForExistence(timeout: 5))
        let full = String(repeating: "x", count: 3999) + "🚀"
        replaceDraft(full)
        XCTAssertTrue(app.buttons["send-message"].isEnabled)
        emoji.click()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.click(); search.typeText("tomato"); app.buttons["tomato"].click()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, full, "A picker must not insert a 4,001st scalar")
        app.typeText("x")
        XCTAssertTrue(app.staticTexts["4,001 / 4,000"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["send-message"].isEnabled)
        resize(app, width: 900)
        capture("composer counter over limit narrow", app: app)
        XCTAssertLessThanOrEqual(composer.frame.maxX, app.buttons["composer-gif"].frame.minX)
        XCTAssertLessThanOrEqual(app.buttons["composer-gif"].frame.maxX, emoji.frame.minX)
        XCTAssertLessThanOrEqual(emoji.frame.maxX, app.buttons["send-message"].frame.minX)
    }

    func testGifPickerRecoverySendViewerAndDraftRetention() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        try await Self.control(["uploadsEnabled": true])
        let source = try imageFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let app = launch(fixture: "klipy")
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        let gif = app.buttons["composer-gif"]
        XCTAssertTrue(gif.waitForExistence(timeout: 10))
        let available = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: app.buttons["attach-files-button"])
        XCTAssertEqual(XCTWaiter.wait(for: [available], timeout: 10), .completed)
        composer.click(); composer.typeText("TEST FIXTURE — keep this draft")
        NSPasteboard.general.clearContents(); NSPasteboard.general.writeObjects([source as NSURL])
        composer.typeKey("v", modifierFlags: .command)
        XCTAssertTrue(app.buttons["staged-file-remove"].waitForExistence(timeout: 10))
        gif.click()
        let search = app.textFields["gif-picker-search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["gif-picker-item-caper-fixture-gif"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["gif-picker-attribution"].exists)
        capture("GIF picker trending", app: app)
        search.click(); search.typeText("fail")
        XCTAssertTrue(app.buttons["gif-picker-retry"].waitForExistence(timeout: 10))
        capture("GIF picker provider error", app: app)
        app.buttons["gif-picker-retry"].click()
        XCTAssertTrue(app.buttons["gif-picker-retry"].waitForExistence(timeout: 10))
        search.click(); search.typeKey("a", modifierFlags: .command); search.typeText("empty")
        XCTAssertTrue(app.staticTexts["gif-picker-empty"].waitForExistence(timeout: 10))
        search.click(); search.typeKey("a", modifierFlags: .command); search.typeText("hello")
        let tile = app.buttons["gif-picker-item-caper-fixture-gif"]
        XCTAssertTrue(tile.waitForExistence(timeout: 10))
        try await Self.control(["holdSends": true])
        tile.click()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        let pending = app.groups.matching(NSPredicate(format: "identifier BEGINSWITH %@", "pending-message-")).firstMatch
        XCTAssertTrue(pending.waitForExistence(timeout: 10))
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — keep this draft")
        XCTAssertTrue(app.buttons["staged-file-remove"].exists, "A GIF sends alone and keeps staged files")
        capture("GIF pending without clearing composer", app: app)
        try await Self.control(["holdSends": false])
        XCTAssertTrue(pending.waitForNonExistence(timeout: 15))
        let delivered = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Open TEST FIXTURE")).firstMatch
        XCTAssertTrue(delivered.waitForExistence(timeout: 10))
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — keep this draft")
        XCTAssertTrue(app.buttons["staged-file-remove"].exists)
        delivered.click()
        XCTAssertTrue(app.images["media-viewer-image"].waitForExistence(timeout: 10), "The delivered GIF must decode in the real viewer")
        capture("decoded GIF viewer", app: app)
        app.buttons["media-viewer-close"].click()
        resize(app, width: 900)
        gif.click()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        // SwiftUI's macOS Picker need not expose the iOS segmented-control role.
        // Keep the query scoped to this picker and exercise its real AX control.
        let kinds = app.descendants(matching: .any).matching(identifier: "gif-picker-kind").firstMatch
        XCTAssertTrue(kinds.waitForExistence(timeout: 5))
        let stickers = kinds.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Stickers")).firstMatch
        XCTAssertTrue(stickers.waitForExistence(timeout: 5)); XCTAssertTrue(stickers.isHittable)
        stickers.click()
        let sticker = app.buttons["gif-picker-item-caper-fixture-sticker"]
        XCTAssertTrue(sticker.waitForExistence(timeout: 10))
        capture("sticker picker narrow", app: app)
        sticker.click()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        let confirmed = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Open TEST FIXTURE")).count == 2
        }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [confirmed], timeout: 15), .completed)
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — keep this draft")
        XCTAssertTrue(app.buttons["staged-file-remove"].exists)
        capture("GIF and sticker delivered narrow", app: app)
    }

    func testVoiceRosterControlsNavigationAndParticipantAudio() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch(fixture: "voice-crowd")
        defer { app.terminate() }
        let mute = app.buttons["voice-call-mute"]
        XCTAssertTrue(mute.waitForExistence(timeout: 15))
        capture("voice crowd", app: app)
        revealSidebar(app.buttons["channel-chan00000003"], in: app.scrollViews["sidebar-channels-scroll"])
        app.buttons["channel-chan00000002"].hover(); app.buttons["channel-chan00000003"].hover()
        app.buttons["channel-chan00000002"].click()
        XCTAssertTrue(app.staticTexts["# design"].waitForExistence(timeout: 10))
        mute.click()
        let muted = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Muted"), object: mute)
        XCTAssertEqual(XCTWaiter.wait(for: [muted], timeout: 5), .completed)
        XCTAssertEqual(app.buttons["microphone-toggle"].value as? String, "Muted")
        XCTAssertTrue(app.staticTexts["# design"].exists, "Mute must not navigate to the call")
        app.buttons["voice-call-deafen"].click()
        let deafened = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Deafened"), object: app.buttons["deafen-toggle"])
        XCTAssertEqual(XCTWaiter.wait(for: [deafened], timeout: 5), .completed)
        let remote = app.buttons["voice-face-fixture-remote"]
        remote.rightClick()
        XCTAssertTrue(app.staticTexts["voice-list-title"].waitForExistence(timeout: 5))
        let volume = app.sliders["TEST FIXTURE Maya volume"]
        XCTAssertTrue(volume.waitForExistence(timeout: 5))
        volume.click(); volume.typeKey(.rightArrow, modifierFlags: [])
        XCTAssertTrue(app.staticTexts["User volume · 101%"].waitForExistence(timeout: 5), "Keyboard gain changes by one percent")
        app.checkBoxes["Mute for me"].click()
        XCTAssertTrue(app.staticTexts["participant-local-muted-fixture-remote"].waitForExistence(timeout: 5))
        capture("participant volume and local mute", app: app)
        app.buttons["voice-list-close"].click()
        XCTAssertFalse(volume.exists)
        remote.click()
        XCTAssertTrue(app.staticTexts["# general"].waitForExistence(timeout: 10), "A face opens the call's chat")
        revealSidebar(app.buttons["channels-toggle"], in: app.scrollViews["sidebar-channels-scroll"])
        app.buttons["channels-toggle"].click()
        XCTAssertTrue(mute.exists)
        XCTAssertTrue(app.buttons["channel-chan00000002"].exists, "The fixture's other live room also stays visible")
        XCTAssertFalse(app.buttons["channel-chan00000003"].exists, "Quiet channels collapse")
        capture("voice channels collapsed", app: app)
        resize(app, width: 900)
        capture("voice crowd narrow", app: app)
        app.buttons["microphone-options"].click()
        XCTAssertTrue(app.sliders["Input volume"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.popUpButtons["Microphone device"].exists)
        capture("microphone options", app: app)
        app.typeKey(.escape, modifierFlags: [])
        app.buttons["output-options"].click()
        XCTAssertTrue(app.sliders["Output volume"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.popUpButtons["Output device"].exists)
        app.typeKey(.escape, modifierFlags: [])
        app.buttons["voice-call-leave"].click()
        XCTAssertTrue(mute.waitForNonExistence(timeout: 5))
        let composer = app.textViews["message-composer"]
        composer.click(); composer.typeText("TEST FIXTURE — call left, draft still usable")
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — call left, draft still usable")
    }

    func testElsewhereVoiceRemainsAboveCollapsedChannels() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch(fixture: "voice-elsewhere")
        defer { app.terminate() }
        let call = app.buttons["voice-call-channel"]
        XCTAssertTrue(call.waitForExistence(timeout: 15))
        XCTAssertTrue(call.label.contains("TEST FIXTURE Elsewhere"))
        XCTAssertLessThan(call.frame.maxY, app.buttons["channels-toggle"].frame.minY)
        app.buttons["channels-toggle"].click()
        XCTAssertTrue(app.buttons["voice-call-leave"].exists)
        XCTAssertTrue(app.staticTexts["# general"].exists)
        capture("voice elsewhere", app: app)
        resize(app, width: 900)
        capture("voice elsewhere narrow", app: app)
        app.buttons["voice-call-leave"].click()
        XCTAssertTrue(call.waitForNonExistence(timeout: 5))
    }

    private func imageFile() throws -> URL {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 240, pixelsHigh: 160,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
            bytesPerRow: 0, bitsPerPixel: 0))
        let colours = [NSColor(deviceRed: 0.9, green: 0.35, blue: 0.1, alpha: 1),
            NSColor(deviceRed: 0.1, green: 0.7, blue: 0.75, alpha: 1), NSColor(deviceRed: 0.55, green: 0.2, blue: 0.8, alpha: 1)]
        for y in 0..<160 {
            for x in 0..<240 {
                bitmap.setColor(colours[x < 80 ? 0 : y < 70 ? 1 : 2], atX: x, y: y)
            }
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("fixture-colours.png")
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: url)
        return url
    }

    func testImagePasteEditSpoilerSendAndDecodedViewer() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        try await Self.control(["uploadsEnabled": true])
        let source = try imageFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        let attach = app.buttons["attach-files-button"]
        let enabled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: attach)
        XCTAssertEqual(XCTWaiter.wait(for: [enabled], timeout: 10), .completed)
        composer.click(); composer.typeText("TEST FIXTURE — edited image")
        NSPasteboard.general.clearContents(); NSPasteboard.general.writeObjects([source as NSURL])
        composer.typeKey("v", modifierFlags: .command)
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — edited image", "File paste must not insert its path")
        let edit = app.buttons["staged-file-edit"]
        XCTAssertTrue(edit.waitForExistence(timeout: 10))
        capture("staged image", app: app)
        app.buttons["staged-file-remove"].hover(); edit.hover()
        // XCTest's element click lands on the clip view's bottom edge on
        // macOS 15. Exercise the real pencil target, not the thumbnail or an
        // internal action. This pointer flow is not an AXPress/VoiceOver gate.
        edit.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click()
        let picture = app.images["attachment-editor-image"]
        let loaded = picture.waitForExistence(timeout: 10)
        XCTAssertTrue(loaded, "The existing editor must decode the staged original")
        guard loaded else { return }
        XCTAssertFalse(composer.exists, "The covered workspace must be absent from accessibility")
        app.buttons["attachment-editor-crop"].click()
        app.buttons["Rotate left"].click()
        app.buttons["Undo"].click(); app.buttons["Redo"].click()
        app.buttons["attachment-editor-spoiler"].click()
        capture("image editor rotated spoiler", app: app)
        app.buttons["attachment-editor-done"].click()
        XCTAssertTrue(composer.waitForExistence(timeout: 10))
        let spoiler = app.buttons["Preview spoiler attachment"]
        XCTAssertTrue(spoiler.waitForExistence(timeout: 10))
        resize(app, width: 900)
        capture("staged spoiler narrow", app: app)
        try await Self.control(["holdSends": true])
        app.buttons["send-message"].click()
        let pending = app.groups.matching(NSPredicate(format: "identifier BEGINSWITH %@", "pending-message-")).firstMatch
        XCTAssertTrue(pending.waitForExistence(timeout: 10), "The local preview must remain visible while confirmation is held")
        XCTAssertTrue(pending.buttons["Reveal spoiler attachment"].exists)
        XCTAssertFalse(app.buttons["staged-file-remove"].exists)
        capture("pending image confirmation", app: app)
        try await Self.control(["holdSends": false])
        let confirmed = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in !pending.exists }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [confirmed], timeout: 15), .completed)
        let reveal = app.buttons["Reveal spoiler attachment"]
        XCTAssertTrue(reveal.waitForExistence(timeout: 20))
        XCTAssertFalse(app.buttons["staged-file-remove"].exists)
        XCTAssertEqual(composer.value as? String, "")
        capture("sent image spoiler narrow", app: app)
        reveal.click()
        let image = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "attachment-")).firstMatch
        XCTAssertTrue(image.waitForExistence(timeout: 5)); image.click()
        XCTAssertTrue(app.images["media-viewer-image"].waitForExistence(timeout: 15), "Uploaded image bytes must decode, not just show a placeholder")
        XCTAssertFalse(composer.exists)
        capture("decoded sent image viewer", app: app)
        app.buttons["media-viewer-close"].hover(); app.buttons["media-viewer-close"].click()
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        composer.click(); composer.typeText("focus restored")
        XCTAssertEqual(composer.value as? String, "focus restored")
    }

    func testFilePickerUploadFailureRetryAndComposerOwnership() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        try await Self.control(["uploadsEnabled": true,
            "failure": ["path": "/api/assets", "method": "POST", "status": 503, "error": "TEST FIXTURE: upload unavailable."]])
        let source = try imageFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let app = launch()
        defer { app.terminate() }
        let attach = app.buttons["attach-files-button"]
        XCTAssertTrue(attach.waitForExistence(timeout: 15))
        let enabled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: attach)
        XCTAssertEqual(XCTWaiter.wait(for: [enabled], timeout: 10), .completed)
        attach.click(); app.menuItems["Upload a file"].click()
        XCTAssertTrue(app.sheets.firstMatch.waitForExistence(timeout: 5))
        app.typeKey("g", modifierFlags: [.command, .shift])
        app.typeText(source.path)
        app.typeKey(.return, modifierFlags: [])
        let add = app.sheets.buttons["Add files"]
        XCTAssertTrue(add.waitForExistence(timeout: 5)); add.click()
        let retry = app.buttons["staged-file-retry"]
        XCTAssertTrue(retry.waitForExistence(timeout: 15))
        capture("upload failure", app: app)
        retry.click()
        let recovered = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in !retry.exists }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [recovered], timeout: 15), .completed)
        XCTAssertTrue(app.buttons["staged-file-remove"].exists)
        app.buttons["channel-chan00000002"].click()
        let otherComposer = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in !app.buttons["staged-file-remove"].exists }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [otherComposer], timeout: 5), .completed, "Staged files belong to their original conversation")
        app.buttons["channel-chan00000001"].click()
        XCTAssertTrue(app.buttons["staged-file-remove"].waitForExistence(timeout: 10))
        app.buttons["send-message"].click()
        XCTAssertTrue(app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "attachment-")).firstMatch.waitForExistence(timeout: 15), "Attachment-only send must be accepted")
        XCTAssertFalse(app.buttons["staged-file-remove"].exists)
        let composer = app.textViews["message-composer"]
        composer.click()
        NSPasteboard.general.clearContents(); NSPasteboard.general.writeObjects([source as NSURL])
        composer.typeKey("v", modifierFlags: .command)
        let remove = app.buttons["staged-file-remove"]
        XCTAssertTrue(remove.waitForExistence(timeout: 10)); remove.click()
        let removed = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in !remove.exists }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [removed], timeout: 5), .completed)
        XCTAssertFalse(app.buttons["send-message"].isEnabled, "Removing the last file leaves no sendable draft")
    }

    func testIncomingRequestRetryAcceptAndBlockedConversation() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        try await Self.control(["messageRequest": ["text": "TEST FIXTURE — A request, not an accepted DM."]])
        let app = launch()
        defer { app.terminate() }
        let requests = app.buttons["message-requests"]
        XCTAssertTrue(requests.waitForExistence(timeout: 15))
        XCTAssertEqual(requests.value as? String, "1 request, Collapsed")
        requests.click()
        let request = app.buttons["message-request-dm0000000003"]
        XCTAssertTrue(request.waitForExistence(timeout: 5)); request.click()
        let accept = app.buttons["message-request-accept"]
        XCTAssertTrue(accept.waitForExistence(timeout: 10))
        XCTAssertTrue(app.textViews["message-dm0000000003m01"].exists)
        let composer = app.textViews["message-composer"]
        XCTAssertFalse(composer.exists, "Reading a request must not accept it or offer a composer")
        XCTAssertFalse(app.buttons["dm-dm0000000003"].exists)
        capture("incoming request", app: app)
        app.buttons["message-request-block"].hover(); app.buttons["message-request-decline"].hover(); accept.hover()
        try await Self.control(["failure": ["path": "/api/dms/dm0000000003/accept", "method": "POST", "status": 503,
            "error": "TEST FIXTURE: accepting unavailable."]])
        accept.click()
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: accepting unavailable."].waitForExistence(timeout: 5))
        XCTAssertFalse(composer.exists)
        capture("request retry", app: app)
        accept.click()
        XCTAssertTrue(composer.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["dm-dm0000000003"].exists)
        XCTAssertFalse(requests.exists)
        app.buttons["dm-options-dm0000000003"].click(); app.menuItems["Block @jordan…"].click()
        let confirmation = app.sheets.containing(.staticText, identifier: "Block Jordan?").firstMatch
        XCTAssertTrue(confirmation.buttons["Cancel"].waitForExistence(timeout: 5)); confirmation.buttons["Cancel"].click()
        XCTAssertTrue(composer.exists)
        app.buttons["dm-options-dm0000000003"].click(); app.menuItems["Block @jordan…"].click()
        confirmation.buttons["Block"].click()
        let unblock = app.buttons["dm-blocked-unblock"]
        XCTAssertTrue(unblock.waitForExistence(timeout: 10))
        XCTAssertFalse(composer.exists)
        let show = app.buttons["blocked-run-toggle"]
        XCTAssertEqual(show.label, "Show 1 blocked message")
        XCTAssertFalse(app.textViews["message-dm0000000003m01"].exists, "Collapsed blocked text is absent from accessibility")
        capture("blocked DM", app: app)
        resize(app, width: 900)
        capture("blocked DM narrow", app: app)
        show.click()
        XCTAssertTrue(app.textViews["message-dm0000000003m01"].waitForExistence(timeout: 5))
        XCTAssertEqual(show.label, "Hide 1 blocked message")
        capture("revealed blocked DM", app: app)
        show.click()
        XCTAssertFalse(app.textViews["message-dm0000000003m01"].exists)
        try await Self.control(["failure": ["path": "/api/blocks/stranger0001", "method": "DELETE", "status": 503,
            "error": "TEST FIXTURE: unblocking unavailable."]])
        unblock.click()
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: unblocking unavailable."].waitForExistence(timeout: 5))
        XCTAssertFalse(composer.exists)
        unblock.click()
        XCTAssertTrue(composer.waitForExistence(timeout: 10))
        XCTAssertTrue(app.textViews["message-dm0000000003m01"].waitForExistence(timeout: 5))
        composer.click(); composer.typeText("TEST FIXTURE — Accepted reply"); app.buttons["send-message"].click()
        XCTAssertTrue(app.textViews.matching(NSPredicate(format: "value == %@", "TEST FIXTURE — Accepted reply")).firstMatch.waitForExistence(timeout: 10))
    }

    func testDecliningOrBlockingRequestReturnsToNavigation() async throws {
        continueAfterFailure = false
        for block in [false, true] {
            try await Self.resetFixture()
            try await Self.control(["messageRequest": ["text": "TEST FIXTURE — Pending request"]])
            let app = launch()
            defer { app.terminate() }
            let requests = app.buttons["message-requests"]
            XCTAssertTrue(requests.waitForExistence(timeout: 15)); requests.click()
            app.buttons["message-request-dm0000000003"].click()
            let decline = app.buttons["message-request-decline"]
            XCTAssertTrue(decline.waitForExistence(timeout: 10))
            if block {
                app.buttons["message-request-block"].click()
                let confirmation = app.sheets.containing(.staticText, identifier: "Block Jordan?").firstMatch
                XCTAssertTrue(confirmation.buttons["Block"].waitForExistence(timeout: 5)); confirmation.buttons["Block"].click()
            } else { decline.click() }
            XCTAssertTrue(requests.waitForNonExistence(timeout: 10))
            XCTAssertFalse(app.buttons["dm-dm0000000003"].exists)
            XCTAssertTrue(app.staticTexts["# general"].waitForExistence(timeout: 10))
            XCTAssertTrue(app.textViews["message-composer"].exists)
        }
    }

    func testOwnerNavigationDialogsRetryAndCreateChannel() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let space = app.buttons["selected-space-name"]
        XCTAssertTrue(space.waitForExistence(timeout: 15))
        capture("navigation", app: app)
        app.buttons["create-channel"].hover(); app.buttons["channel-options"].hover(); space.hover(); space.click()
        app.menuItems["Space settings"].click()
        let name = app.textFields["Space name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["channel-chan00000001"].exists, "The modal hides background controls from accessibility")
        name.click(); name.typeKey("a", modifierFlags: .command); name.typeText("Native studio")
        try await Self.control(["failure": ["path": "/api/spaces/space0000001", "method": "PATCH", "status": 503,
            "error": "TEST FIXTURE: rename unavailable."]])
        let save = app.windows.firstMatch.buttons["Save name"]
        save.click()
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: rename unavailable."].waitForExistence(timeout: 5))
        XCTAssertEqual(name.value as? String, "Native studio", "A rejected save preserves the edit")
        capture("space settings retry", app: app)
        save.click()
        XCTAssertTrue(app.staticTexts["TEST FIXTURE: rename unavailable."].waitForNonExistence(timeout: 5))
        app.windows.firstMatch.buttons["Close"].click()
        XCTAssertTrue(space.waitForExistence(timeout: 5))
        XCTAssertEqual(space.label, "Native studio")
        space.click(); app.menuItems["Invite people…"].click()
        XCTAssertTrue(app.staticTexts["Invite people"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.textFields["Space name"].exists, "Invite opens the invite-only surface")
        capture("invite people", app: app)
        app.windows.firstMatch.buttons["Close"].click()
        app.buttons["create-channel"].click()
        let channelName = app.textFields["project-updates"]
        XCTAssertTrue(channelName.waitForExistence(timeout: 5))
        channelName.click(); channelName.typeText("native-navigation")
        capture("create channel", app: app)
        app.windows.firstMatch.buttons["Create channel"].click()
        XCTAssertTrue(app.staticTexts["# native-navigation"].waitForExistence(timeout: 10))
        app.buttons["create-space"].click()
        XCTAssertTrue(app.buttons["join-invite-link"].waitForExistence(timeout: 5))
        app.buttons["join-invite-link"].click()
        XCTAssertTrue(app.textFields["Invite link"].waitForExistence(timeout: 5))
        capture("join invite link", app: app)
        app.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(app.buttons["create-space"].waitForExistence(timeout: 5))
    }

    func testChannelMenuMuteBrowseLeaveAndRejoin() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        try await Self.control(["incomingReaction": ["channelId": "chan00000002", "messageId": "chan00000002m01",
            "emoji": "🍅", "userId": "member000001"]])
        try await Self.control(["incomingPin": ["channelId": "chan00000002", "messageId": "chan00000002m01", "userId": "member000001"]])
        let app = launch()
        defer { app.terminate() }
        let design = app.buttons["channel-chan00000002"]
        XCTAssertTrue(design.waitForExistence(timeout: 15))
        app.buttons["channel-options-chan00000002"].click()
        app.menuItems["Mute channel"].hover()
        let preset = app.menuItems["Until I turn it back on"]
        XCTAssertTrue(preset.waitForExistence(timeout: 5)); preset.click()
        let muted = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", "Muted"), object: design)
        XCTAssertEqual(XCTWaiter.wait(for: [muted], timeout: 5), .completed)
        app.buttons["channel-options-chan00000002"].click()
        XCTAssertTrue(app.menuItems["Unmute channel"].waitForExistence(timeout: 5))
        app.menuItems["Leave channel"].click()
        XCTAssertTrue(app.staticTexts["Leave #design?"].waitForExistence(timeout: 5))
        app.windows.firstMatch.buttons["Cancel"].click()
        XCTAssertTrue(design.waitForExistence(timeout: 5))
        app.buttons["channel-options-chan00000002"].click(); app.menuItems["Leave channel"].click()
        app.windows.firstMatch.buttons["Leave channel"].click()
        XCTAssertTrue(design.waitForNonExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["# general"].exists, "Leaving a different channel must not navigate")
        app.buttons["selected-space-name"].click(); app.menuItems["Browse channels"].click()
        let search = app.searchFields["browse-channels-search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.click(); search.typeText(" #des ")
        let result = app.buttons["browse-channel-chan00000002"]
        XCTAssertTrue(result.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["browse-channel-chan00000001"].exists)
        capture("browse unjoined channel", app: app)
        result.click()
        XCTAssertTrue(app.buttons["join-channel"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.textViews["message-composer"].exists, "A preview cannot send")
        XCTAssertFalse(app.buttons["composer-voice-record"].exists, "A preview cannot start dictation or a voice note")
        capture("channel preview", app: app)
        let previewMessage = app.textViews["message-chan00000002m01"]
        XCTAssertTrue(previewMessage.waitForExistence(timeout: 5))
        let previewChip = app.buttons["reaction-chan00000002m01-🍅"]
        let previewTooltip = app.descendants(matching: .any)["reaction-tooltip"]
        XCTAssertTrue(previewChip.waitForExistence(timeout: 5)); XCTAssertFalse(previewChip.isEnabled)
        previewChip.hover(); try await Task.sleep(for: .milliseconds(500))
        XCTAssertTrue(previewTooltip.waitForExistence(timeout: 5), "Disabled preview chips still expose the read-only hover summary")
        let previewNames = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label BEGINSWITH %@", "Maya reacted with "), object: previewTooltip)
        XCTAssertEqual(XCTWaiter.wait(for: [previewNames], timeout: 10), .completed)
        capture("disabled reaction chip hover in read-only preview", app: app)
        app.buttons["join-channel"].hover(); XCTAssertTrue(previewTooltip.waitForNonExistence(timeout: 5))
        XCTAssertEqual(previewChip.label, "🍅 1")
        previewMessage.rightClick()
        let details = messageMenuItem("View reactions", app: app, body: previewMessage)
        XCTAssertTrue(details.waitForExistence(timeout: 5)); XCTAssertTrue(details.isEnabled)
        XCTAssertFalse(messageMenuItem("Add reaction…", app: app, body: previewMessage).isEnabled)
        details.click()
        let people = app.descendants(matching: .any)["reactor-row-member000001"]
        XCTAssertTrue(people.waitForExistence(timeout: 10), "Reading names does not require joining or a chat session")
        capture("who reacted in a read-only channel preview", app: app)
        app.buttons["Done"].click()
        XCTAssertTrue(app.buttons["join-channel"].waitForExistence(timeout: 5))
        let pinHistoryPath = "/api/chat/channels/chan00000002/messages"
        let beforePins = try await Self.fixtureGET(pinHistoryPath)
        for (id, title) in [("chan00000002m01", "Unpin message"), ("chan00000002m02", "Pin message")] {
            let body = app.textViews["message-\(id)"]
            app.descendants(matching: .any)["message-row-\(id)"].hover()
            app.buttons["add-reaction-\(id)"].hover(); app.buttons["message-options-\(id)"].hover()
            app.buttons["message-options-\(id)"].click()
            let option = messageMenuItem(title, app: app, body: body)
            XCTAssertTrue(option.waitForExistence(timeout: 5)); XCTAssertFalse(option.isEnabled, "Preview options cannot mutate pins")
            app.typeKey(.escape, modifierFlags: [])
            body.rightClick()
            let context = messageMenuItem(title, app: app, body: body)
            XCTAssertTrue(context.waitForExistence(timeout: 5)); XCTAssertFalse(context.isEnabled, "Right-click cannot bypass preview restrictions")
            capture("preview disabled \(title) context menu", app: app)
            app.typeKey(.escape, modifierFlags: [])
        }
        app.buttons["toggle-members"].hover(); app.buttons["channel-pins"].hover(); app.buttons["channel-pins"].click()
        let pins = app.descendants(matching: .any)["pinned-messages"]
        XCTAssertTrue(pins.descendants(matching: .any)["pinned-message-chan00000002m01"].waitForExistence(timeout: 5),
            "Reading the pin list must not require joining or a chat session")
        XCTAssertTrue(app.buttons["join-channel"].exists, "Pins is a popover, not the modal accessibility overlay")
        XCTAssertFalse(app.textViews["message-composer"].exists)
        capture("pins readable in unjoined channel preview", app: app)
        app.buttons["close-pins"].click()
        XCTAssertTrue(pins.waitForNonExistence(timeout: 5))
        let afterPins = try await Self.fixtureGET(pinHistoryPath)
        XCTAssertEqual(afterPins["cursor"] as? String, beforePins["cursor"] as? String, "Reading preview pins must not mutate them")
        let persistedPins = try XCTUnwrap(afterPins["pinnedMessages"] as? [[String: Any]])
        XCTAssertEqual(persistedPins.compactMap { $0["id"] as? String }, ["chan00000002m01"])
        let pinner = try XCTUnwrap(persistedPins.first?["pin"] as? [String: Any])
        XCTAssertEqual((pinner["author"] as? [String: Any])?["id"] as? String, "member000001")
        app.buttons["join-channel"].click()
        XCTAssertTrue(app.textViews["message-composer"].waitForExistence(timeout: 10))
        XCTAssertTrue(design.exists)
        XCTAssertTrue(app.staticTexts["# design"].exists)
        app.buttons["channels-toggle"].click()
        XCTAssertTrue(design.exists, "The fixture's live #design room survives collapse after rejoining")
        XCTAssertFalse(app.buttons["channel-chan00000001"].exists, "Quiet joined channels collapse")
        XCTAssertTrue(result.exists, "Browse remains available while the joined section is collapsed")
        try await Self.control(["media": ["channelId": "chan00000002", "participants": []]])
        XCTAssertTrue(design.waitForNonExistence(timeout: 10), "The joined row disappears when its last voice participant leaves")
        XCTAssertTrue(result.exists, "The browse result is independent of live-room filtering")
        app.buttons["channels-toggle"].click()
        XCTAssertTrue(design.exists)
    }

    func testAccountDialogsProtectProfileAndRestoreComposerFocus() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        composer.click(); composer.typeText("Keep this draft")
        app.buttons["account-profile"].click()
        let name = app.textFields["Display name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.click(); name.typeKey("a", modifierFlags: .command); name.typeText("Unsaved native profile")
        app.typeKey(.escape, modifierFlags: [])
        let confirmation = app.sheets.containing(.staticText, identifier: "Discard profile changes?").firstMatch
        XCTAssertTrue(confirmation.buttons["Keep editing"].waitForExistence(timeout: 5))
        confirmation.buttons["Keep editing"].click()
        XCTAssertEqual(name.value as? String, "Unsaved native profile")
        app.buttons["Close profile"].click(); confirmation.buttons["Discard"].click()
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        app.typeText(" after closing")
        XCTAssertEqual(composer.value as? String, "Keep this draft after closing", "Closing restores the previous editor focus")
        app.buttons["account-settings-menu"].click(); app.menuItems["Privacy and blocked accounts"].click()
        XCTAssertTrue(app.staticTexts["Privacy"].waitForExistence(timeout: 5))
        capture("privacy", app: app)
        app.buttons["Close privacy settings"].click()
        app.buttons["account-settings-menu"].click(); app.menuItems["Notifications"].click()
        XCTAssertTrue(app.staticTexts["Notifications"].waitForExistence(timeout: 5))
        capture("notifications", app: app)
        app.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.value as? String, "Keep this draft after closing")

        // The sidebar inbox is distinct from the account's notification settings.
        // Exercise both native dismissal paths without adding another launch flow.
        let entry = app.buttons["activity-inbox"]
        let dialog = app.descendants(matching: .any)["activity-inbox-dialog"].firstMatch
        let done = app.buttons["activity-done"]
        let window = app.windows["Caper — AppKit development (not feature complete)"]
        var draft = "Keep this draft after closing"
        for width in [CGFloat(1280), CGFloat(900)] {
            resize(app, width: width)
            XCTAssertTrue(entry.waitForExistence(timeout: 5)); XCTAssertEqual(entry.label, "Notifications")
            XCTAssertEqual(app.buttons.matching(identifier: "activity-inbox").count, 1)
            composer.click()
            entry.hover(); app.buttons["selected-space-name"].hover(); entry.click()
            XCTAssertTrue(dialog.waitForExistence(timeout: 5))
            XCTAssertTrue(app.buttons["activity-open-inbox0000002"].waitForExistence(timeout: 10))
            XCTAssertEqual(done.label, "Done"); XCTAssertTrue(done.isHittable)
            XCTAssertFalse(entry.exists); XCTAssertFalse(composer.exists)
            XCTAssertFalse(app.scrollViews["message-timeline"].exists)
            XCTAssertFalse(app.buttons["channel-chan00000001"].exists)
            XCTAssertFalse(app.buttons["account-settings-menu"].exists, "All background navigation must leave the modal AX tree")
            XCTAssertTrue(window.frame.contains(dialog.frame)); XCTAssertTrue(window.frame.contains(done.frame))
            XCTAssertGreaterThan(dialog.frame.width, 300); XCTAssertLessThanOrEqual(dialog.frame.width, 681)
            XCTAssertGreaterThan(dialog.frame.height, 200); XCTAssertLessThanOrEqual(dialog.frame.height, 681)
            for control in [done, app.buttons["activity-filter-all"], app.buttons["activity-open-inbox0000002"]] {
                XCTAssertTrue(control.isHittable); XCTAssertTrue(dialog.frame.contains(control.frame))
            }
            capture(width == 1280 ? "activity inbox bounded modal and AX isolation wide" : "activity inbox bounded modal and AX isolation narrow", app: app)
            if width == 1280 { done.hover(); app.buttons["activity-filter-invitations"].hover(); done.click() }
            else { app.typeKey(.escape, modifierFlags: []) }
            XCTAssertTrue(dialog.waitForNonExistence(timeout: 5)); XCTAssertTrue(composer.waitForExistence(timeout: 5))
            XCTAssertEqual(composer.value as? String, draft)
            XCTAssertEqual(entry.value as? String, "2 unread", "Opening and closing must not mark notifications read")
            let suffix = width == 1280 ? " after inbox Done" : " after inbox Escape"
            app.typeText(suffix); draft += suffix
            XCTAssertEqual(composer.value as? String, draft, "The native host must restore focus, not just retain text")
        }
        let unread = try await Self.fixtureGET("/api/notifications/inbox")
        XCTAssertEqual(unread["unreadCount"] as? Int, 2)
    }

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

    func testDraftFormattingCopyRepairUndoMenusAndSend() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        composer.click(); composer.typeText("format me")
        composer.typeKey("a", modifierFlags: .command)
        composer.typeKey("b", modifierFlags: .command)
        XCTAssertEqual(composer.value as? String, "**format me**")
        composer.typeKey("b", modifierFlags: .command)
        XCTAssertEqual(composer.value as? String, "format me", "Cmd-B toggles the selected style off")
        composer.typeKey("z", modifierFlags: .command)
        XCTAssertEqual(composer.value as? String, "**format me**", "A formatting toggle is one native undoable edit")
        composer.typeKey(.rightArrow, modifierFlags: [])
        composer.typeText(" and _italic_")
        // The web ProseMirror composer keeps the style across typed whitespace;
        // the shared Rust bridge must do the same through the focused native editor.
        let styled = "**format me** **and** **_italic_**"
        XCTAssertEqual(composer.value as? String, styled, "Continuous typing reopens bold after spaces and nests the typed italic span")
        composer.typeKey("a", modifierFlags: .command); composer.typeKey("c", modifierFlags: .command)
        XCTAssertEqual(NSPasteboard.general.string(forType: .string), "format me and italic", "Copy excludes hidden delimiters")
        composer.typeKey("x", modifierFlags: .command)
        XCTAssertEqual(composer.value as? String, "", "Cut repairs orphaned delimiters, not just visible characters")
        composer.typeKey("z", modifierFlags: .command)
        XCTAssertEqual(composer.value as? String, styled)
        composer.typeKey("a", modifierFlags: .command); composer.typeText("menu style")
        composer.typeKey("a", modifierFlags: .command); composer.rightClick()
        let italic = composer.menuItems["I — Italic"]
        XCTAssertTrue(italic.waitForExistence(timeout: 5))
        composer.menuItems["B — Bold"].hover(); italic.hover(); italic.click()
        XCTAssertEqual(composer.value as? String, "_menu style_")
        app.menuBars.menuBarItems["Edit"].click(); app.menuBars.menuItems["B — Bold"].click()
        XCTAssertEqual(composer.value as? String, "_**menu style**_", "The Edit menu formats the same native selection")
        composer.typeKey(.rightArrow, modifierFlags: [])
        composer.typeText(" tail")
        XCTAssertEqual(composer.value as? String, "**_menu style_** **_tail_**", "Both styles continue across whitespace, as in web")
        capture("draft formatting menus", app: app)
        app.buttons["send-message"].click()
        XCTAssertTrue(app.textViews.matching(NSPredicate(format: "value == %@", "menu style tail")).firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(composer.value as? String, "")
        composer.click(); composer.typeText("**bold** ")
        XCTAssertEqual(composer.value as? String, "**bold** ")
        composer.typeKey(.leftArrow, modifierFlags: [])
        composer.typeKey(.rightArrow, modifierFlags: [])
        composer.typeText("a")
        XCTAssertEqual(composer.value as? String, "**bold** a", "Moving the caret away and back ends the pending style; typing alone must not")
        composer.typeKey("a", modifierFlags: .command)
        composer.typeText("**bold** _italic_ @maya @feedback https://example.com/a_b 🙂")
        XCTAssertEqual(composer.value as? String, "**bold** _italic_ @maya @feedback https://example.com/a_b 🙂")
        capture("draft style mentions and link", app: app)
        let window = app.windows["Caper — AppKit development (not feature complete)"]
        resize(app, width: 900)
        XCTAssertTrue(window.frame.contains(composer.frame))
        capture("draft formatting narrow", app: app)
    }

    func testComposerKeyByKeyMarkdownClosuresKeepFollowingTextLiteral() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        addTeardownBlock { try await Self.resetFixture() }
        let historyPath = "/api/chat/channels/chan00000001/messages"
        let before = try await Self.fixtureGET(historyPath)
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        XCTAssertEqual(composer.value as? String, "")
        let source = "**Release check** for _the voice beta_: see **both _builds_** and **Done** when"
        var prefix = ""
        composer.click()
        // Send every character to the actual first responder, including each
        // closing marker and the next space. No paste, whole-string insertion,
        // refocusing, selection repair, or Return can mask the typing state.
        // This differs deliberately from continuing an already formatted run.
        for (index, character) in source.enumerated() {
            app.typeText(String(character))
            prefix.append(character)
            XCTAssertEqual(composer.value as? String, prefix,
                "Source after key \(index + 1) (\(String(character).debugDescription)) must equal the exact typed prefix")
        }
        let finalSource = try XCTUnwrap(composer.value as? String)
        XCTAssertEqual(finalSource, source)
        XCTAssertTrue(finalSource.hasSuffix("**Done** when"), "The tail after the explicitly closed bold span stays literal")
        XCTAssertFalse(finalSource.hasSuffix("**when**"))
        XCTAssertTrue(app.buttons["send-message"].isEnabled)
        capture("key-by-key markdown closures unsent 1280", app: app)
        resize(app, width: 900)
        XCTAssertTrue(composer.isHittable)
        XCTAssertEqual(composer.value as? String, source)
        capture("key-by-key markdown exact source and plain tail unsent 900", app: app)
        let after = try await Self.fixtureGET(historyPath)
        XCTAssertEqual(after["cursor"] as? String, before["cursor"] as? String, "Typing never submits the draft")
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

    func testSidebarSectionsIndependentScrollDividerGeometryAndRelaunchPreserveState() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        addTeardownBlock { try await Self.resetFixture() }
        // Clear only the development host's real shipping-key preference.
        // An argument-domain value would conceal both missing writes and a
        // broken relaunch read. Never touch the shipping app's defaults domain.
        let preferences = try XCTUnwrap(UserDefaults(suiteName: "chat.caper.appkit-development"))
        let shareKey = "caper.channelSidebarChannelsShare"
        preferences.removeObject(forKey: shareKey)
        XCTAssertTrue(preferences.synchronize())
        XCTAssertNil(preferences.object(forKey: shareKey))
        defer { preferences.removeObject(forKey: shareKey); _ = preferences.synchronize() }

        // The existing fixture permits channel creation and exactly two
        // accepted non-self DM peers. The 96pt direct pane below forces those
        // real rows to overflow; there is no invented many-DMs control.
        var extraChannels: [String] = []
        for suffix in ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet", "kilo", "lima"] {
            let channel = try await Self.fixturePOST("/api/spaces/space0000001/channels",
                body: ["name": "sidebar-fixture-\(suffix)", "private": false], status: 201)
            XCTAssertEqual(channel["joined"] as? Bool, true)
            extraChannels.append(try XCTUnwrap(channel["id"] as? String))
        }
        for (username, id) in [("maya", "dm0000000004"), ("fixture_alex", "dm0000000001")] {
            let conversation = try await Self.fixturePOST("/api/dms", body: ["username": username])
            XCTAssertEqual(conversation["id"] as? String, id)
            XCTAssertEqual(conversation["status"] as? String, "accepted")
        }
        try await Self.control(["messageRequest": ["text": "TEST FIXTURE — sidebar request stays pending"],
            "channelReads": ["chan00000002": "0"]])
        let detail = try await Self.fixtureGET("/api/spaces/space0000001")
        XCTAssertEqual((detail["channels"] as? [[String: Any]])?.count, 15)
        let historyPath = "/api/chat/channels/chan00000001/messages"
        let historyBefore = try await Self.fixtureGET(historyPath)
        let app = launch(fixture: "voice-crowd")
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        let channels = app.scrollViews["sidebar-channels-scroll"]
        let directs = app.scrollViews["sidebar-direct-scroll"]
        let divider = app.sliders["sidebar-sections-resize"]
        let space = app.buttons["selected-space-name"]
        let account = app.buttons["account-profile"]
        let timeline = app.scrollViews["message-timeline"]
        let heading = channels.buttons["channels-toggle"]
        let general = channels.buttons["channel-chan00000001"]
        let design = channels.buttons["channel-chan00000002"]
        let mute = channels.buttons["voice-call-mute"]
        let activity = directs.buttons["activity-inbox"]
        let requests = directs.buttons["message-requests"]
        let newMessage = directs.buttons["new-message"]
        let search = channels.searchFields["browse-channels-search"]
        let lastID = try XCTUnwrap(extraChannels.last)
        let browseResult = channels.buttons["browse-channel-\(lastID)"]
        let draft = "TEST FIXTURE — independent sidebar sections keep this unsent draft"
        XCTAssertTrue(channels.waitForExistence(timeout: 5)); XCTAssertTrue(directs.exists)
        XCTAssertTrue(divider.exists); XCTAssertEqual(divider.label, "Channels and direct messages divider")
        XCTAssertEqual(divider.value as? String, "50 percent channels")
        XCTAssertEqual(channels.frame.height, directs.frame.height, accuracy: 1)
        XCTAssertTrue(heading.exists); XCTAssertTrue(mute.waitForExistence(timeout: 10))
        XCTAssertTrue(directs.staticTexts["Direct messages"].exists)
        XCTAssertTrue(directs.buttons["dm-self"].exists)
        XCTAssertTrue(directs.buttons["dm-dm0000000004"].waitForExistence(timeout: 10))
        XCTAssertTrue(directs.buttons["dm-dm0000000001"].waitForExistence(timeout: 5))
        XCTAssertFalse(channels.buttons["activity-inbox"].exists)
        XCTAssertFalse(directs.buttons["channels-toggle"].exists)
        composer.click(); composer.typeText(draft)
        revealSidebar(mute, in: channels); mute.click()
        let muted = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Muted"), object: mute)
        XCTAssertEqual(XCTWaiter.wait(for: [muted], timeout: 5), .completed)
        revealSidebar(requests, in: directs); requests.click()
        XCTAssertTrue(directs.buttons["message-request-dm0000000003"].waitForExistence(timeout: 5))
        space.click(); app.menuItems["Browse channels"].click()
        revealSidebar(search, in: channels); search.click(); search.typeText("sidebar-fixture-lima")
        XCTAssertTrue(browseResult.waitForExistence(timeout: 5))
        let unread = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", "Unread activity"), object: design)
        XCTAssertEqual(XCTWaiter.wait(for: [unread], timeout: 10), .completed)

        func assertState() {
            XCTAssertEqual(composer.value as? String, draft); XCTAssertTrue(composer.isHittable)
            XCTAssertTrue(app.staticTexts["# general"].exists)
            XCTAssertTrue((general.value as? String ?? "").contains("Selected"))
            XCTAssertTrue((design.value as? String ?? "").contains("Unread activity"))
            XCTAssertEqual(mute.value as? String, "Muted")
            XCTAssertEqual(app.buttons["microphone-toggle"].value as? String, "Muted")
            XCTAssertEqual(app.buttons["deafen-toggle"].value as? String, "On")
            XCTAssertEqual(activity.value as? String, "2 unread")
            XCTAssertEqual(requests.value as? String, "1 request, Expanded")
            XCTAssertEqual(search.value as? String, "sidebar-fixture-lima")
            XCTAssertTrue(browseResult.exists)
            XCTAssertTrue(space.isHittable); XCTAssertTrue(account.isHittable)
        }
        func assertSavedShare(_ expected: Double, accuracy: Double = 0.0001) {
            let persisted = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
                _ = preferences.synchronize()
                return preferences.object(forKey: shareKey) != nil && abs(preferences.double(forKey: shareKey) - expected) < accuracy
            }, object: nil)
            XCTAssertEqual(XCTWaiter.wait(for: [persisted], timeout: 5), .completed,
                "The development host must persist its real sidebar share, not merely change its AX value")
        }
        for width in [CGFloat(1280), CGFloat(900)] {
            resize(app, width: width)
            let available = channels.frame.height + directs.frame.height
            let lower = min(CGFloat(96), available / 3)
            let headerFrame = space.frame, accountFrame = account.frame, timelineFrame = timeline.frame
            let channelsTop = channels.frame.minY, directsBottom = directs.frame.maxY
            func assertGeometry(_ height: CGFloat) {
                XCTAssertEqual(channels.frame.height, height, accuracy: 2)
                XCTAssertEqual(directs.frame.height, available - height, accuracy: 2)
                XCTAssertEqual(divider.frame.height, 12, accuracy: 1)
                XCTAssertEqual(divider.frame.minY, channels.frame.maxY, accuracy: 1)
                XCTAssertEqual(directs.frame.minY, divider.frame.maxY, accuracy: 1)
                XCTAssertEqual(channels.frame.minY, channelsTop, accuracy: 1)
                XCTAssertEqual(directs.frame.maxY, directsBottom, accuracy: 1)
                XCTAssertGreaterThanOrEqual(channels.frame.minY, space.frame.maxY)
                XCTAssertLessThanOrEqual(directs.frame.maxY, account.frame.minY)
                XCTAssertEqual(space.frame, headerFrame); XCTAssertEqual(account.frame, accountFrame)
                XCTAssertEqual(timeline.frame, timelineFrame, "Only the two sidebar sections may change geometry")
                XCTAssertEqual(app.sliders["channel-sidebar-resize"].value as? String, "280 pixels")
            }
            divider.doubleClick()
            XCTAssertEqual(divider.value as? String, "50 percent channels")
            assertGeometry(available / 2)
            channels.scroll(byDeltaX: 0, deltaY: 10000); directs.scroll(byDeltaX: 0, deltaY: 10000)
            heading.hover(); channels.buttons["channel-options"].hover(); activity.hover(); divider.hover()
            assertState()
            capture("sidebar sections default divider and live fixture \(Int(width))", app: app)

            // Both documents genuinely overflow at End. Wheel input in one
            // scrollport must leave a concrete row in the other unmoved.
            divider.click(); divider.typeKey(.end, modifierFlags: [])
            assertGeometry(available - lower)
            channels.scroll(byDeltaX: 0, deltaY: 10000); directs.scroll(byDeltaX: 0, deltaY: 10000)
            let generalY = general.frame.minY, activityFrame = activity.frame
            XCTAssertFalse(channels.frame.contains(browseResult.frame), "The fixture must really overflow channels")
            channels.scroll(byDeltaX: 0, deltaY: -10000)
            XCTAssertTrue(browseResult.isHittable); XCTAssertTrue(channels.frame.contains(browseResult.frame))
            XCTAssertLessThan(general.frame.minY, generalY - 20)
            XCTAssertEqual(activity.frame, activityFrame, "Scrolling channels must not scroll direct messages")
            browseResult.hover(); channels.buttons["browse-options-\(lastID)"].hover(); browseResult.hover()
            assertGeometry(available - lower); assertState()
            capture("sidebar channels independently scrolled to Browse result \(Int(width))", app: app)
            let resultFrame = browseResult.frame
            XCTAssertFalse(directs.frame.contains(newMessage.frame), "The supported DM rows must really overflow the minimum pane")
            directs.scroll(byDeltaX: 0, deltaY: -10000)
            XCTAssertTrue(newMessage.isHittable); XCTAssertTrue(directs.frame.contains(newMessage.frame))
            XCTAssertLessThan(activity.frame.minY, activityFrame.minY - 20)
            XCTAssertEqual(browseResult.frame, resultFrame, "Scrolling direct messages must not scroll channels")
            // These are real pointer movements over a DM, its separate menu,
            // New message and the fixed account bar, not a fabricated hover.
            revealSidebar(directs.buttons["dm-dm0000000001"], in: directs)
            directs.buttons["dm-dm0000000001"].hover(); directs.buttons["dm-options-dm0000000001"].hover()
            revealSidebar(newMessage, in: directs); newMessage.hover(); app.buttons["account-settings-menu"].hover()
            let directBottomFrame = newMessage.frame
            let refreshedPeer = "fixture-sidebar-\(Int(width))"
            try await Self.control(["media": ["channelId": "chan00000002", "participants": [
                ["id": "fixture-voice-maya", "name": "TEST FIXTURE Maya", "muted": false, "deafened": false],
                ["id": refreshedPeer, "name": "TEST FIXTURE — sidebar refresh \(Int(width))", "muted": true, "deafened": false],
            ]]])
            XCTAssertTrue(channels.buttons["room-face-\(refreshedPeer)"].waitForExistence(timeout: 10))
            XCTAssertEqual(browseResult.frame, resultFrame, "A live roster refresh keeps the channel scroll origin")
            XCTAssertEqual(newMessage.frame, directBottomFrame, "A live roster refresh cannot reset the independent DM scroll origin")
            assertGeometry(available - lower); assertState()
            capture("sidebar direct messages independently scrolled with live fixture refresh and fixed account \(Int(width))", app: app)

            divider.doubleClick(); assertGeometry(available / 2)
            let start = divider.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            let delta: CGFloat = width == 1280 ? 67 : -67
            start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: 0, dy: delta)))
            assertGeometry(available / 2 + delta)
            let draggedHeight = channels.frame.height
            assertSavedShare(Double(draggedHeight / available), accuracy: 0.005)
            divider.click(); divider.typeKey(.upArrow, modifierFlags: [])
            assertGeometry(draggedHeight - 10)
            divider.typeKey(.downArrow, modifierFlags: [])
            assertGeometry(draggedHeight)
            assertState()
            capture("sidebar pointer drag and ten-point keyboard geometry \(Int(width))", app: app)
            divider.typeKey(.home, modifierFlags: [])
            assertGeometry(lower)
            divider.typeKey(.upArrow, modifierFlags: []); assertGeometry(lower)
            revealSidebar(heading, in: channels)
            heading.hover(); channels.buttons["create-channel"].hover(); channels.buttons["channel-options"].hover()
            assertState()
            capture("sidebar minimum channels with reachable neighboring actions \(Int(width))", app: app)
            divider.click(); divider.typeKey(.end, modifierFlags: [])
            assertGeometry(available - lower)
            divider.typeKey(.downArrow, modifierFlags: []); assertGeometry(available - lower)
            divider.doubleClick(); assertGeometry(available / 2)
            XCTAssertEqual(divider.value as? String, "50 percent channels")
            revealSidebar(heading, in: channels); heading.click()
            XCTAssertEqual(heading.value as? String, "Collapsed")
            XCTAssertTrue(mute.exists); XCTAssertTrue(design.exists, "Both live rooms survive collapsing quiet channels")
            XCTAssertFalse(channels.buttons["channel-\(lastID)"].exists)
            assertState(); assertGeometry(available / 2)
            capture("sidebar reset and collapsed channels retain live call Browse requests and draft \(Int(width))", app: app)
            heading.click()
            XCTAssertEqual(heading.value as? String, "Expanded")
        }

        // Finish with a non-default pointer write. Neither launch overrides
        // this key; no fixture/default reset occurs between the two processes.
        let start = divider.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 47)))
        let savedShare = Double(channels.frame.height / (channels.frame.height + directs.frame.height))
        let savedValue = try XCTUnwrap(divider.value as? String)
        XCTAssertGreaterThan(savedShare, 0.5); XCTAssertNotEqual(savedValue, "50 percent channels")
        let other = channels.buttons["channel-\(try XCTUnwrap(extraChannels.first))"]
        revealSidebar(other, in: channels); other.click()
        XCTAssertTrue(app.staticTexts["# sidebar-fixture-alpha"].waitForExistence(timeout: 10))
        revealSidebar(general, in: channels); general.click()
        let restoredDraft = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", draft), object: composer)
        XCTAssertEqual(XCTWaiter.wait(for: [restoredDraft], timeout: 10), .completed)
        XCTAssertEqual(divider.value as? String, savedValue)
        assertState()
        let historyAfter = try await Self.fixtureGET(historyPath)
        XCTAssertEqual(historyAfter["cursor"] as? String, historyBefore["cursor"] as? String)
        let conversations = try await Self.fixtureGET("/api/dms")
        XCTAssertEqual((conversations["conversations"] as? [[String: Any]])?.first { $0["id"] as? String == "dm0000000003" }?["status"] as? String,
            "incoming", "Scrolling, hovering and resizing cannot accept the pending request")
        app.terminate()
        let relaunched = launch(fixture: "voice-crowd", preservePanelWidths: true)
        defer { relaunched.terminate() }
        let restored = relaunched.sliders["sidebar-sections-resize"]
        XCTAssertTrue(restored.waitForExistence(timeout: 15))
        for width in [CGFloat(900), CGFloat(1280)] {
            resize(relaunched, width: width)
            let restoredChannels = relaunched.scrollViews["sidebar-channels-scroll"]
            let restoredDirects = relaunched.scrollViews["sidebar-direct-scroll"]
            XCTAssertEqual(restored.value as? String, savedValue)
            XCTAssertEqual(Double(restoredChannels.frame.height / (restoredChannels.frame.height + restoredDirects.frame.height)), savedShare, accuracy: 0.005)
            XCTAssertTrue(relaunched.textViews["message-composer"].isHittable)
            restored.hover(); relaunched.buttons["account-settings-menu"].hover()
            capture("sidebar saved split restored in a new process \(Int(width))", app: relaunched)
        }
        restored.doubleClick()
        XCTAssertEqual(restored.value as? String, "50 percent channels")
        XCTAssertEqual(relaunched.scrollViews["sidebar-channels-scroll"].frame.height,
            relaunched.scrollViews["sidebar-direct-scroll"].frame.height, accuracy: 1)
        assertSavedShare(0.5)
    }

    func testSidebarWidthDragKeyboardAndRelaunchKeepChannelsAndComposerUsable() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        let window = app.windows["Caper — AppKit development (not feature complete)"]
        let handle = app.sliders["channel-sidebar-resize"]
        let timeline = app.scrollViews["message-timeline"]
        let members = app.scrollViews["member-list"]
        func sidebarWidth() -> CGFloat { timeline.frame.minX - window.frame.minX - 60 }
        XCTAssertTrue(handle.waitForExistence(timeout: 5))
        XCTAssertEqual(handle.value as? String, "280 pixels")
        XCTAssertEqual(sidebarWidth(), 280, accuracy: 2)
        composer.click(); composer.typeText("TEST FIXTURE — sidebar resize keeps this draft")
        app.buttons["create-channel"].hover(); app.buttons["channel-options"].hover(); handle.hover()
        let start = handle.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: 67, dy: 0)))
        XCTAssertEqual(sidebarWidth(), 347, accuracy: 3)
        let draggedWidth = sidebarWidth()
        handle.click(); handle.typeKey(.leftArrow, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(), draggedWidth - 10, accuracy: 1)
        handle.typeKey(.rightArrow, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(), draggedWidth, accuracy: 1)
        capture("sidebar drag and keyboard resize", app: app)
        handle.typeKey(.home, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(), 220, accuracy: 2)
        XCTAssertTrue(app.buttons["microphone-toggle"].isHittable)
        XCTAssertTrue(app.buttons["account-settings-menu"].isHittable)
        capture("minimum sidebar with account controls", app: app)
        handle.typeKey(.end, modifierFlags: [])
        XCTAssertEqual(sidebarWidth(), 440, accuracy: 2)
        resize(app, width: 900)
        XCTAssertTrue(members.waitForNonExistence(timeout: 5), "Docking depends on space left after navigation")
        XCTAssertGreaterThanOrEqual(timeline.frame.width, 320)
        handle.click(); handle.typeKey(.home, modifierFlags: [])
        XCTAssertTrue(members.waitForExistence(timeout: 5), "Narrowing navigation can dock members without changing the window")
        XCTAssertGreaterThanOrEqual(timeline.frame.width, 320)
        capture("narrow sidebar docks members at intermediate window", app: app)
        resize(app, width: 840)
        handle.click(); handle.typeKey(.end, modifierFlags: [])
        XCTAssertFalse(members.exists)
        XCTAssertGreaterThanOrEqual(timeline.frame.width, 320)
        XCTAssertTrue(composer.isHittable)
        capture("maximum sidebar in minimum window", app: app)
        handle.doubleClick()
        XCTAssertEqual(sidebarWidth(), 280, accuracy: 2)
        resize(app, width: 1280)
        let reset = handle.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        reset.press(forDuration: 0.1, thenDragTo: reset.withOffset(CGVector(dx: 47, dy: 0)))
        XCTAssertEqual(sidebarWidth(), 327, accuracy: 3)
        let saved = handle.value as? String
        app.buttons["channel-chan00000002"].click()
        XCTAssertTrue(composer.waitForExistence(timeout: 10))
        app.buttons["channel-chan00000001"].click()
        let restoredDraft = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "TEST FIXTURE — sidebar resize keeps this draft"), object: composer)
        XCTAssertEqual(XCTWaiter.wait(for: [restoredDraft], timeout: 10), .completed)
        XCTAssertEqual(handle.value as? String, saved)
        app.terminate()
        let relaunched = launch(preservePanelWidths: true)
        defer { relaunched.terminate() }
        let restored = relaunched.sliders["channel-sidebar-resize"]
        XCTAssertTrue(restored.waitForExistence(timeout: 15))
        resize(relaunched, width: 1280)
        XCTAssertEqual(restored.value as? String, saved)
        XCTAssertTrue(relaunched.textViews["message-composer"].isHittable)
        capture("sidebar preference restored after relaunch", app: relaunched)
        restored.doubleClick()
    }

    func testMemberWidthDragKeyboardOverlayAndRelaunchPreservePreferenceAndDraft() async throws {
        continueAfterFailure = false
        try await Self.resetFixture()
        let app = launch()
        defer { app.terminate() }
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 15))
        resize(app, width: 1280)
        let handle = app.sliders["members-panel-resize"]
        let list = app.scrollViews["member-list"]
        let timeline = app.scrollViews["message-timeline"]
        XCTAssertTrue(handle.waitForExistence(timeout: 5))
        XCTAssertEqual(list.frame.width, 220, accuracy: 1)
        composer.click(); composer.typeText("TEST FIXTURE — resize keeps this draft")
        capture("default member width", app: app)
        app.buttons["Search members"].hover(); app.buttons["Close member list"].hover(); handle.hover()
        let start = handle.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: -93, dy: 0)))
        XCTAssertEqual(list.frame.width, 313, accuracy: 3)
        let draggedWidth = list.frame.width
        handle.click(); handle.typeKey(.leftArrow, modifierFlags: [])
        XCTAssertEqual(list.frame.width, draggedWidth + 10, accuracy: 1)
        handle.typeKey(.rightArrow, modifierFlags: [])
        XCTAssertEqual(list.frame.width, draggedWidth, accuracy: 1)
        capture("member drag and keyboard width", app: app)
        handle.typeKey(.home, modifierFlags: [])
        XCTAssertEqual(list.frame.width, 220, accuracy: 1)
        handle.typeKey(.end, modifierFlags: [])
        XCTAssertEqual(list.frame.width, 440, accuracy: 1)
        resize(app, width: 900)
        XCTAssertTrue(list.exists, "Dock when there are 540 points after navigation, not only at 1100-point windows")
        XCTAssertEqual(timeline.frame.width, 320, accuracy: 3)
        XCTAssertLessThan(list.frame.width, 250)
        capture("member width clamped with 320-point conversation", app: app)
        resize(app, width: 840)
        XCTAssertTrue(list.waitForNonExistence(timeout: 5), "Intermediate windows do not overlay members by default")
        XCTAssertFalse(handle.exists)
        let fullConversation = timeline.frame.width
        app.buttons["toggle-members"].click()
        XCTAssertTrue(list.waitForExistence(timeout: 5))
        XCTAssertEqual(timeline.frame.width, fullConversation, accuracy: 1, "The explicitly opened list overlays instead of shrinking the conversation")
        XCTAssertEqual(app.buttons["Close member list"].frame.minY - app.buttons["toggle-members"].frame.minY, 51, accuracy: 2)
        XCTAssertGreaterThanOrEqual(list.frame.minX - timeline.frame.minX, 63)
        capture("explicit intermediate member overlay", app: app)
        app.buttons["Close member list"].click()
        resize(app, width: 1280)
        XCTAssertFalse(list.exists, "An explicit close stays closed when the window widens")
        app.buttons["toggle-members"].click()
        XCTAssertTrue(list.waitForExistence(timeout: 5))
        XCTAssertEqual(list.frame.width, 440, accuracy: 1, "The narrow clamp did not replace the saved width")
        handle.doubleClick()
        XCTAssertEqual(list.frame.width, 220, accuracy: 1)
        let reset = handle.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        reset.press(forDuration: 0.1, thenDragTo: reset.withOffset(CGVector(dx: -87, dy: 0)))
        XCTAssertEqual(list.frame.width, 307, accuracy: 3)
        let savedWidth = list.frame.width
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — resize keeps this draft")
        composer.click(); composer.typeText(" and still edits")
        XCTAssertEqual(composer.value as? String, "TEST FIXTURE — resize keeps this draft and still edits")
        app.buttons["Close member list"].click()
        app.terminate()
        let relaunched = launch(preservePanelWidths: true)
        defer { relaunched.terminate() }
        XCTAssertTrue(relaunched.textViews["message-composer"].waitForExistence(timeout: 15))
        resize(relaunched, width: 1280)
        let restored = relaunched.scrollViews["member-list"]
        XCTAssertTrue(restored.waitForExistence(timeout: 5), "Visibility is session-local, unlike the saved width")
        XCTAssertEqual(restored.frame.width, savedWidth, accuracy: 1)
        capture("member preference restored after relaunch", app: relaunched)
        relaunched.sliders["members-panel-resize"].doubleClick()
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
        resize(app, width: 390)
        let window = app.windows["Caper — AppKit development (not feature complete)"]
        XCTAssertEqual(window.frame.width, 390, accuracy: 0.5)
        for control in [app.staticTexts["Welcome to Caper"], email, app.buttons["Send code"]] {
            XCTAssertTrue(control.exists)
            XCTAssertTrue(window.frame.contains(control.frame))
            XCTAssertGreaterThanOrEqual(control.frame.minX, window.frame.minX + 24)
            XCTAssertLessThanOrEqual(control.frame.maxX, window.frame.maxX - 24)
        }
        XCTAssertEqual(email.frame.width, 342, accuracy: 1)
        XCTAssertEqual(app.buttons["Send code"].frame.width, email.frame.width, accuracy: 1)
        XCTAssertLessThanOrEqual(email.frame.maxY, app.buttons["Send code"].frame.minY)
        XCTAssertFalse(app.buttons["Send code"].isEnabled)
        email.hover(); app.buttons["Send code"].hover()
        capture("390 signed-out email form within window margins", app: app)
        email.click(); email.typeText("not-an-email")
        app.buttons["Send code"].click()
        XCTAssertTrue(app.buttons["Send code"].waitForExistence(timeout: 10))
        XCTAssertTrue(email.isEnabled)
        XCTAssertEqual(app.staticTexts["sign-in-error"].value as? String, "Enter a valid email address.")
        XCTAssertTrue(window.frame.contains(app.staticTexts["sign-in-error"].frame))
        capture("sign-in error", app: app)
        email.click(); email.typeKey("a", modifierFlags: .command)
        email.typeText("owner@example.test\n")
        let code = app.textFields["Sign-in code"]
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        for control in [app.staticTexts["Check your email"], code, app.buttons["Sign in"], app.buttons["Use a different email"]] {
            XCTAssertTrue(control.exists)
            XCTAssertTrue(window.frame.contains(control.frame))
            XCTAssertGreaterThanOrEqual(control.frame.minX, window.frame.minX + 24)
            XCTAssertLessThanOrEqual(control.frame.maxX, window.frame.maxX - 24)
        }
        XCTAssertEqual(code.frame.width, 342, accuracy: 1)
        XCTAssertLessThanOrEqual(code.frame.maxY, app.buttons["Sign in"].frame.minY)
        XCTAssertLessThanOrEqual(app.buttons["Sign in"].frame.maxY, app.buttons["Use a different email"].frame.minY)
        XCTAssertFalse(app.buttons["Sign in"].isEnabled)
        code.hover(); app.buttons["Use a different email"].hover()
        capture("390 six-character sign-in form and recovery controls", app: app)
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
        resize(app, width: 1280)
        XCTAssertEqual(code.value as? String, "ABC234", "Resizing the sign-in form preserves the normalized code")
        app.buttons["Sign in"].click()
        XCTAssertTrue(app.textViews["message-composer"].waitForExistence(timeout: 15))
        app.buttons["account-settings-menu"].click()
        app.menuItems["Log out"].click()
        XCTAssertTrue(email.waitForExistence(timeout: 10))
    }
}
