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
}
