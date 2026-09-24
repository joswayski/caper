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
}
