import XCTest
import CaperRTCBridgeIOS
import WebRTC
@testable import CaperCore

final class IOSAudioBridgeTests: XCTestCase {
    func testPinnedFactoryAndWorkerWithoutHardware() {
        RTCInitializeSSL()
        XCTAssertTrue(CaperIOSCustomAudioFactoryAvailable())
        XCTAssertNotNil(CaperCreateIOSAudioPeerFactory(.syntheticTest()))
        XCTAssertTrue(CaperIOSNativeDenoiseWorkersRunWithoutHardware(),
                      "Bundled DPDFNet at 48 kHz and RNNoise at 44.1 kHz must run without device I/O")
    }

    @MainActor
    func testRealLocalPeersReceiveOnlyEligibleSyntheticPCM() async throws {
        RTCInitializeSSL()
        let device = CaperIOSAudioDevice.syntheticTest()
        let factory = try XCTUnwrap(CaperCreateIOSAudioPeerFactory(device))
        let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)
        let caller = try XCTUnwrap(factory.peerConnection(with: VoiceClient.configuration(iceServers: []), constraints: constraints, delegate: nil))
        let callee = try XCTUnwrap(factory.peerConnection(with: VoiceClient.configuration(iceServers: []), constraints: constraints, delegate: nil))
        defer { caller.close(); callee.close() }
        let source = factory.audioSource(with: constraints)
        let track = factory.audioTrack(with: source, trackId: "ios-synthetic-pcm")
        let transceiver = try XCTUnwrap(caller.addTransceiver(with: track, init: RTCRtpTransceiverInit()))
        var directionError: NSError?
        transceiver.setDirection(.sendOnly, error: &directionError)
        XCTAssertNil(directionError)
        let offer = try await caller.offer(for: constraints)
        try await caller.setLocalDescription(offer)
        try await callee.setRemoteDescription(try XCTUnwrap(caller.localDescription))
        let answer = try await callee.answer(for: constraints)
        try await callee.setLocalDescription(answer)
        let deadline = ContinuousClock.now + .seconds(10)
        while callee.iceGatheringState != .complete || callee.localDescription?.sdp.contains("a=candidate:") != true {
            guard ContinuousClock.now < deadline else { throw URLError(.timedOut) }
            try await Task.sleep(for: .milliseconds(25))
        }
        try await caller.setRemoteDescription(try XCTUnwrap(callee.localDescription))
        try await VoiceClient.waitForConnected(caller)
        try await VoiceClient.waitForConnected(callee)
        let started = ContinuousClock.now + .seconds(5)
        while !device.syntheticRecordingActive || !device.syntheticPlayoutActive {
            guard ContinuousClock.now < started else { throw URLError(.timedOut) }
            try await Task.sleep(for: .milliseconds(25))
        }
        let wave: [Int16] = (0..<480).map { i in
            let phase = 2 * Double.pi * Double(i) / 48_000
            return Int16(11_000 * sin(220 * phase) + 3_000 * sin(440 * phase))
        }
        let pcm: Data = wave.withUnsafeBytes { (bytes: UnsafeRawBufferPointer) in Data(bytes) }
        device.publicationEnabled = false
        XCTAssertTrue(device.beginComparison())
        var gatedPeak = 0
        for _ in 0..<80 {
            XCTAssertTrue(device.injectSyntheticPCM(pcm))
            XCTAssertEqual(device.syntheticLastPublishedPeak, 0)
            if let playback = device.pullSyntheticPlayoutFrames(480) { gatedPeak = max(gatedPeak, Self.peak(playback)) }
            try await Task.sleep(for: .milliseconds(10))
        }
        let comparison = try XCTUnwrap(device.endComparison())
        XCTAssertGreaterThan(Self.peak(comparison.natural), 8_000)
        XCTAssertEqual(comparison.natural.count, comparison.enhanced.count)
        XCTAssertLessThanOrEqual(gatedPeak, 32, "Opus comfort noise may be nonzero after an exact-zero encoder input")

        device.publicationEnabled = true
        var audible = 0
        for _ in 0..<160 {
            XCTAssertTrue(device.injectSyntheticPCM(pcm))
            XCTAssertGreaterThan(device.syntheticLastPublishedPeak, 1_000)
            if let playback = device.pullSyntheticPlayoutFrames(480) { audible = max(audible, Self.peak(playback)) }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertGreaterThan(audible, 100)

        device.processingStrength = 100
        device.inputGain = 0
        XCTAssertTrue(device.beginComparison())
        for _ in 0..<30 {
            XCTAssertTrue(device.injectSyntheticPCM(pcm))
            XCTAssertEqual(device.syntheticLastPublishedPeak, 0)
        }
        let silent = try XCTUnwrap(device.endComparison())
        XCTAssertEqual(silent.natural.count, 30 * pcm.count)
        XCTAssertTrue(silent.natural.allSatisfy { $0 == 0 })
        XCTAssertTrue(silent.enhanced.allSatisfy { $0 == 0 })
        device.inputGain = 170
        for _ in 0..<10 {
            XCTAssertTrue(device.injectSyntheticPCM(pcm))
            XCTAssertGreaterThan(device.syntheticLastPublishedPeak, 1_000)
        }
    }

    private static func peak(_ pcm: Data) -> Int {
        pcm.withUnsafeBytes { $0.bindMemory(to: Int16.self).map { abs(Int($0)) }.max() ?? 0 }
    }
}

private extension RTCPeerConnection {
    func offer(for constraints: RTCMediaConstraints) async throws -> RTCSessionDescription {
        try await withCheckedThrowingContinuation { continuation in
            offer(for: constraints) { value, error in
                if let value { continuation.resume(returning: value) }
                else { continuation.resume(throwing: error ?? URLError(.badServerResponse)) }
            }
        }
    }
    func answer(for constraints: RTCMediaConstraints) async throws -> RTCSessionDescription {
        try await withCheckedThrowingContinuation { continuation in
            answer(for: constraints) { value, error in
                if let value { continuation.resume(returning: value) }
                else { continuation.resume(throwing: error ?? URLError(.badServerResponse)) }
            }
        }
    }
    func setLocalDescription(_ value: RTCSessionDescription) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            setLocalDescription(value) { error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume() }
            }
        }
    }
    func setRemoteDescription(_ value: RTCSessionDescription) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            setRemoteDescription(value) { error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume() }
            }
        }
    }
}
