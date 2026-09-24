import XCTest
import CaperRTCBridge
import WebRTC
@testable import CaperCore

final class AudioBridgeTests: XCTestCase {
    func testPinnedMacOSBinaryAcceptsCustomAudioDeviceWithoutHardware() {
        RTCInitializeSSL()
        XCTAssertTrue(CaperCustomAudioFactoryAvailable(), "The pinned WebRTC framework must export M153's custom audio factory selector")
        let synthetic = CaperSyntheticAudioDevice()
        let factory = CaperCreateAudioPeerFactory(synthetic)
        XCTAssertNotNil(factory, "A header-only bridge works only if the custom ADM implementation is linked in the binary")
        _ = factory
        // No tracks or peer connections: this probe must not request microphone access or open devices.
    }

    func testSyntheticDelegateMovesExactPCMWithoutHardware() {
        XCTAssertTrue(CaperSyntheticAudioCallbacksWork(), "The M153 delegate contract must exchange signed mono PCM in both directions")
    }

    func testLiveCaptureGainAndProcessingOnSyntheticPCM() {
        XCTAssertTrue(CaperSyntheticVoiceDSPWorks(), "0% processing retains asymmetric 200% gain and 100% strength transforms speech")
    }

    func testComparisonStopBetweenRegistrationAndRecheckReleasesCallback() {
        XCTAssertTrue(CaperSyntheticComparisonStopWorks(), "A callback entered before stop must leave even when its second check skips copying")
    }

    func testPublishedPCMGateSilencesCaptureUntilExplicitlyEnabled() {
        XCTAssertTrue(CaperSyntheticPublicationGateWorks(), "Silence reaches WebRTC while pre-ready, muted, deafened, or comparing; enabled asymmetric PCM remains intact")
    }

    func testMacAudioControlsClampAndRejectUnavailableRouteWithoutOpeningHardware() {
        let device = CaperMacAudioDevice()
        XCTAssertEqual(device.inputUID, "")
        XCTAssertEqual(device.outputUID, "")
        XCTAssertFalse(device.selectInputUID("missing-microphone-uid"))
        XCTAssertFalse(device.selectOutputUID("missing-speaker-uid"))
        XCTAssertEqual(device.inputUID, "")
        XCTAssertEqual(device.outputUID, "")
        device.inputGain = 250
        device.processingStrength = -3
        XCTAssertEqual(device.inputGain, 200)
        XCTAssertEqual(device.processingStrength, 0)
    }

    @MainActor
    func testLocalPeersCarryOnlyAllowedSyntheticCapturePCM() async throws {
        RTCInitializeSSL()
        let device = CaperMacAudioDevice.syntheticTestDevice()
        let factory = try XCTUnwrap(CaperCreateAudioPeerFactory(device))
        let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)
        let caller = try XCTUnwrap(factory.peerConnection(with: VoiceClient.configuration(iceServers: []), constraints: constraints, delegate: nil))
        let callee = try XCTUnwrap(factory.peerConnection(with: VoiceClient.configuration(iceServers: []), constraints: constraints, delegate: nil))
        defer { caller.close(); callee.close() }

        let source = factory.audioSource(with: constraints)
        let track = factory.audioTrack(with: source, trackId: "synthetic-pcm")
        let transceiver = try XCTUnwrap(caller.addTransceiver(with: track, init: RTCRtpTransceiverInit()))
        var directionError: NSError?
        transceiver.setDirection(.sendOnly, error: &directionError)
        XCTAssertNil(directionError)

        let offer = try await caller.offer(for: constraints)
        try await caller.setLocalDescription(offer)
        try await callee.setRemoteDescription(try XCTUnwrap(caller.localDescription))
        let answer = try await callee.answer(for: constraints)
        try await callee.setLocalDescription(answer)
        let gatherDeadline = ContinuousClock.now + .seconds(10)
        while callee.iceGatheringState != .complete || callee.localDescription?.sdp.contains("a=candidate:") != true {
            guard ContinuousClock.now < gatherDeadline else { throw URLError(.timedOut) }
            try await Task.sleep(for: .milliseconds(25))
        }
        try await caller.setRemoteDescription(try XCTUnwrap(callee.localDescription))
        try await VoiceClient.waitForConnected(caller)
        try await VoiceClient.waitForConnected(callee)
        let startDeadline = ContinuousClock.now + .seconds(5)
        while !device.syntheticRecordingActive || !device.syntheticPlayoutActive {
            guard ContinuousClock.now < startDeadline else { throw URLError(.timedOut) }
            try await Task.sleep(for: .milliseconds(25))
        }

        // Speech-like asymmetric signal, not a trivial zero/symmetric input. PCM
        // takes the exact production capture/DSP/gate path into M153's real ADM.
        let wave = (0..<480).map { i in Int16(11_000 * sin(2 * Double.pi * 220 * Double(i) / 48_000)
            + 3_000 * sin(2 * Double.pi * 440 * Double(i) / 48_000)) }
        let pcm = wave.withUnsafeBytes { Data($0) }
        device.publicationEnabled = false
        XCTAssertTrue(device.beginComparison())
        var gatedPeak = 0
        for _ in 0..<80 {
            XCTAssertTrue(device.injectSyntheticPCM(pcm))
            if let output = device.pullSyntheticPlayoutFrames(480) { gatedPeak = max(gatedPeak, Self.peak(output)) }
            try await Task.sleep(for: .milliseconds(10))
        }
        let local = try XCTUnwrap(device.endComparison())
        XCTAssertEqual(local.natural.count, local.enhanced.count)
        XCTAssertEqual(local.sampleRate, 48_000)
        XCTAssertGreaterThan(Self.peak(local.natural), 8_000, "Raw local comparison must retain the injected signal")
        XCTAssertEqual(gatedPeak, 0, "Actual peer playout must not receive mic samples while publication is gated")

        device.publicationEnabled = true
        var audiblePeak = 0
        for _ in 0..<200 {
            XCTAssertTrue(device.injectSyntheticPCM(pcm))
            if let output = device.pullSyntheticPlayoutFrames(480) { audiblePeak = max(audiblePeak, Self.peak(output)) }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertGreaterThan(audiblePeak, 100, "The real local peer must decode nonzero PCM when publication opens")
    }

    private static func peak(_ data: Data) -> Int {
        data.withUnsafeBytes { bytes in bytes.bindMemory(to: Int16.self).map { abs(Int($0)) }.max() ?? 0 }
    }
}
