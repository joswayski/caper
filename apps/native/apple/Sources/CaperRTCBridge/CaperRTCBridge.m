#import "CaperRTCBridge.h"
#import "CaperVoiceDSP.h"
#import "sdk/objc/components/audio/RTCAudioDevice.h"
#import <stdlib.h>

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
- (BOOL)exerciseCallbacks;
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

- (BOOL)exerciseCallbacks {
    if (!_delegate) { return NO; }
    int16_t input[] = {100, -200, 300, -400};
    int16_t output[] = {0, 0, 0, 0};
    AudioBufferList inputList = {.mNumberBuffers = 1,
        .mBuffers = {{.mNumberChannels = 1, .mDataByteSize = sizeof(input), .mData = input}}};
    AudioBufferList outputList = {.mNumberBuffers = 1,
        .mBuffers = {{.mNumberChannels = 1, .mDataByteSize = sizeof(output), .mData = output}}};
    AudioUnitRenderActionFlags flags = 0;
    AudioTimeStamp timestamp = {0};
    if (_delegate.deliverRecordedData(&flags, &timestamp, 1, 4, &inputList, NULL, NULL) != noErr) { return NO; }
    if (_delegate.getPlayoutData(&flags, &timestamp, 0, 4, &outputList) != noErr) { return NO; }
    return output[0] == 400 && output[1] == -300 && output[2] == 200 && output[3] == -100;
}
@end

@interface CaperSyntheticAudioDelegate : NSObject <RTCAudioDeviceDelegate>
@property(nonatomic, assign) BOOL receivedExpectedInput;
@end

@implementation CaperSyntheticAudioDelegate
- (double)preferredInputSampleRate { return 48000; }
- (NSTimeInterval)preferredInputIOBufferDuration { return 0.02; }
- (double)preferredOutputSampleRate { return 48000; }
- (NSTimeInterval)preferredOutputIOBufferDuration { return 0.02; }
- (RTCAudioDeviceDeliverRecordedDataBlock)deliverRecordedData {
    return ^OSStatus(AudioUnitRenderActionFlags *flags, const AudioTimeStamp *time, NSInteger bus,
                     UInt32 frames, const AudioBufferList *data, void *context,
                     RTCAudioDeviceRenderRecordedDataBlock render) {
        const int16_t *samples = data->mBuffers[0].mData;
        self.receivedExpectedInput = bus == 1 && frames == 4 && render == nil &&
            samples[0] == 100 && samples[1] == -200 && samples[2] == 300 && samples[3] == -400;
        return self.receivedExpectedInput ? noErr : kAudio_ParamError;
    };
}
- (RTCAudioDeviceGetPlayoutDataBlock)getPlayoutData {
    return ^OSStatus(AudioUnitRenderActionFlags *flags, const AudioTimeStamp *time, NSInteger bus,
                     UInt32 frames, AudioBufferList *data) {
        if (bus != 0 || frames != 4 || data->mBuffers[0].mDataByteSize != 8) { return kAudio_ParamError; }
        int16_t *samples = data->mBuffers[0].mData;
        for (int i = 0; i < 4; i++) { samples[i] = (int16_t)(400 - i * 100) * (i % 2 ? -1 : 1); }
        return noErr;
    };
}
- (void)notifyAudioInputParametersChange {}
- (void)notifyAudioOutputParametersChange {}
- (void)notifyAudioInputInterrupted {}
- (void)notifyAudioOutputInterrupted {}
- (void)dispatchAsync:(dispatch_block_t)block { block(); }
- (void)dispatchSync:(dispatch_block_t)block { block(); }
@end

BOOL CaperSyntheticAudioCallbacksWork(void) {
    CaperSyntheticAudioDevice *device = [CaperSyntheticAudioDevice new];
    CaperSyntheticAudioDelegate *delegate = [CaperSyntheticAudioDelegate new];
    if (![device initializeWithDelegate:delegate]) { return NO; }
    BOOL worked = [device exerciseCallbacks] && delegate.receivedExpectedInput;
    [device terminateDevice];
    return worked && !device.isInitialized;
}

BOOL CaperSyntheticVoiceDSPWorks(void) {
    CaperVoiceDSP state = {0};
    int16_t bypass[] = {100, -200, 12000, -16000};
    CaperProcessVoice(bypass, 4, 44100, 200, 0, &state);
    BOOL gainWorked = abs(bypass[0] - 200) <= 1 && abs(bypass[1] + 400) <= 1 &&
        abs(bypass[2] - 24000) <= 1 && abs(bypass[3] + 32000) <= 1;
    int16_t processed[] = {20000, -20000, 20000, -20000};
    CaperProcessVoice(processed, 4, 44100, 100, 100, &state);
    return gainWorked && (processed[0] != 20000 || processed[1] != -20000);
}
