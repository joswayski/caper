import XCTest
import WebRTC
@testable import CaperCore

@MainActor
final class VoiceTransportTests: XCTestCase {
    func testGatheredNativeSDPConnectsWithoutTrickleAndCanRestart() async throws {
        RTCInitializeSSL()
        let factory = RTCPeerConnectionFactory()
        let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)
        let caller = try XCTUnwrap(factory.peerConnection(with: VoiceClient.configuration(iceServers: []), constraints: constraints, delegate: nil))
        let callee = try XCTUnwrap(factory.peerConnection(with: VoiceClient.configuration(iceServers: []), constraints: constraints, delegate: nil))
        defer { caller.close(); callee.close() }
        // Exercise the real native transport with no microphone, external
        // service, mock SDP, candidate forwarding or physical-audio claim.
        let channel = try XCTUnwrap(caller.dataChannel(forLabel: "ice-regression", configuration: RTCDataChannelConfiguration()))
        var previousCredentials: String?
        for restarting in [false, true] {
            if restarting { caller.restartIce() }
            let pendingOffer = try await caller.offer(for: constraints)
            try await caller.setLocalDescription(pendingOffer)
            try await VoiceClient.waitForGathering(caller)
            let offer = try XCTUnwrap(caller.localDescription)
            XCTAssertTrue(offer.sdp.contains("a=candidate:"), "The non-trickle offer must actually contain gathered candidates")
            let credentials = try XCTUnwrap(offer.sdp.components(separatedBy: "\r\n").first { $0.hasPrefix("a=ice-ufrag:") })
            if let previousCredentials { XCTAssertTrue(credentials != previousCredentials, "Restart must gather a fresh ICE generation") }
            previousCredentials = credentials
            try await callee.setRemoteDescription(offer)
            let pendingAnswer = try await callee.answer(for: constraints)
            try await callee.setLocalDescription(pendingAnswer)
            try await VoiceClient.waitForGathering(callee)
            let answer = try XCTUnwrap(callee.localDescription)
            XCTAssertTrue(answer.sdp.contains("a=candidate:"))
            try await caller.setRemoteDescription(answer)
            try await VoiceClient.waitForConnected(caller)
            try await VoiceClient.waitForConnected(callee)
        }
        let deadline = ContinuousClock.now + .seconds(5)
        while channel.readyState != .open && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(25))
        }
        XCTAssertEqual(channel.readyState, .open, "A real DTLS/SCTP connection must open, not just parse the SDP")
    }
}
