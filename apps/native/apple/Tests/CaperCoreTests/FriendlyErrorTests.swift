import XCTest
@testable import CaperCore

/// Mirrors apps/web/src/spaces/errors.test.ts.
final class FriendlyErrorTests: XCTestCase {
    func testKnownServerErrorsReadAsSentences() {
        XCTAssertEqual(FriendlyError.message(for: APIError(status: 409, message: "channel name already exists")),
                       "A channel with that name already exists.")
        XCTAssertEqual(FriendlyError.message(for: APIError(status: 409, message: "user must join the space first")),
                       "This person needs to join the space before you can add them to a channel.")
        XCTAssertEqual(FriendlyError.message(for: APIError(status: 409, message: "user already in space")),
                       "This person is already in the space.")
        XCTAssertEqual(FriendlyError.message(for: APIError(status: 429, message: "invitation cooldown; try again after 24 hours")),
                       "This person recently responded to an invitation. You can invite them again after 24 hours.")
    }

    func testNetworkFailuresAndTimeoutsDoNotShowSystemText() {
        for code: URLError.Code in [.notConnectedToInternet, .cannotConnectToHost, .networkConnectionLost, .cannotFindHost, .dnsLookupFailed] {
            XCTAssertEqual(FriendlyError.message(for: URLError(code)), "Couldn’t reach Caper. Check your connection.", "\(code)")
        }
        XCTAssertEqual(FriendlyError.message(for: URLError(.timedOut)), "That took too long. Try again.")
        // URLSession reports failures as NSError; they bridge to URLError.
        let bridged = NSError(domain: NSURLErrorDomain, code: NSURLErrorNotConnectedToInternet)
        XCTAssertEqual(FriendlyError.message(for: bridged), "Couldn’t reach Caper. Check your connection.")
    }

    func testUnknownTextIsCapitalizedAndPunctuatedAndReadableTextIsKept() {
        XCTAssertEqual(FriendlyError.message(for: APIError(status: 429, message: "too many things; try again later")),
                       "Too many things. Try again later.")
        XCTAssertEqual(FriendlyError.message(for: APIError(status: 403, message: "This channel is no longer accessible.")),
                       "This channel is no longer accessible.")
        XCTAssertEqual(FriendlyError.message(for: UserFacingError(message: "You can't block yourself.")), "You can't block yourself.")
        XCTAssertEqual(FriendlyError.message(for: APIError(status: 500, message: "  ")), "That didn’t work. Try again.")
        XCTAssertEqual(FriendlyError.message(for: CancellationError()), "That didn’t work. Try again.")
    }

    func testDisplayMappingLeavesTheRawErrorForCodeThatBranchesOnIt() {
        let error = APIError(status: 409, message: "user already in space")
        XCTAssertEqual(FriendlyError.message(for: error), "This person is already in the space.")
        XCTAssertEqual(error.message, "user already in space")
        XCTAssertEqual(error.localizedDescription, "user already in space")
    }
}
