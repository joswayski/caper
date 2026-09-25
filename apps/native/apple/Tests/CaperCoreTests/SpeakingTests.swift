import XCTest
@testable import CaperCore

final class SpeakingTests: XCTestCase {
    func testSpeechThresholdReleaseAndMute() {
        var lastLoud: [String: Date] = [:]
        let start = Date(timeIntervalSince1970: 1_000)
        // Web's VoiceActivity threshold is RMS 0.004.
        XCTAssertEqual(VoiceClient.speaking(levels: ["maya": 0.005, "alex": 0.003], muted: [], lastLoud: &lastLoud, now: start), ["maya"])
        // Stays lit through the 180 ms release after the last loud sample.
        XCTAssertEqual(VoiceClient.speaking(levels: ["maya": 0], muted: [], lastLoud: &lastLoud, now: start.addingTimeInterval(0.15)), ["maya"])
        XCTAssertEqual(VoiceClient.speaking(levels: ["maya": 0], muted: [], lastLoud: &lastLoud, now: start.addingTimeInterval(0.2)), [])
        // A muted participant is never shown speaking, and muting clears a lit one.
        XCTAssertEqual(VoiceClient.speaking(levels: ["maya": 0.5], muted: ["maya"], lastLoud: &lastLoud, now: start.addingTimeInterval(1)), [])
        _ = VoiceClient.speaking(levels: ["alex": 0.5], muted: [], lastLoud: &lastLoud, now: start.addingTimeInterval(2))
        XCTAssertEqual(VoiceClient.speaking(levels: [:], muted: ["alex"], lastLoud: &lastLoud, now: start.addingTimeInterval(2.05)), [])
    }
}
