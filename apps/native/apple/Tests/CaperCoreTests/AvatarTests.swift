import XCTest
@testable import CaperCore

final class AvatarTests: XCTestCase {
    func testPersistedIndicesAndFallback() throws {
        for index in [0, 31, 32, 255, 256, 799] { XCTAssertEqual(CaperAvatar.index(for: index), index) }
        for index in [nil, -1, 800] { XCTAssertNil(CaperAvatar.index(for: index)) }

        XCTAssertNil(try JSONDecoder().decode(Account.self, from: Data(#"{"id":"old"}"#.utf8)).avatarId)
        XCTAssertEqual(try JSONDecoder().decode(Account.self, from: Data(#"{"id":"saved","avatarId":16}"#.utf8)).avatarId, 16)
    }
}
