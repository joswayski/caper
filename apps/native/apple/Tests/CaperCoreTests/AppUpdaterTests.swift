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
        XCTAssertNil(legacy.lastError)
        let failed = try XCTUnwrap(AppUpdater.parse(Data(#"{"update":true,"version":"0.1.7","last_error":"Access denied"}"#.utf8)))
        XCTAssertEqual(failed.lastError, "Access denied")
        XCTAssertNil(AppUpdater.parse(Data(#"{"update":false}"#.utf8)))
    }

    @MainActor
    func testLongNotesScrollWithinTheSpaceLeftByAlertActions() throws {
        let size = AppUpdater.notesSize(screen: NSSize(width: 800, height: 600), chromeHeight: 260)
        XCTAssertEqual(size, NSSize(width: 420, height: 260))
        XCTAssertEqual(AppUpdater.notesSize(screen: NSSize(width: 1920, height: 1080), chromeHeight: 260).height, 340)
        let update = AppUpdater.Update(version: "0.1.42", notes: "Fallback",
            changelog: [AppUpdater.ChangelogEntry(version: "0.1.42", notes: String(repeating: "• A long change that must wrap across multiple lines at the alert width without changing the scroll range as you read it.\n", count: 100)),
                        AppUpdater.ChangelogEntry(version: "0.1.39", notes: "Skipped change")],
            historyComplete: false, canApply: true, lastError: "Access denied")
        let scroll = AppUpdater.notesView(update, size: size)
        let text = try XCTUnwrap(scroll.documentView as? NSTextView)
        XCTAssertEqual(scroll.frame.size, size)
        XCTAssertTrue(scroll.hasVerticalScroller)
        XCTAssertGreaterThan(text.frame.height, scroll.contentSize.height)
        XCTAssertTrue(text.string.contains("0.1.39\nSkipped change"))
        XCTAssertTrue(text.string.contains("Earlier release notes aren’t available"))
        XCTAssertTrue(text.string.contains("Access denied"))
        XCTAssertFalse(text.string.contains("Fallback"))
        XCTAssertTrue(text.isSelectable)
        XCTAssertFalse(text.isEditable)
        XCTAssertFalse(scroll.hasHorizontalScroller)
        XCTAssertEqual(text.frame.width, scroll.contentSize.width, accuracy: 0.5)

        let documentSize = text.frame.size
        let end = (text.string as NSString).range(of: "Skipped change")
        XCTAssertNotEqual(end.location, NSNotFound)
        text.scrollRangeToVisible(end)
        let layout = try XCTUnwrap(text.layoutManager)
        let container = try XCTUnwrap(text.textContainer)
        let glyphs = layout.glyphRange(forCharacterRange: end, actualCharacterRange: nil)
        let endRect = layout.boundingRect(forGlyphRange: glyphs, in: container)
            .offsetBy(dx: text.textContainerOrigin.x, dy: text.textContainerOrigin.y)
        let visible = scroll.contentView.documentVisibleRect
        XCTAssertGreaterThanOrEqual(visible.maxY, endRect.maxY - 1, "The last release must be reachable")
        XCTAssertLessThanOrEqual(visible.minY, endRect.minY)

        let lastOffset = documentSize.height - scroll.contentSize.height
        for offset in [CGFloat.zero, lastOffset / 2, lastOffset, CGFloat.zero] {
            scroll.contentView.scroll(to: NSPoint(x: 0, y: offset))
            scroll.reflectScrolledClipView(scroll.contentView)
            scroll.layoutSubtreeIfNeeded()
            XCTAssertEqual(text.frame.width, documentSize.width, accuracy: 0.5, "Scrolling must not rewrap the document")
            XCTAssertEqual(text.frame.height, documentSize.height, accuracy: 0.5, "Scrolling must not change the document height")
            XCTAssertEqual(scroll.frame.size, size, "Update controls keep their reserved space")
        }
    }
}
