import XCTest
import WebRTC
@testable import CaperCore

@MainActor
final class VoiceTransportTests: XCTestCase {
    func testImmediateNativeSDPConnectsAndRestartsWithServerCandidates() async throws {
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
            let local = try XCTUnwrap(caller.localDescription)
            // Exercise the asymmetric SFU flow even if this machine gathers
            // unusually quickly: the caller need not supply any candidates.
            let offer = RTCSessionDescription(type: .offer, sdp: local.sdp.components(separatedBy: "\r\n")
                .filter { !$0.hasPrefix("a=candidate:") && $0 != "a=end-of-candidates" }
                .joined(separator: "\r\n"))
            let credentials = try XCTUnwrap(offer.sdp.components(separatedBy: "\r\n").first { $0.hasPrefix("a=ice-ufrag:") })
            if let previousCredentials { XCTAssertTrue(credentials != previousCredentials, "Restart must gather a fresh ICE generation") }
            previousCredentials = credentials
            try await callee.setRemoteDescription(offer)
            let pendingAnswer = try await callee.answer(for: constraints)
            try await callee.setLocalDescription(pendingAnswer)
            // Only the local server stand-in needs a gathered answer. After
            // restart, COMPLETE can briefly describe the old ICE generation.
            let gatheringDeadline = ContinuousClock.now + .seconds(10)
            while callee.iceGatheringState != .complete || callee.localDescription?.sdp.contains("a=candidate:") != true {
                guard ContinuousClock.now < gatheringDeadline else { throw URLError(.timedOut) }
                try await Task.sleep(for: .milliseconds(25))
            }
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
