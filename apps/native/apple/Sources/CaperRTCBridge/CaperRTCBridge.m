#import "CaperRTCBridge.h"
#import "sdk/objc/components/audio/RTCAudioDevice.h"

// Match the M153 selector even if the distributed macOS framework omits the declaration.
@interface RTCPeerConnectionFactory (CaperAudioFactory)
- (instancetype)initWithEncoderFactory:(nullable id<RTCVideoEncoderFactory>)encoderFactory
                         decoderFactory:(nullable id<RTCVideoDecoderFactory>)decoderFactory
                            audioDevice:(nullable id<RTCAudioDevice>)audioDevice;
@end

BOOL CaperCustomAudioFactoryAvailable(void) {
    return [[RTCPeerConnectionFactory alloc] respondsToSelector:
            @selector(initWithEncoderFactory:decoderFactory:audioDevice:)];
}

RTCPeerConnectionFactory *CaperCreateAudioPeerFactory(NSObject *device) {
    if (!CaperCustomAudioFactoryAvailable()) { return nil; }
    if (![device conformsToProtocol:@protocol(RTCAudioDevice)]) { return nil; }
    return [[RTCPeerConnectionFactory alloc] initWithEncoderFactory:nil decoderFactory:nil audioDevice:(id<RTCAudioDevice>)device];
}

@interface CaperSyntheticAudioDevice () <RTCAudioDevice>
@end

@implementation CaperSyntheticAudioDevice {
    id<RTCAudioDeviceDelegate> _delegate;
}
- (double)deviceInputSampleRate { return 48000; }
- (NSTimeInterval)inputIOBufferDuration { return 0.02; }
- (NSInteger)inputNumberOfChannels { return 1; }
- (NSTimeInterval)inputLatency { return 0; }
- (double)deviceOutputSampleRate { return 48000; }
- (NSTimeInterval)outputIOBufferDuration { return 0.02; }
- (NSInteger)outputNumberOfChannels { return 1; }
- (NSTimeInterval)outputLatency { return 0; }
- (BOOL)isInitialized { return _delegate != nil; }
- (BOOL)initializeWithDelegate:(id<RTCAudioDeviceDelegate>)delegate { _delegate = delegate; return YES; }
- (BOOL)terminateDevice { _delegate = nil; return YES; }
- (BOOL)isPlayoutInitialized { return NO; }
- (BOOL)initializePlayout { return NO; }
- (BOOL)isPlaying { return NO; }
- (BOOL)startPlayout { return NO; }
- (BOOL)stopPlayout { return YES; }
- (BOOL)isRecordingInitialized { return NO; }
- (BOOL)initializeRecording { return NO; }
- (BOOL)isRecording { return NO; }
- (BOOL)startRecording { return NO; }
- (BOOL)stopRecording { return YES; }
@end
