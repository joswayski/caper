import XCTest
@testable import CaperCore

final class CodeResendTests: XCTestCase {
    func testWaitsAMinuteAfterEachCode() {
        let sent = Date(timeIntervalSince1970: 1_000_000)
        XCTAssertEqual(CodeResend.secondsRemaining(sentAt: sent, now: sent), 60)
        XCTAssertEqual(CodeResend.secondsRemaining(sentAt: sent, now: sent.addingTimeInterval(17.4)), 43, "rounds up")
        XCTAssertEqual(CodeResend.secondsRemaining(sentAt: sent, now: sent.addingTimeInterval(59.9)), 1)
        XCTAssertEqual(CodeResend.secondsRemaining(sentAt: sent, now: sent.addingTimeInterval(60)), 0)
        XCTAssertEqual(CodeResend.secondsRemaining(sentAt: sent, now: sent.addingTimeInterval(600)), 0)
        XCTAssertEqual(CodeResend.secondsRemaining(sentAt: nil, now: sent), 0)
    }

    func testCountdownCopy() {
        XCTAssertEqual(CodeResend.label(secondsRemaining: 60), "Resend code in 1:00")
        XCTAssertEqual(CodeResend.label(secondsRemaining: 42), "Resend code in 0:42")
        XCTAssertEqual(CodeResend.label(secondsRemaining: 5), "Resend code in 0:05")
        XCTAssertEqual(CodeResend.label(secondsRemaining: 0), "Resend code")
        XCTAssertEqual(CodeResend.maximumResends, 2, "three codes per email entry")
    }
}
