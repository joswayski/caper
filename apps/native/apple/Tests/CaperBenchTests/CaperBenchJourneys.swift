import XCTest

/// Device benchmark journeys (bench.sh). The same file is built into
/// CaperBenchUIKitTests and CaperBenchSwiftUITests, so each scheme measures its
/// own bench app (`XCUIApplication()`) the same way. The app talks only to the
/// fixture in CAPER_FIXTURE_URL (bench.sh passes TEST_RUNNER_CAPER_FIXTURE_URL),
/// started with CAPER_BENCH_MESSAGES=3000 and CAPER_BENCH_FORMATTED_MESSAGES.
/// Nothing is sent: the journeys read history and type a draft they never post.
@MainActor
final class CaperBenchJourneys: XCTestCase {
    /// A fixture channel: its ID, name and row count, and how its rows are
    /// identified. UIKit rows use the clientMessageId, SwiftUI rows the message
    /// ID; both end in the row's seq.
    private struct Channel {
        let id, name: String
        let count: Int
        let idPrefix, clientPrefix: String

        func rowIdentifiers(seq: Int) -> [String] {
            let padded14 = String(format: "%014ld", seq), padded12 = String(format: "%012ld", seq)
            return ["message-row-\(idPrefix)\(padded14)", "message-row-\(clientPrefix)\(padded12)"]
        }
    }

    private static let general = Channel(id: "chan00000001", name: "general", count: 3000,
                                         idPrefix: "b", clientPrefix: "00000000-0000-4000-9000-")
    private static var formatting: Channel {
        let count = Int(ProcessInfo.processInfo.environment["CAPER_BENCH_FORMATTED_MESSAGES"] ?? "") ?? 400
        return Channel(id: "chan00000070", name: "bench-formatting", count: count,
                       idPrefix: "f", clientPrefix: "00000000-0000-4000-a000-")
    }
    /// The fixture's history page size: launch shows only the newest 50 rows.
    private static let pageSize = 50
    /// Long, formatted and link-heavy, without Return (which would send).
    private static let draft = "**Release check** for _the voice beta_: see https://example.com/docs/42/release-notes?tab=voice#section-3 "
        + "and www.example.org/specs/17, plus <https://example.net/no-card/9> (also https://example.com/wiki/Caper_(app)/7). "
        + "@maya can you confirm **both _builds_** and *the narrow layout*? Then https://example.com/a/very/long/path/segment-1234.html. "
        + "**Done** when _history loading_ and **link previews** look right on https://example.org/checklist?step=4#final."
    private static var localNetworkAllowed = false

    private var fixtureURL = ""

    override func setUpWithError() throws {
        continueAfterFailure = false
        guard let url = ProcessInfo.processInfo.environment["CAPER_FIXTURE_URL"], !url.isEmpty else {
            throw XCTSkip("Set TEST_RUNNER_CAPER_FIXTURE_URL to the bench fixture (bench.sh does).")
        }
        fixtureURL = url
        try allowLocalNetworkOnce()
    }

    private func configured(cache: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += ["-caper.space-landing.owner0000001", "", "-caper.channelSidebarChannelsShare", "0.5"]
        app.launchEnvironment["CAPER_TEST_MODE"] = "parity"
        app.launchEnvironment["CAPER_API_BASE_URL"] = fixtureURL
        app.launchEnvironment["CAPER_TEST_BEARER"] = "fixture-owner-token"
        app.launchEnvironment["CAPER_TEST_SPACE_ID"] = "space0000001"
        // Only the UIKit app has a startup cache; the SwiftUI app ignores this.
        if cache { app.launchEnvironment["CAPER_UIKIT_STARTUP_CACHE"] = "1" }
        return app
    }

    /// iOS asks once per app before it reaches the Mac's LAN address.
    private func allowLocalNetworkOnce() throws {
        guard !Self.localNetworkAllowed else { return }
        let app = configured()
        app.launch()
        let allow = XCUIApplication(bundleIdentifier: "com.apple.springboard").buttons["Allow"]
        if allow.waitForExistence(timeout: 5) { allow.tap() }
        app.terminate()
        Self.localNetworkAllowed = true
    }

    private func rows(in app: XCUIApplication) -> XCUIElementQuery {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'message-row-'"))
    }

    private func row(_ channel: Channel, seq: Int, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier IN %@", channel.rowIdentifiers(seq: seq))).firstMatch
    }

    /// The seqs of the message rows currently in the accessibility tree.
    private func visibleSeqs(in app: XCUIApplication) -> [Int] {
        rows(in: app).allElementsBoundByIndex.compactMap { element in
            Int(String(element.identifier.reversed().prefix { $0.isNumber }.reversed()))
        }
    }

    /// Launches into #general, opens `channel`, and checks that its newest page
    /// loaded: the title names it and its newest fixture row is on screen.
    private func open(_ channel: Channel, in app: XCUIApplication) {
        app.launch()
        let composer = app.textViews["message-composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 30), "The app must reach the fixture at \(fixtureURL)")
        if channel.id != Self.general.id {
            if !app.buttons["Close navigation"].isHittable { app.buttons["Back to Browse"].tap() }
            let entry = app.descendants(matching: .any)["channel-\(channel.id)"].firstMatch
            XCTAssertTrue(entry.waitForExistence(timeout: 10),
                          "Missing #\(channel.name): start the fixture with CAPER_BENCH_FORMATTED_MESSAGES")
            entry.tap()
            XCTAssertTrue(composer.waitForExistence(timeout: 10))
        }
        let title = app.descendants(matching: .any)["channel-title-menu"].firstMatch
        let titled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "#\(channel.name) options"), object: title)
        XCTAssertEqual(XCTWaiter.wait(for: [titled], timeout: 10), .completed, "#\(channel.name) must be open, not \(title.label)")
        XCTAssertTrue(row(channel, seq: channel.count, in: app).waitForExistence(timeout: 15),
                      "#\(channel.name)'s newest row (seq \(channel.count)) must be loaded before measuring")
        // Let the first page and its artwork settle outside the measurement.
        sleep(2)
        let oldest = visibleSeqs(in: app).min() ?? 0
        XCTAssertGreaterThan(oldest, channel.count - Self.pageSize, "Only the newest page may be loaded before measuring")
    }

    private func options(_ iterations: Int = 5) -> XCTMeasureOptions {
        let options = XCTMeasureOptions()
        options.iterationCount = iterations
        options.invocationOptions = [.manuallyStart, .manuallyStop]
        return options
    }

    /// Process start to first frame, without and with the UIKit startup cache.
    func testLaunch() {
        let launches = XCTMeasureOptions()
        launches.iterationCount = 5
        measure(metrics: [XCTApplicationLaunchMetric(waitUntilResponsive: false)], options: launches) {
            configured().launch()
        }
    }

    func testLaunchWithStartupCache() {
        let first = configured(cache: true)
        open(Self.general, in: first)
        XCUIDevice.shared.press(.home)
        sleep(2)
        first.terminate()
        let launches = XCTMeasureOptions()
        launches.iterationCount = 5
        measure(metrics: [XCTApplicationLaunchMetric(waitUntilResponsive: false)], options: launches) {
            configured(cache: true).launch()
        }
    }

    /// Fast flings up through #general's 3,000 short rows, loading older pages.
    func testScrollHistory() {
        scrollHistory(in: Self.general)
    }

    /// The same through #bench-formatting's long, formatted, link-heavy rows.
    func testScrollFormattedHistory() {
        scrollHistory(in: Self.formatting)
    }

    private func scrollHistory(in channel: Channel) {
        let app = configured()
        measure(metrics: [XCTOSSignpostMetric.scrollingAndDecelerationMetric, XCTCPUMetric(application: app),
                          XCTMemoryMetric(application: app), XCTClockMetric()], options: options()) {
            // A fresh launch each time, so every iteration loads history cold.
            open(channel, in: app)
            // UIKit's collection view or SwiftUI's scroll view, not the whole window.
            let timeline = app.descendants(matching: .any)
                .matching(NSPredicate(format: "identifier IN %@", ["message-timeline", "chat-timeline"])).firstMatch
            XCTAssertTrue(timeline.exists, "Missing the message timeline")
            startMeasuring()
            for _ in 0..<8 { timeline.swipeDown(velocity: .fast) }
            sleep(1)
            stopMeasuring()
            // The flings must have reached rows older than the launch page.
            let oldest = visibleSeqs(in: app).min() ?? .max
            XCTAssertLessThanOrEqual(oldest, channel.count - Self.pageSize,
                                     "Scrolling must load history older than the newest \(Self.pageSize) rows")
            app.terminate()
        }
    }

    /// Types the formatted, link-heavy draft into #bench-formatting's composer
    /// (live highlighting, link and mention detection); it is never sent.
    func testTypeFormattedDraft() {
        let app = configured()
        measure(metrics: [XCTCPUMetric(application: app), XCTMemoryMetric(application: app), XCTClockMetric()],
                options: options()) {
            open(Self.formatting, in: app)
            let composer = app.textViews["message-composer"]
            composer.tap()
            XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
            XCTAssertEqual((composer.value as? String) ?? "", "", "Every iteration must start from an empty draft")
            startMeasuring()
            composer.typeText(Self.draft)
            stopMeasuring()
            XCTAssertEqual(composer.value as? String, Self.draft, "The exact draft must reach the composer, unchanged")
            app.terminate()
        }
    }
}
