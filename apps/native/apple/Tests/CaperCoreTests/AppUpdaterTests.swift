import XCTest

final class AppUpdaterTests: XCTestCase {
    @MainActor
    func testDownloadLinksToTheRunningArchitectureArchive() {
        #if arch(arm64)
        XCTAssertEqual(AppUpdater.downloadURL.absoluteString,
            "https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Apple-Silicon.zip")
        #else
        XCTAssertEqual(AppUpdater.downloadURL.absoluteString,
            "https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Intel.zip")
        #endif
    }
}
