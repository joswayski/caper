#import "CaperIOSAudioDevice.h"
#import "CaperDenoisePipeline.h"
#import "CaperVoiceDSP.h"
#import "sdk/objc/components/audio/RTCAudioDevice.h"
#import <AVFAudio/AVFAudio.h>
#import <AudioUnit/AudioUnit.h>
#import <stdatomic.h>
#import <unistd.h>
#import <math.h>

enum { kCaperIOSMaxFrames = 8192 };
static const unsigned kCaperIOSComparisonActive = 1u << 31;

@implementation CaperIOSAudioComparison
- (instancetype)initWithNatural:(NSData *)natural enhanced:(NSData *)enhanced sampleRate:(double)rate {
    if ((self = [super init])) { _natural = [natural copy]; _enhanced = [enhanced copy]; _sampleRate = rate; }
    return self;
}
@end

@implementation CaperIOSAudioProcessingReport
- (instancetype)initWithStatistics:(CaperDenoiseStatistics)stats rate:(double)rate {
    if ((self = [super init])) {
        _mode = stats.mode; _processedHops = stats.processedHops;
        _meanProcessingMs = stats.processedHops ? (double)stats.totalProcessingMicros / stats.processedHops / 1000 : 0;
        _maxProcessingMs = (double)stats.maxProcessingMicros / 1000;
        _queuedInputMs = rate ? (double)stats.queuedInputFrames / rate * 1000 : 0;
    }
    return self;
}
@end

@interface CaperIOSAudioDevice () <RTCAudioDevice>
@end

@implementation CaperIOSAudioDevice {
    id<RTCAudioDeviceDelegate> _delegate;
    AudioUnit _unit;
    _Atomic bool _recording, _playing, _publicationEnabled;
    _Atomic unsigned _comparisonState, _comparisonFrames;
    _Atomic uint32_t _publicationEpoch;
    _Atomic int _gain, _strength, _syntheticPeak;
    _Atomic double _sampleRate;
    _Atomic float _comparisonLevel;
    BOOL _initializedInput, _initializedOutput, _synthetic, _comparisonOwnsUnit, _terminating;
    uint64_t _lifecycle;
    CaperDenoisePipeline *_denoiser;
    double _denoiserRate;
    CaperVoiceDSP _contour;
    uint32_t _contourEpoch;
    int16_t _capture[kCaperIOSMaxFrames];
    uint32_t _captureEpochs[kCaperIOSMaxFrames];
    int16_t *_natural, *_enhanced;
    unsigned _comparisonCapacity;
}

- (instancetype)init {
    if ((self = [super init])) {
        atomic_init(&_gain, 100); atomic_init(&_strength, 25);
        atomic_init(&_publicationEpoch, 1); atomic_init(&_publicationEnabled, false);
        atomic_init(&_recording, false); atomic_init(&_playing, false);
        atomic_init(&_comparisonState, 0);
        atomic_init(&_comparisonFrames, 0); atomic_init(&_sampleRate, 0);
        atomic_init(&_syntheticPeak, 0);
        atomic_init(&_comparisonLevel, 0);
    }
    return self;
}
+ (instancetype)syntheticTestDevice { CaperIOSAudioDevice *device = [self new]; device->_synthetic = YES; return device; }
- (NSInteger)inputGain { return atomic_load(&_gain); }
- (void)setInputGain:(NSInteger)value {
    int next = (int)MAX(0, MIN(200, value));
    int previous = atomic_load(&_gain);
    if (previous && !next) {
        atomic_store(&_gain, 0);
        atomic_fetch_add(&_publicationEpoch, 1);
    } else {
        if (!previous && next) atomic_fetch_add(&_publicationEpoch, 1);
        atomic_store(&_gain, next);
    }
}
- (NSInteger)processingStrength { return atomic_load(&_strength); }
- (void)setProcessingStrength:(NSInteger)value { atomic_store(&_strength, (int)MAX(0, MIN(100, value))); }
- (BOOL)publicationEnabled { return atomic_load(&_publicationEnabled); }
- (void)setPublicationEnabled:(BOOL)value {
    if (atomic_load(&_publicationEnabled) != value) {
        if (!value) {
            atomic_store(&_publicationEnabled, false);
            atomic_fetch_add(&_publicationEpoch, 1);
        } else {
            atomic_fetch_add(&_publicationEpoch, 1);
            atomic_store(&_publicationEnabled, true);
        }
    }
}
- (NSInteger)denoiseMode { @synchronized (self) { return _synthetic ? 0 : CaperDenoisePipelineMode(_denoiser); } }
- (BOOL)denoiseFailed { @synchronized (self) { return !_synthetic && CaperDenoisePipelineFailed(_denoiser); } }
- (CaperIOSAudioProcessingReport *)audioProcessingReport {
    @synchronized (self) {
        return _synthetic || !_denoiser ? nil : [[CaperIOSAudioProcessingReport alloc]
            initWithStatistics:CaperDenoisePipelineStatistics(_denoiser) rate:_denoiserRate];
    }
}
- (double)currentRate {
    if (_synthetic) return 48000;
    double rate = [AVAudioSession sharedInstance].sampleRate;
    return rate >= 8000 && rate <= 192000 ? rate : 48000;
}
- (BOOL)prepareDenoise {
    if (_synthetic) return YES;
    @synchronized (self) {
        if (_unit || _recording || (atomic_load(&_comparisonState) & kCaperIOSComparisonActive))
            return _denoiser && !CaperDenoisePipelineFailed(_denoiser);
        double rate = self.currentRate;
        NSURL *model = [[NSBundle bundleForClass:self.class] URLForResource:@"dpdfnet8_48khz_hr" withExtension:@"onnx"];
        CaperDenoisePipeline *prepared = CaperDenoisePipelineCreate(model.fileSystemRepresentation, rate);
        if (!prepared) return NO;
        CaperDenoisePipelineDestroy(_denoiser);
        _denoiser = prepared; _denoiserRate = rate;
        return YES;
    }
}
- (BOOL)isInitialized { return _delegate != nil; }
- (BOOL)initializeWithDelegate:(id<RTCAudioDeviceDelegate>)delegate {
    @synchronized (self) { _lifecycle++; _terminating = NO; _delegate = delegate; }
    return YES;
}
- (void)stopUnitIfIdle {
    if (_unit && !_recording && !_playing && !(atomic_load(&_comparisonState) & kCaperIOSComparisonActive)) {
        AudioOutputUnitStop(_unit); AudioUnitUninitialize(_unit); AudioComponentInstanceDispose(_unit);
        _unit = NULL; _initializedInput = NO; _initializedOutput = NO;
    }
}
- (BOOL)terminateDevice {
    @synchronized (self) { _lifecycle++; _terminating = YES; }
    self.publicationEnabled = NO;
    atomic_fetch_and(&_comparisonState, ~kCaperIOSComparisonActive);
    if (_unit) { AudioOutputUnitStop(_unit); AudioUnitUninitialize(_unit); AudioComponentInstanceDispose(_unit); _unit = NULL; }
    while (atomic_load(&_comparisonState)) usleep(1000);
    free(_natural); free(_enhanced); _natural = NULL; _enhanced = NULL;
    atomic_store(&_recording, false); atomic_store(&_playing, false);
    _initializedInput = NO; _initializedOutput = NO;
    @synchronized (self) { CaperDenoisePipelineDestroy(_denoiser); _denoiser = NULL; _delegate = nil; }
    return YES;
}
- (void)dealloc { [self terminateDevice]; }

static uint32_t CaperIOSEpoch(CaperIOSAudioDevice *device) {
    uint32_t before = atomic_load(&device->_publicationEpoch);
    BOOL allowed = atomic_load(&device->_publicationEnabled) &&
        !(atomic_load(&device->_comparisonState) & kCaperIOSComparisonActive) && atomic_load(&device->_gain) != 0;
    return allowed && before == atomic_load(&device->_publicationEpoch) ? before : 0;
}

static OSStatus CaperIOSDeliver(CaperIOSAudioDevice *device, AudioUnitRenderActionFlags *flags,
                                const AudioTimeStamp *time, UInt32 frames, uint32_t entryEpoch) {
    unsigned state = atomic_load(&device->_comparisonState);
    BOOL entered = NO;
    while (state & kCaperIOSComparisonActive) {
        if (atomic_compare_exchange_weak(&device->_comparisonState, &state, state + 1)) { entered = YES; break; }
    }
    BOOL comparing = entered && (atomic_load(&device->_comparisonState) & kCaperIOSComparisonActive);
    uint32_t epoch = entryEpoch == CaperIOSEpoch(device) ? entryEpoch : 0;
    int gain = atomic_load(&device->_gain);
    BOOL processed = YES;
    if (!device->_synthetic) {
        processed = CaperDenoisePipelineProcess(device->_denoiser, device->_capture,
            device->_capture, device->_captureEpochs, frames, gain, epoch);
    } else {
        for (UInt32 i = 0; i < frames; ++i) device->_captureEpochs[i] = epoch;
    }
    if (!gain) {
        memset(device->_capture, 0, frames * sizeof(int16_t));
        memset(device->_captureEpochs, 0, frames * sizeof(uint32_t));
        device->_contour = (CaperVoiceDSP){0}; device->_contourEpoch = 0;
    }
    unsigned offset = 0, count = 0;
    if (comparing) {
        offset = atomic_load(&device->_comparisonFrames);
        count = MIN(frames, device->_comparisonCapacity - MIN(offset, device->_comparisonCapacity));
        if (count) {
            memcpy(device->_natural + offset, device->_capture, count * sizeof(int16_t));
            double sum = 0;
            for (unsigned i = 0; i < count; ++i) { double sample = device->_capture[i] / 32768.0; sum += sample * sample; }
            atomic_store(&device->_comparisonLevel, (float)sqrt(sum / count));
        }
    }
    if (gain && processed) {
        CaperProcessVoiceEpochs(device->_capture, device->_captureEpochs, frames, atomic_load(&device->_sampleRate),
            device->_synthetic ? gain : 100, atomic_load(&device->_strength), &device->_contour, &device->_contourEpoch);
    }
    if (comparing && count) {
        memcpy(device->_enhanced + offset, device->_capture, count * sizeof(int16_t));
        atomic_store(&device->_comparisonFrames, offset + count);
    }
    if (entered) atomic_fetch_sub(&device->_comparisonState, 1);
    uint32_t current = atomic_load(&device->_publicationEpoch);
    for (UInt32 i = 0; i < frames; ++i) {
        if (device->_captureEpochs[i] != current || !device->_captureEpochs[i] ||
            !atomic_load(&device->_publicationEnabled) || !atomic_load(&device->_gain) || comparing)
            device->_capture[i] = 0;
    }
    if (device->_synthetic) {
        int peak = 0;
        for (UInt32 i = 0; i < frames; ++i) peak = MAX(peak, abs((int)device->_capture[i]));
        atomic_store(&device->_syntheticPeak, peak);
    }
    AudioBufferList capture = {.mNumberBuffers = 1,
        .mBuffers = {{.mNumberChannels = 1, .mDataByteSize = frames * sizeof(int16_t), .mData = device->_capture}}};
    return device->_recording && device->_delegate
        ? device->_delegate.deliverRecordedData(flags, time, 1, frames, &capture, NULL, NULL) : noErr;
}

static OSStatus CaperIOSInput(void *context, AudioUnitRenderActionFlags *flags,
                              const AudioTimeStamp *time, UInt32 bus, UInt32 frames, AudioBufferList *data) {
    CaperIOSAudioDevice *device = (__bridge CaperIOSAudioDevice *)context;
    if (frames > kCaperIOSMaxFrames || !device->_unit) return kAudio_ParamError;
    uint32_t entryEpoch = CaperIOSEpoch(device);
    AudioBufferList capture = {.mNumberBuffers = 1,
        .mBuffers = {{.mNumberChannels = 1, .mDataByteSize = frames * sizeof(int16_t), .mData = device->_capture}}};
    OSStatus status = AudioUnitRender(device->_unit, flags, time, 1, frames, &capture);
    return status == noErr ? CaperIOSDeliver(device, flags, time, frames, entryEpoch) : status;
}

static OSStatus CaperIOSOutput(void *context, AudioUnitRenderActionFlags *flags,
                               const AudioTimeStamp *time, UInt32 bus, UInt32 frames, AudioBufferList *data) {
    CaperIOSAudioDevice *device = (__bridge CaperIOSAudioDevice *)context;
    if (!device->_delegate || !device->_playing) {
        for (UInt32 i = 0; i < data->mNumberBuffers; ++i) memset(data->mBuffers[i].mData, 0, data->mBuffers[i].mDataByteSize);
        *flags |= kAudioUnitRenderAction_OutputIsSilence;
        return noErr;
    }
    OSStatus result = device->_delegate.getPlayoutData(flags, time, bus, frames, data);
    if (result != noErr) {
        for (UInt32 i = 0; i < data->mNumberBuffers; ++i) memset(data->mBuffers[i].mData, 0, data->mBuffers[i].mDataByteSize);
        *flags |= kAudioUnitRenderAction_OutputIsSilence;
    }
    return result;
}

- (BOOL)createUnit {
    if (_synthetic) { atomic_store(&_sampleRate, 48000); return YES; }
    if (_unit) return YES;
    if (!_denoiser || CaperDenoisePipelineFailed(_denoiser) || _denoiserRate != self.currentRate) return NO;
    AudioComponentDescription desc = {kAudioUnitType_Output, kAudioUnitSubType_VoiceProcessingIO, kAudioUnitManufacturer_Apple, 0, 0};
    AudioComponent component = AudioComponentFindNext(NULL, &desc);
    if (!component || AudioComponentInstanceNew(component, &_unit) != noErr) return NO;
    UInt32 on = 1;
    AudioStreamBasicDescription format = {0};
    format.mSampleRate = _denoiserRate; format.mFormatID = kAudioFormatLinearPCM;
    format.mFormatFlags = kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked;
    format.mBytesPerPacket = 2; format.mFramesPerPacket = 1;
    format.mBytesPerFrame = 2; format.mChannelsPerFrame = 1; format.mBitsPerChannel = 16;
    AURenderCallbackStruct input = {CaperIOSInput, (__bridge void *)self};
    AURenderCallbackStruct output = {CaperIOSOutput, (__bridge void *)self};
    OSStatus status = AudioUnitSetProperty(_unit, kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Input, 1, &on, sizeof(on));
    if (status == noErr) status = AudioUnitSetProperty(_unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output, 1, &format, sizeof(format));
    if (status == noErr) status = AudioUnitSetProperty(_unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 0, &format, sizeof(format));
    if (status == noErr) status = AudioUnitSetProperty(_unit, kAudioOutputUnitProperty_SetInputCallback, kAudioUnitScope_Global, 1, &input, sizeof(input));
    if (status == noErr) status = AudioUnitSetProperty(_unit, kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0, &output, sizeof(output));
    if (status == noErr) status = AudioUnitInitialize(_unit);
    if (status != noErr) { AudioComponentInstanceDispose(_unit); _unit = NULL; return NO; }
    atomic_store(&_sampleRate, _denoiserRate);
    _contour = (CaperVoiceDSP){0}; _contourEpoch = 0;
    return YES;
}
- (double)deviceInputSampleRate { double rate = atomic_load(&_sampleRate); return rate ? rate : self.currentRate; }
- (double)deviceOutputSampleRate { return self.deviceInputSampleRate; }
- (NSTimeInterval)inputIOBufferDuration { return _synthetic ? .01 : [AVAudioSession sharedInstance].IOBufferDuration; }
- (NSTimeInterval)outputIOBufferDuration { return self.inputIOBufferDuration; }
- (NSInteger)inputNumberOfChannels { return 1; }
- (NSInteger)outputNumberOfChannels { return 1; }
- (NSTimeInterval)inputLatency { return 0; }
- (NSTimeInterval)outputLatency { return 0; }
- (BOOL)isRecordingInitialized { return _initializedInput; }
- (BOOL)initializeRecording { if (![self createUnit]) return NO; _initializedInput = YES; return YES; }
- (BOOL)isPlayoutInitialized { return _initializedOutput; }
- (BOOL)initializePlayout { if (![self createUnit]) return NO; _initializedOutput = YES; return YES; }
- (BOOL)isRecording { return atomic_load(&_recording); }
- (BOOL)isPlaying { return atomic_load(&_playing); }
- (BOOL)startRecording {
    if (![self initializeRecording]) return NO;
    if (!_synthetic && !_playing && !(atomic_load(&_comparisonState) & kCaperIOSComparisonActive) && AudioOutputUnitStart(_unit) != noErr) return NO;
    atomic_store(&_recording, true); return YES;
}
- (BOOL)startPlayout {
    if (![self initializePlayout]) return NO;
    if (!_synthetic && !_recording && !(atomic_load(&_comparisonState) & kCaperIOSComparisonActive) && AudioOutputUnitStart(_unit) != noErr) return NO;
    atomic_store(&_playing, true); return YES;
}
- (BOOL)stopRecording { atomic_store(&_recording, false); [self stopUnitIfIdle]; return YES; }
- (BOOL)stopPlayout { atomic_store(&_playing, false); [self stopUnitIfIdle]; return YES; }

- (float)comparisonLevel { return atomic_load(&_comparisonLevel); }

- (BOOL)beginComparison {
    id<RTCAudioDeviceDelegate> delegate;
    uint64_t lifecycle;
    @synchronized (self) { if (_terminating) return NO; delegate = _delegate; lifecycle = _lifecycle; }
    __block BOOL started = NO;
    void (^start)(void) = ^{
        if (self->_delegate != delegate || self->_lifecycle != lifecycle || self->_natural) return;
        if (![self initializeRecording]) return;
        unsigned capacity = (unsigned)(30 * self.deviceInputSampleRate);
        if (capacity > 30 * 192000 || capacity < 30 * 8000) return;
        self->_natural = calloc(capacity, sizeof(int16_t)); self->_enhanced = calloc(capacity, sizeof(int16_t));
        if (!self->_natural || !self->_enhanced) {
            free(self->_natural); free(self->_enhanced); self->_natural = NULL; self->_enhanced = NULL; return;
        }
        self->_comparisonCapacity = capacity; atomic_store(&self->_comparisonFrames, 0);
        self->_comparisonOwnsUnit = !self->_recording && !self->_playing;
        if (self->_comparisonOwnsUnit && !self->_synthetic && AudioOutputUnitStart(self->_unit) != noErr) {
            free(self->_natural); free(self->_enhanced); self->_natural = NULL; self->_enhanced = NULL; return;
        }
        atomic_fetch_add(&self->_publicationEpoch, 1);
        atomic_store(&self->_comparisonState, kCaperIOSComparisonActive);
        started = YES;
    };
    if (delegate) [delegate dispatchSync:start]; else start();
    return started;
}
- (CaperIOSAudioComparison *)endComparison {
    id<RTCAudioDeviceDelegate> delegate;
    uint64_t lifecycle;
    @synchronized (self) { delegate = _delegate; lifecycle = _lifecycle; }
    __block CaperIOSAudioComparison *result = nil;
    void (^finish)(void) = ^{
        if (self->_delegate != delegate || self->_lifecycle != lifecycle) return;
        atomic_fetch_add(&self->_publicationEpoch, 1);
        atomic_fetch_and(&self->_comparisonState, ~kCaperIOSComparisonActive);
        while (atomic_load(&self->_comparisonState)) usleep(1000);
        if (self->_natural && self->_enhanced) {
            size_t bytes = atomic_load(&self->_comparisonFrames) * sizeof(int16_t);
            result = [[CaperIOSAudioComparison alloc]
                initWithNatural:[NSData dataWithBytes:self->_natural length:bytes]
                enhanced:[NSData dataWithBytes:self->_enhanced length:bytes] sampleRate:self.deviceInputSampleRate];
        }
        free(self->_natural); free(self->_enhanced); self->_natural = NULL; self->_enhanced = NULL;
        if (self->_comparisonOwnsUnit) {
            self->_comparisonOwnsUnit = NO; [self stopUnitIfIdle];
        }
    };
    if (delegate) [delegate dispatchSync:finish]; else finish();
    return result;
}
- (void)audioRouteInterrupted {
    self.publicationEnabled = NO;
    [self endComparison];
    id<RTCAudioDeviceDelegate> delegate;
    @synchronized (self) { delegate = _delegate; }
    void (^reset)(void) = ^{
        if (self->_unit) {
            AudioOutputUnitStop(self->_unit); AudioUnitUninitialize(self->_unit);
            AudioComponentInstanceDispose(self->_unit); self->_unit = NULL;
        }
        atomic_store(&self->_recording, false); atomic_store(&self->_playing, false);
        self->_initializedInput = NO; self->_initializedOutput = NO;
        @synchronized (self) { CaperDenoisePipelineDestroy(self->_denoiser); self->_denoiser = NULL; self->_denoiserRate = 0; }
    };
    if (delegate) [delegate dispatchSync:reset]; else reset();
}
- (BOOL)syntheticRecordingActive { return _synthetic && _recording && _delegate != nil; }
- (BOOL)syntheticPlayoutActive { return _synthetic && _playing && _delegate != nil; }
- (NSInteger)syntheticLastPublishedPeak { return _synthetic ? atomic_load(&_syntheticPeak) : 0; }
- (BOOL)injectSyntheticPCM:(NSData *)pcm {
    if (!self.syntheticRecordingActive || !pcm.length || pcm.length > sizeof(_capture) || pcm.length % 2) return NO;
    uint32_t epoch = CaperIOSEpoch(self);
    memcpy(_capture, pcm.bytes, pcm.length);
    AudioUnitRenderActionFlags flags = 0; AudioTimeStamp time = {0};
    return CaperIOSDeliver(self, &flags, &time, (UInt32)(pcm.length / 2), epoch) == noErr;
}
- (NSData *)pullSyntheticPlayoutFrames:(uint32_t)frames {
    if (!self.syntheticPlayoutActive || frames > kCaperIOSMaxFrames) return nil;
    int16_t samples[kCaperIOSMaxFrames] = {0};
    AudioBufferList data = {.mNumberBuffers = 1,
        .mBuffers = {{.mNumberChannels = 1, .mDataByteSize = frames * sizeof(int16_t), .mData = samples}}};
    AudioUnitRenderActionFlags flags = 0; AudioTimeStamp time = {0};
    return CaperIOSOutput((__bridge void *)self, &flags, &time, 0, frames, &data) == noErr
        ? [NSData dataWithBytes:samples length:frames * sizeof(int16_t)] : nil;
}
@end
