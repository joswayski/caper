import XCTest
import CaperRTCBridge
import WebRTC

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
}
