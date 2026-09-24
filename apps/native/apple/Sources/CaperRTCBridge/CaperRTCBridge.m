#import "CaperRTCBridge.h"
#import "CaperDpdfnet.h"
#import "CaperDenoisePipeline.h"
#import "CaperVoiceDSP.h"
#import "sdk/objc/components/audio/RTCAudioDevice.h"
#import <math.h>
#import <stdlib.h>
#import <unistd.h>

BOOL CaperNativeDpdfnetModelWorks(void) {
    NSURL *model = [[NSBundle bundleForClass:CaperMacAudioDevice.class]
        URLForResource:@"dpdfnet8_48khz_hr" withExtension:@"onnx"];
    CaperDpdfnet *engine = CaperDpdfnetCreate(model.fileSystemRepresentation);
    if (!engine) { return NO; }
    float input[480] = {0}, output[480] = {0};
    BOOL worked = CaperDpdfnetProcess(engine, input, output) &&
                  CaperDpdfnetProcess(engine, input, output);
    CaperDpdfnetDestroy(engine);
    return worked;
}

BOOL CaperNativeDenoiseWorkersRunWithoutHardware(void) {
    NSURL *model = [[NSBundle bundleForClass:CaperMacAudioDevice.class]
        URLForResource:@"dpdfnet8_48khz_hr" withExtension:@"onnx"];
    for (int engine = 1; engine <= 2; engine++) {
        double rate = engine == 1 ? 44100 : 48000;
        CaperDenoisePipeline *pipeline = CaperDenoisePipelineCreate(engine == 1 ? model.fileSystemRepresentation : NULL, rate);
        if (!pipeline || CaperDenoisePipelineMode(pipeline) != engine) {
            CaperDenoisePipelineDestroy(pipeline); return NO;
        }
        int16_t input[480], output[480];
        uint32_t epochs[480];
        unsigned frames = (unsigned)rate / 100;
        for (unsigned hop = 0; hop < 20; hop++) {
            for (unsigned i = 0; i < frames; i++) input[i] = (int16_t)(6000 * sin(2 * M_PI * (hop * frames + i) * 180 / rate));
            if (!CaperDenoisePipelineProcess(pipeline, input, output, epochs, frames, 100, 1)) {
                CaperDenoisePipelineDestroy(pipeline); return NO;
            }
            usleep(30000); // Runtime correctness only; sustained 10 ms timing needs physical acceptance.
        }
        CaperDenoiseStatistics stats = CaperDenoisePipelineStatistics(pipeline);
        BOOL worked = stats.mode == engine && stats.processedHops >= 10 &&
            stats.totalProcessingMicros > 0 && stats.maxProcessingMicros > 0;
        CaperDenoisePipelineDestroy(pipeline);
        if (!worked) return NO;
    }
    return YES;
}

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

BOOL CaperSyntheticContourEpochWorks(void) {
    CaperVoiceDSP state = {0};
    uint32_t lastEpoch = 0;
    int16_t mixed[960] = {22000};
    uint32_t epochs[960] = {0};
    for (unsigned i = 480; i < 960; i++) epochs[i] = 7;
    CaperProcessVoiceEpochs(mixed, epochs, 960, 48000, 100, 100, &state, &lastEpoch);
    if (!mixed[0] || lastEpoch != 7) return NO;
    for (unsigned i = 480; i < 960; i++) if (mixed[i]) return NO;

    // Also cross the boundary between callbacks, where live contour state is
    // retained. The public silence must match a freshly initialized filter.
    int16_t privateImpulse[480] = {22000}, publicSilence[480] = {0};
    uint32_t privateEpoch[480] = {0}, publicEpoch[480];
    for (unsigned i = 0; i < 480; i++) publicEpoch[i] = 9;
    CaperProcessVoiceEpochs(privateImpulse, privateEpoch, 480, 48000, 100, 100, &state, &lastEpoch);
    CaperProcessVoiceEpochs(publicSilence, publicEpoch, 480, 48000, 100, 100, &state, &lastEpoch);
    for (unsigned i = 0; i < 480; i++) if (publicSilence[i]) return NO;
    return lastEpoch == 9;
}
