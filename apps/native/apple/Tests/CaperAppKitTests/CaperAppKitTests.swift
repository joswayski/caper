import AppKit
import XCTest

@MainActor final class CaperAppKitTests: XCTestCase {
    private func launch(signedIn: Bool = true, fixture: String? = nil) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["CAPER_TEST_MODE"] = "parity"
        app.launchEnvironment["CAPER_API_BASE_URL"] = "http://127.0.0.1:3001"
        app.launchEnvironment["CAPER_UI_FIXTURE"] = fixture
        if signedIn {
            app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-owner-token"
            app.launchEnvironment["CAPER_TEST_SPACE_ID"] = "space0000001"
        }
        app.launchArguments = ["-ApplePersistenceIgnoreState", "YES", "-caper.space-landing.owner0000001", ""]
        app.launch()
        return app
    }

    private func capture(_ name: String, app: XCUIApplication) {
        let screenshot = XCTAttachment(screenshot: app.windows["Caper — AppKit development (not feature complete)"].screenshot())
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

    private nonisolated static func control(_ body: [String: Any]) async throws {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:3001/__fixture/control")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (_, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
    }

    private nonisolated static func resetFixture() async throws { try await control(["reset": true]) }

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
        capture("channel preview", app: app)
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
        let styled = "**format me** and _italic_"
        XCTAssertEqual(composer.value as? String, styled, "Typing after hidden closing delimiters preserves both pairs")
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
        XCTAssertEqual(composer.value as? String, "_**menu style**_ tail")
        capture("draft formatting menus", app: app)
        app.buttons["send-message"].click()
        XCTAssertTrue(app.textViews.matching(NSPredicate(format: "value == %@", "menu style tail")).firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(composer.value as? String, "")
        composer.click(); composer.typeText("**bold** _italic_ @maya @feedback https://example.com/a_b 🙂")
        XCTAssertEqual(composer.value as? String, "**bold** _italic_ @maya @feedback https://example.com/a_b 🙂")
        capture("draft style mentions and link", app: app)
        let window = app.windows["Caper — AppKit development (not feature complete)"]
        resize(app, width: 900)
        XCTAssertTrue(window.frame.contains(composer.frame))
        capture("draft formatting narrow", app: app)
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
        XCTAssertFalse(app.buttons["Send code"].isEnabled)
        email.click(); email.typeText("not-an-email")
        app.buttons["Send code"].click()
        XCTAssertTrue(app.buttons["Send code"].waitForExistence(timeout: 10))
        XCTAssertTrue(email.isEnabled)
        XCTAssertEqual(app.staticTexts["sign-in-error"].value as? String, "Enter a valid email address.")
        capture("sign-in error", app: app)
        email.click(); email.typeKey("a", modifierFlags: .command)
        email.typeText("owner@example.test\n")
        let code = app.textFields["Sign-in code"]
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Sign in"].isEnabled)
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
        app.buttons["Sign in"].click()
        XCTAssertTrue(app.textViews["message-composer"].waitForExistence(timeout: 15))
        app.buttons["account-settings-menu"].click()
        app.menuItems["Log out"].click()
        XCTAssertTrue(email.waitForExistence(timeout: 10))
    }
}
