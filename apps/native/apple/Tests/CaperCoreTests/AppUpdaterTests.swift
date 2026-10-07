import AppKit
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
        XCTAssertNil(AppUpdater.parse(Data(#"{"update":false}"#.utf8)))
    }

    @MainActor
    func testLongNotesScrollWithinTheSpaceLeftByAlertActions() throws {
        let size = AppUpdater.notesSize(screen: NSSize(width: 800, height: 600), chromeHeight: 260)
        XCTAssertEqual(size, NSSize(width: 420, height: 260))
        XCTAssertEqual(AppUpdater.notesSize(screen: NSSize(width: 1920, height: 1080), chromeHeight: 260).height, 340)
        let update = AppUpdater.Update(version: "0.1.42", notes: "Fallback",
            changelog: [AppUpdater.ChangelogEntry(version: "0.1.42", notes: String(repeating: "• A long wrapped change\n", count: 100)),
                        AppUpdater.ChangelogEntry(version: "0.1.39", notes: "Skipped change")],
            historyComplete: false, canApply: true)
        let scroll = AppUpdater.notesView(update, size: size)
        let text = try XCTUnwrap(scroll.documentView as? NSTextView)
        XCTAssertEqual(scroll.frame.size, size)
        XCTAssertTrue(scroll.hasVerticalScroller)
        XCTAssertGreaterThan(text.frame.height, scroll.contentSize.height)
        XCTAssertTrue(text.string.contains("0.1.39\nSkipped change"))
        XCTAssertTrue(text.string.contains("Earlier release notes aren’t available"))
        XCTAssertFalse(text.string.contains("Fallback"))
    }
}
