import XCTest
@testable import CaperCore

final class SpaceLandingTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let name = "SpaceLandingTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    func testRoundTripsPerAccount() {
        let store = defaults()
        XCTAssertNil(SpaceLanding.load(accountID: "a", defaults: store))
        SpaceLanding.save(SpaceLanding(spaceID: "s1", channelID: "c2"), accountID: "a", defaults: store)
        XCTAssertEqual(SpaceLanding.load(accountID: "a", defaults: store), SpaceLanding(spaceID: "s1", channelID: "c2"))
        XCTAssertNil(SpaceLanding.load(accountID: "b", defaults: store), "another account never inherits it")
        SpaceLanding.save(SpaceLanding(spaceID: "s3", channelID: nil), accountID: "a", defaults: store)
        XCTAssertEqual(SpaceLanding.load(accountID: "a", defaults: store), SpaceLanding(spaceID: "s3", channelID: nil))
    }

    func testCorruptStorageIsIgnored() {
        let store = defaults()
        store.set(Data("not json".utf8), forKey: SpaceLanding.key(accountID: "a"))
        XCTAssertNil(SpaceLanding.load(accountID: "a", defaults: store))
    }

    func testInitialSpaceOrder() {
        let ids = ["s1", "s2", "s3"]
        let saved = SpaceLanding(spaceID: "s3", channelID: "c")
        XCTAssertEqual(SpaceLanding.initialSpaceID(spaceIDs: ids, selected: nil, preferred: nil, saved: saved), "s3")
        XCTAssertEqual(SpaceLanding.initialSpaceID(spaceIDs: ids, selected: "s2", preferred: nil, saved: saved), "s2")
        XCTAssertEqual(SpaceLanding.initialSpaceID(spaceIDs: ids, selected: nil, preferred: "s2", saved: saved), "s2")
        XCTAssertEqual(SpaceLanding.initialSpaceID(spaceIDs: ids, selected: nil, preferred: nil,
                                                   saved: SpaceLanding(spaceID: "gone", channelID: nil)), "s1",
                       "a space the account left falls back to the first")
        XCTAssertNil(SpaceLanding.initialSpaceID(spaceIDs: [], selected: nil, preferred: nil, saved: saved))
    }
}
