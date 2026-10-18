import AppKit
import CaperCore
import XCTest

final class AppUpdaterTests: XCTestCase {
    @MainActor
    func testChecksEveryMinuteWithTheExistingStartupDelay() {
        XCTAssertEqual(AppUpdater.firstCheckDelay, 20)
        XCTAssertEqual(AppUpdater.checkInterval, 60)
    }

    @MainActor
    func testDownloadLinksToTheRunningArchitectureInstaller() {
        #if arch(arm64)
        XCTAssertEqual(AppUpdater.downloadURL.absoluteString,
            "https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Apple-Silicon.dmg")
        #else
        XCTAssertEqual(AppUpdater.downloadURL.absoluteString,
            "https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Intel.dmg")
        #endif
    }

    func testOnlyWritableApplicationLocationsCanOpenReleaseClients() {
        let home = URL(fileURLWithPath: "/Users/fixture")
        for path in ["/Applications/Caper.app", "/Applications/Utilities/Caper.app",
                     "/Users/fixture/Applications/Caper.app"] {
            XCTAssertFalse(AppUpdater.needsInstallation(URL(fileURLWithPath: path), home: home,
                                                       parentWritable: true), path)
            XCTAssertTrue(AppUpdater.needsInstallation(URL(fileURLWithPath: path), home: home,
                                                      parentWritable: false), path)
        }
        for path in ["/Users/fixture/Downloads/Caper.app", "/Volumes/Caper/Caper.app",
                     "/private/var/folders/fixture/AppTranslocation/id/d/Caper.app",
                     "/Applications-old/Caper.app", "/Users/fixture/Applications-old/Caper.app",
                     "/Users/another/Applications/Caper.app",
                     "/Applications/../Downloads/Caper.app"] {
            XCTAssertTrue(AppUpdater.needsInstallation(URL(fileURLWithPath: path), home: home,
                                                      parentWritable: true), path)
        }
    }

    @MainActor
    func testFixturesDoNotPromptOrModifyInstallation() {
        let updater = AppUpdater(environment: ["CAPER_TEST_MODE": "parity"])
        XCTAssertFalse(updater.isAvailable)
        XCTAssertTrue(updater.prepareInstallation())
        XCTAssertNil(updater.controls.check)
        XCTAssertNil(updater.controls.review)
        XCTAssertFalse(updater.controls.isChecking)
    }

    @MainActor
    func testInstalledVersionComesFromTheRunningBundleNotTheLatestRelease() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).app")
        let contents = folder.appendingPathComponent("Contents")
        try FileManager.default.createDirectory(at: contents, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let info: [String: String] = [
            "CFBundleIdentifier": "chat.caper.version-fixture",
            "CFBundleShortVersionString": "0.2.7",
            "CFBundleVersion": "42",
            "CaperSourceRevision": String(repeating: "abcdef0123", count: 4),
        ]
        let data = try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0)
        try data.write(to: contents.appendingPathComponent("Info.plist"))
        let bundle = try XCTUnwrap(Bundle(url: folder))
        let updater = AppUpdater(bundle: bundle, environment: ["CAPER_TEST_MODE": "parity"])
        XCTAssertEqual(updater.installedVersion, "Installed: Caper 0.2.7 (build 42) · abcdef0")
        XCTAssertFalse(updater.isAvailable, "Showing the installed version must not enable update checks")
    }

    func testStackedNotesAndLegacyOutputRemainReadable() throws {
        let data = Data(#"{"update":true,"version":"0.1.42","notes":"Latest only","changelog":[{"build":42,"version":"0.1.42","notes":"Newest"},{"build":39,"version":"0.1.39","notes":"Skipped"}],"history_complete":true,"can_apply":true}"#.utf8)
        let update = try XCTUnwrap(AppUpdater.parse(data))
        XCTAssertTrue(update.historyComplete)
        XCTAssertEqual(update.changelog.map(\.version), ["0.1.42", "0.1.39"])
        XCTAssertEqual(update.changelog.last?.notes, "Skipped")
        let legacy = try XCTUnwrap(AppUpdater.parse(Data(#"{"update":true,"version":"0.1.7","notes":"Legacy"}"#.utf8)))
        XCTAssertEqual(legacy.notes, "Legacy")
        XCTAssertTrue(legacy.changelog.isEmpty)
        XCTAssertFalse(legacy.historyComplete)
        XCTAssertFalse(legacy.canApply)
        XCTAssertNil(legacy.lastError)
        let failed = try XCTUnwrap(AppUpdater.parse(Data(#"{"update":true,"version":"0.1.7","last_error":"Access denied"}"#.utf8)))
        XCTAssertEqual(failed.lastError, "Access denied")
        XCTAssertNil(AppUpdater.parse(Data(#"{"update":false}"#.utf8)))
        XCTAssertNil(legacy.highlights)
    }

    func testStructuredHighlightsShowADateInsteadOfAVersion() throws {
        let data = Data(#"{"update":true,"version":"0.1.82","notes":"plain","history_complete":true,"highlights":[{"title":"Voice in the channel list","body":"Hop in with one click."}],"released":"2026-10-18","can_apply":true}"#.utf8)
        let update = try XCTUnwrap(AppUpdater.parse(data))
        XCTAssertEqual(update.highlights, [DesktopUpdate.Highlight(title: "Voice in the channel list", body: "Hop in with one click.")])
        XCTAssertEqual(update.released, "2026-10-18")
        let label = try XCTUnwrap(update.releasedLabel)
        XCTAssertTrue(label.contains("2026"))
        XCTAssertFalse(label.contains("0.1.82"))
    }

    @MainActor
    func testReviewKeepsAllNotesNewestFirstAndPreservesErrorsAndHistoryGap() {
        let update = AppUpdater.Update(version: "0.1.42", notes: "Fallback",
            changelog: [AppUpdater.ChangelogEntry(version: "0.1.42", notes: String(repeating: "A long change\n", count: 100)),
                        AppUpdater.ChangelogEntry(version: "0.1.9", notes: "Skipped change")],
            historyComplete: false, canApply: true, lastError: "Access denied")
        XCTAssertEqual(update.entries.map(\.version), ["0.1.42", "0.1.9"])
        XCTAssertEqual(update.entries.first?.notes, String(repeating: "A long change\n", count: 100))
        XCTAssertEqual(update.entries.last?.notes, "Skipped change")
        XCTAssertTrue(update.historyNotice.contains("Earlier release notes aren’t available"))
        XCTAssertEqual(update.lastError, "Access denied")
    }

    @MainActor
    func testAutomaticResultsAdvertiseQuietlyAndLaterDoesNotLoseReview() throws {
        let updater = AppUpdater(environment: ["CAPER_TEST_MODE": "parity"])
        updater.finish(.success(Data(#"{"update":true,"version":"0.1.42","notes":"Changes","can_apply":false}"#.utf8)))
        XCTAssertNil(updater.offeredUpdate, "Automatic checks never take focus")
        XCTAssertFalse(updater.checking)
        let review = try XCTUnwrap(updater.controls.review)
        review()
        XCTAssertEqual(updater.offeredUpdate?.version, "0.1.42")
        XCTAssertEqual(updater.offeredUpdate?.entries.first?.notes, "Changes")
        XCTAssertFalse(updater.controls.canApply)
        updater.applyReviewedUpdate()
        XCTAssertNotNil(updater.offeredUpdate, "Installer-only copies cannot apply")
        updater.dismissReview()
        XCTAssertNil(updater.offeredUpdate)
        review()
        XCTAssertEqual(updater.offeredUpdate?.version, "0.1.42", "Later preserves the available update")
        updater.finish(.success(Data(#"{"update":true,"version":"0.1.43","notes":"New changes"}"#.utf8)))
        XCTAssertEqual(updater.offeredUpdate?.version, "0.1.43", "An open review follows refreshed releases")
        updater.finish(.failure(NSError(domain: "test", code: 1)))
        XCTAssertEqual(updater.offeredUpdate?.version, "0.1.43", "A failed automatic check preserves review")
        updater.finish(.success(Data(#"{"update":false}"#.utf8)))
        XCTAssertNil(updater.offeredUpdate)
        XCTAssertNil(updater.controls.review)
    }
}
