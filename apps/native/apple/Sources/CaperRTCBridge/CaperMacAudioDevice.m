#import "CaperMacAudioDevice.h"
#import "CaperVoiceDSP.h"
#import "sdk/objc/components/audio/RTCAudioDevice.h"
#import <AudioUnit/AudioUnit.h>
#import <CoreAudio/CoreAudio.h>
#import <math.h>
#import <stdatomic.h>
#import <unistd.h>

enum { kCaperMaxFrames = 8192 };
static const double kCaperSampleRate = 48000;

@implementation CaperAudioRoute
- (instancetype)initWithUID:(NSString *)uid name:(NSString *)name {
    if ((self = [super init])) { _uid = [uid copy]; _name = [name copy]; }
    return self;
}
@end

@implementation CaperAudioComparison
- (instancetype)initWithNatural:(NSData *)natural enhanced:(NSData *)enhanced {
    return [self initWithNatural:natural enhanced:enhanced sampleRate:kCaperSampleRate];
}
- (instancetype)initWithNatural:(NSData *)natural enhanced:(NSData *)enhanced sampleRate:(double)sampleRate {
    if ((self = [super init])) { _natural = [natural copy]; _enhanced = [enhanced copy]; _sampleRate = sampleRate; }
    return self;
}
@end

static AudioObjectPropertyAddress CaperAddress(AudioObjectPropertySelector selector, AudioObjectPropertyScope scope) {
    return (AudioObjectPropertyAddress){selector, scope, kAudioObjectPropertyElementMain};
}

static NSArray<CaperAudioRoute *> *CaperRoutes(BOOL input) {
    AudioObjectPropertyAddress devices = CaperAddress(kAudioHardwarePropertyDevices, kAudioObjectPropertyScopeGlobal);
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &devices, 0, NULL, &size) != noErr) { return @[]; }
    AudioDeviceID *ids = malloc(size);
    if (!ids) { return @[]; }
    NSMutableArray<CaperAudioRoute *> *routes = [NSMutableArray new];
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &devices, 0, NULL, &size, ids) == noErr) {
        for (NSUInteger i = 0; i < size / sizeof(AudioDeviceID); i++) {
            AudioObjectPropertyAddress streams = CaperAddress(kAudioDevicePropertyStreamConfiguration,
                input ? kAudioObjectPropertyScopeInput : kAudioObjectPropertyScopeOutput);
            UInt32 configSize = 0;
            if (AudioObjectGetPropertyDataSize(ids[i], &streams, 0, NULL, &configSize) != noErr) { continue; }
            AudioBufferList *config = malloc(configSize);
            if (!config) { continue; }
            UInt32 channels = 0;
            if (AudioObjectGetPropertyData(ids[i], &streams, 0, NULL, &configSize, config) == noErr) {
                for (UInt32 j = 0; j < config->mNumberBuffers; j++) { channels += config->mBuffers[j].mNumberChannels; }
            }
            free(config);
            if (!channels) { continue; }
            AudioObjectPropertyAddress uidAddress = CaperAddress(kAudioDevicePropertyDeviceUID, kAudioObjectPropertyScopeGlobal);
            AudioObjectPropertyAddress nameAddress = CaperAddress(kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal);
            CFStringRef uid = NULL, name = NULL;
            UInt32 stringSize = sizeof(CFStringRef);
            OSStatus uidStatus = AudioObjectGetPropertyData(ids[i], &uidAddress, 0, NULL, &stringSize, &uid);
            stringSize = sizeof(CFStringRef);
            OSStatus nameStatus = AudioObjectGetPropertyData(ids[i], &nameAddress, 0, NULL, &stringSize, &name);
            if (uidStatus == noErr && nameStatus == noErr && uid && name) {
                [routes addObject:[[CaperAudioRoute alloc] initWithUID:(__bridge NSString *)uid name:(__bridge NSString *)name]];
            }
            if (uid) { CFRelease(uid); }
            if (name) { CFRelease(name); }
        }
    }
    free(ids);
    return routes;
}

static AudioDeviceID CaperResolveRoute(NSString *uid, BOOL input) {
    if (!uid.length) {
        AudioDeviceID device = kAudioObjectUnknown;
        AudioObjectPropertyAddress address = CaperAddress(input ? kAudioHardwarePropertyDefaultInputDevice : kAudioHardwarePropertyDefaultOutputDevice,
                                                         kAudioObjectPropertyScopeGlobal);
        UInt32 size = sizeof(device);
        return AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, &device) == noErr ? device : kAudioObjectUnknown;
    }
    AudioObjectPropertyAddress address = CaperAddress(kAudioHardwarePropertyDeviceForUID, kAudioObjectPropertyScopeGlobal);
    AudioValueTranslation translation = {0};
    AudioDeviceID device = kAudioObjectUnknown;
    CFStringRef cfUID = (__bridge CFStringRef)uid;
    translation.mInputData = &cfUID;
    translation.mInputDataSize = sizeof(cfUID);
    translation.mOutputData = &device;
    translation.mOutputDataSize = sizeof(device);
    UInt32 size = sizeof(translation);
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, &translation) != noErr) { return kAudioObjectUnknown; }
    for (CaperAudioRoute *route in CaperRoutes(input)) { if ([route.uid isEqualToString:uid]) { return device; } }
    return kAudioObjectUnknown;
}

static double CaperHardwareRate(AudioDeviceID route) {
    if (route == kAudioObjectUnknown) { return 0; }
    AudioObjectPropertyAddress address = CaperAddress(kAudioDevicePropertyNominalSampleRate, kAudioObjectPropertyScopeGlobal);
    Float64 rate = 0;
    UInt32 size = sizeof(rate);
    return AudioObjectGetPropertyData(route, &address, 0, NULL, &size, &rate) == noErr &&
           isfinite(rate) && rate >= 8000 && rate <= 192000 ? rate : 0;
}

@interface CaperMacAudioDevice () <RTCAudioDevice>
@end

static OSStatus CaperDefaultRouteChanged(AudioObjectID object, UInt32 count,
                                         const AudioObjectPropertyAddress *addresses, void *context) {
    CaperMacAudioDevice *device = (__bridge CaperMacAudioDevice *)context;
    for (UInt32 i = 0; i < count; i++) {
        if (addresses[i].mSelector == kAudioHardwarePropertyDefaultInputDevice) {
            dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
                if (!device.inputUID.length) { [device selectInputUID:@""]; }
            });
        } else if (addresses[i].mSelector == kAudioHardwarePropertyDefaultOutputDevice) {
            dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
                if (!device.outputUID.length) { [device selectOutputUID:@""]; }
            });
        }
    }
    return noErr;
}

@implementation CaperMacAudioDevice {
    id<RTCAudioDeviceDelegate> _delegate;
    AudioUnit _inputUnit;
    AudioUnit _outputUnit;
    BOOL _recording;
    BOOL _playing;
    int16_t _capture[kCaperMaxFrames];
    _Atomic int _gain;
    _Atomic int _strength;
    _Atomic bool _publicationEnabled;
    _Atomic double _inputRate;
    _Atomic double _outputRate;
    CaperVoiceDSP _processing;
    int16_t *_comparisonNatural;
    int16_t *_comparisonEnhanced;
    _Atomic unsigned _comparisonFrames;
    unsigned _comparisonCapacityFrames;
    double _comparisonSampleRate;
    _Atomic unsigned _comparisonCallbacks;
    _Atomic bool _comparing;
    BOOL _comparisonOwnsInput;
    BOOL _inputListener;
    BOOL _outputListener;
}

- (instancetype)init {
    if ((self = [super init])) {
        _inputUID = @""; _outputUID = @"";
        atomic_init(&_gain, 100); atomic_init(&_strength, 25);
        atomic_init(&_publicationEnabled, false);
        atomic_init(&_inputRate, 0); atomic_init(&_outputRate, 0);
        atomic_init(&_comparisonFrames, 0); atomic_init(&_comparisonCallbacks, 0);
        atomic_init(&_comparing, false);
    }
    return self;
}
+ (NSArray<CaperAudioRoute *> *)inputRoutes { return CaperRoutes(YES); }
+ (NSArray<CaperAudioRoute *> *)outputRoutes { return CaperRoutes(NO); }
- (uint32_t)resolvedOutputDeviceID { return CaperResolveRoute(_outputUID, NO); }
- (NSInteger)inputGain { return atomic_load(&_gain); }
- (void)setInputGain:(NSInteger)value { atomic_store(&_gain, (int)MAX(0, MIN(200, value))); }
- (NSInteger)processingStrength { return atomic_load(&_strength); }
- (void)setProcessingStrength:(NSInteger)value { atomic_store(&_strength, (int)MAX(0, MIN(100, value))); }
- (BOOL)publicationEnabled { return atomic_load_explicit(&_publicationEnabled, memory_order_acquire); }
- (void)setPublicationEnabled:(BOOL)value { atomic_store_explicit(&_publicationEnabled, value, memory_order_release); }

static void CaperGatePublishedPCM(int16_t *samples, unsigned frames, bool enabled) {
    if (!enabled) { memset(samples, 0, frames * sizeof(int16_t)); }
}

BOOL CaperSyntheticPublicationGateWorks(void) {
    int16_t samples[] = {800, -1900, 3000, -6500};
    CaperGatePublishedPCM(samples, 4, false); // joining, muted, deafened, or comparing
    for (unsigned i = 0; i < 4; i++) { if (samples[i]) { return NO; } }
    int16_t allowed[] = {800, -1900, 3000, -6500};
    CaperGatePublishedPCM(allowed, 4, true);
    return allowed[0] == 800 && allowed[1] == -1900 && allowed[2] == 3000 && allowed[3] == -6500;
}

// The second active check may observe stop after the callback registered itself.
// Return the registration separately so the callback always releases it.
static bool CaperComparisonEnter(_Atomic bool *active, _Atomic unsigned *callbacks) {
    if (!atomic_load_explicit(active, memory_order_acquire)) { return false; }
    atomic_fetch_add_explicit(callbacks, 1, memory_order_acq_rel);
    return true;
}

static void CaperComparisonLeave(_Atomic unsigned *callbacks, bool entered) {
    if (entered) { atomic_fetch_sub_explicit(callbacks, 1, memory_order_acq_rel); }
}

BOOL CaperSyntheticComparisonStopWorks(void) {
    _Atomic bool active = true;
    _Atomic unsigned callbacks = 0;
    bool entered = CaperComparisonEnter(&active, &callbacks);
    atomic_store_explicit(&active, false, memory_order_release); // stop before the callback's second check
    bool comparing = entered && atomic_load_explicit(&active, memory_order_acquire);
    CaperComparisonLeave(&callbacks, entered);
    return entered && !comparing && atomic_load_explicit(&callbacks, memory_order_acquire) == 0;
}

static OSStatus CaperInputCallback(void *context, AudioUnitRenderActionFlags *flags,
                                   const AudioTimeStamp *time, UInt32 bus, UInt32 frames, AudioBufferList *data) {
    CaperMacAudioDevice *device = (__bridge CaperMacAudioDevice *)context;
    if (frames > kCaperMaxFrames || !device->_inputUnit) { return kAudio_ParamError; }
    AudioBufferList capture = {.mNumberBuffers = 1,
        .mBuffers = {{.mNumberChannels = 1, .mDataByteSize = frames * sizeof(int16_t), .mData = device->_capture}}};
    OSStatus status = AudioUnitRender(device->_inputUnit, flags, time, 1, frames, &capture);
    if (status != noErr) { return status; }
    bool entered = CaperComparisonEnter(&device->_comparing, &device->_comparisonCallbacks);
    bool comparing = entered && atomic_load_explicit(&device->_comparing, memory_order_acquire);
    unsigned offset = 0, count = 0;
    if (comparing) {
        offset = atomic_load_explicit(&device->_comparisonFrames, memory_order_relaxed);
        count = MIN(frames, device->_comparisonCapacityFrames - MIN(offset, device->_comparisonCapacityFrames));
        if (count) { memcpy(device->_comparisonNatural + offset, device->_capture, count * sizeof(int16_t)); }
    }
    CaperProcessVoice(device->_capture, frames, atomic_load_explicit(&device->_inputRate, memory_order_relaxed),
        atomic_load_explicit(&device->_gain, memory_order_relaxed),
        atomic_load_explicit(&device->_strength, memory_order_relaxed), &device->_processing);
    if (comparing) {
        if (count) {
            memcpy(device->_comparisonEnhanced + offset, device->_capture, count * sizeof(int16_t));
            atomic_store_explicit(&device->_comparisonFrames, offset + count, memory_order_release);
        }
    }
    CaperComparisonLeave(&device->_comparisonCallbacks, entered);
    CaperGatePublishedPCM(device->_capture, frames,
        atomic_load_explicit(&device->_publicationEnabled, memory_order_acquire) && !comparing);
    return device->_recording && device->_delegate
        ? device->_delegate.deliverRecordedData(flags, time, 1, frames, &capture, NULL, NULL) : noErr;
}

static OSStatus CaperOutputCallback(void *context, AudioUnitRenderActionFlags *flags,
                                    const AudioTimeStamp *time, UInt32 bus, UInt32 frames, AudioBufferList *data) {
    CaperMacAudioDevice *device = (__bridge CaperMacAudioDevice *)context;
    if (!device->_delegate) { return kAudio_ParamError; }
    OSStatus status = device->_delegate.getPlayoutData(flags, time, bus, frames, data);
    if (status != noErr) {
        for (UInt32 i = 0; i < data->mNumberBuffers; i++) { memset(data->mBuffers[i].mData, 0, data->mBuffers[i].mDataByteSize); }
        *flags |= kAudioUnitRenderAction_OutputIsSilence;
    }
    return status;
}

- (BOOL)createUnitForInput:(BOOL)input {
    AudioDeviceID route = CaperResolveRoute(input ? _inputUID : _outputUID, input);
    if (route == kAudioObjectUnknown) { return NO; }
    AudioComponentDescription desc = {kAudioUnitType_Output, kAudioUnitSubType_HALOutput, kAudioUnitManufacturer_Apple, 0, 0};
    AudioComponent component = AudioComponentFindNext(NULL, &desc);
    AudioUnit unit = NULL;
    if (!component || AudioComponentInstanceNew(component, &unit) != noErr) { return NO; }
    UInt32 on = 1, off = 0;
    OSStatus status = AudioUnitSetProperty(unit, kAudioOutputUnitProperty_EnableIO,
                                           input ? kAudioUnitScope_Input : kAudioUnitScope_Output, input ? 1 : 0, &on, sizeof(on));
    if (status == noErr) { status = AudioUnitSetProperty(unit, kAudioOutputUnitProperty_EnableIO,
                                           input ? kAudioUnitScope_Output : kAudioUnitScope_Input, input ? 0 : 1, &off, sizeof(off)); }
    if (status == noErr) { status = AudioUnitSetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &route, sizeof(route)); }
    AudioStreamBasicDescription hardware = {0};
    UInt32 hardwareSize = sizeof(hardware);
    if (status == noErr) { status = AudioUnitGetProperty(unit, kAudioUnitProperty_StreamFormat,
                                            input ? kAudioUnitScope_Input : kAudioUnitScope_Output,
                                            input ? 1 : 0, &hardware, &hardwareSize); }
    AudioStreamBasicDescription format = {0};
    format.mSampleRate = hardware.mSampleRate;
    if (status == noErr && (!isfinite(format.mSampleRate) || format.mSampleRate < 8000 || format.mSampleRate > 192000)) {
        status = kAudio_ParamError;
    }
    format.mFormatID = kAudioFormatLinearPCM;
    format.mFormatFlags = kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked;
    format.mFramesPerPacket = 1; format.mChannelsPerFrame = 1;
    format.mBitsPerChannel = 16; format.mBytesPerFrame = 2; format.mBytesPerPacket = 2;
    if (status == noErr) { status = AudioUnitSetProperty(unit, kAudioUnitProperty_StreamFormat,
                                            input ? kAudioUnitScope_Output : kAudioUnitScope_Input,
                                            input ? 1 : 0, &format, sizeof(format)); }
    AURenderCallbackStruct callback = {input ? CaperInputCallback : CaperOutputCallback, (__bridge void *)self};
    if (status == noErr) { status = AudioUnitSetProperty(unit, input ? kAudioOutputUnitProperty_SetInputCallback : kAudioUnitProperty_SetRenderCallback,
                                            input ? kAudioUnitScope_Global : kAudioUnitScope_Input, 0, &callback, sizeof(callback)); }
    if (status == noErr) { status = AudioUnitInitialize(unit); }
    if (status != noErr) { AudioComponentInstanceDispose(unit); return NO; }
    if (input) {
        _processing = (CaperVoiceDSP){0};
        atomic_store(&_inputRate, format.mSampleRate);
        _inputUnit = unit;
    } else {
        atomic_store(&_outputRate, format.mSampleRate);
        _outputUnit = unit;
    }
    return YES;
}

- (void)disposeInput:(BOOL)input {
    AudioUnit *slot = input ? &_inputUnit : &_outputUnit;
    if (!*slot) { return; }
    AudioOutputUnitStop(*slot);
    AudioUnitUninitialize(*slot);
    AudioComponentInstanceDispose(*slot);
    *slot = NULL;
    atomic_store(input ? &_inputRate : &_outputRate, 0);
}

- (BOOL)switchInput:(BOOL)input uid:(NSString *)uid {
    if (input && self.isComparing) { return NO; }
    if (uid.length && CaperResolveRoute(uid, input) == kAudioObjectUnknown) { return NO; }
    if (!_delegate) {
        if (input) { _inputUID = [uid copy]; } else { _outputUID = [uid copy]; }
        return YES;
    }
    __block BOOL switched = NO;
    void (^change)(void) = ^{
        NSString *old = input ? self->_inputUID : self->_outputUID;
        BOOL active = input ? self->_recording : self->_playing;
        if (!active) {
            [self disposeInput:input];
            if (input) { self->_inputUID = [uid copy]; } else { self->_outputUID = [uid copy]; }
            switched = YES;
            return;
        }
        if (input) { [self->_delegate notifyAudioInputInterrupted]; }
        else { [self->_delegate notifyAudioOutputInterrupted]; }
        [self disposeInput:input];
        if (input) { self->_inputUID = [uid copy]; } else { self->_outputUID = [uid copy]; }
        switched = [self createUnitForInput:input] && (!active || AudioOutputUnitStart(input ? self->_inputUnit : self->_outputUnit) == noErr);
        if (!switched) {
            [self disposeInput:input];
            if (input) { self->_inputUID = old; self->_recording = NO; }
            else { self->_outputUID = old; self->_playing = NO; }
            if ([self createUnitForInput:input] && active && AudioOutputUnitStart(input ? self->_inputUnit : self->_outputUnit) == noErr) {
                if (input) { self->_recording = YES; } else { self->_playing = YES; }
            }
        }
        if (input) { [self->_delegate notifyAudioInputParametersChange]; }
        else { [self->_delegate notifyAudioOutputParametersChange]; }
    };
    if (_delegate) { [_delegate dispatchSync:change]; } else { change(); }
    return switched;
}
- (BOOL)selectInputUID:(NSString *)uid { return [self switchInput:YES uid:uid]; }
- (BOOL)selectOutputUID:(NSString *)uid { return [self switchInput:NO uid:uid]; }

- (BOOL)isComparing { return atomic_load(&_comparing); }
- (BOOL)beginComparison {
    if (_comparisonNatural || _comparisonEnhanced) { return NO; }
    if (![self initializeRecording]) { return NO; }
    _comparisonSampleRate = self.deviceInputSampleRate;
    if (_comparisonSampleRate < 8000 || _comparisonSampleRate > 192000) { return NO; }
    _comparisonCapacityFrames = (unsigned)(30 * _comparisonSampleRate);
    const size_t bytes = _comparisonCapacityFrames * sizeof(int16_t);
    _comparisonNatural = calloc(1, bytes);
    _comparisonEnhanced = calloc(1, bytes);
    if (!_comparisonNatural || !_comparisonEnhanced) {
        free(_comparisonNatural); free(_comparisonEnhanced);
        _comparisonNatural = NULL; _comparisonEnhanced = NULL;
        return NO;
    }
    atomic_store(&_comparisonFrames, 0);
    _comparisonOwnsInput = !_recording;
    if (_comparisonOwnsInput && AudioOutputUnitStart(_inputUnit) != noErr) {
        free(_comparisonNatural); free(_comparisonEnhanced);
        _comparisonNatural = NULL; _comparisonEnhanced = NULL; _comparisonOwnsInput = NO;
        return NO;
    }
    atomic_store_explicit(&_comparing, true, memory_order_release);
    return YES;
}
- (CaperAudioComparison *)endComparison {
    atomic_store_explicit(&_comparing, false, memory_order_release);
    while (atomic_load_explicit(&_comparisonCallbacks, memory_order_acquire)) { usleep(1000); }
    if (_comparisonOwnsInput && !_recording) { [self disposeInput:YES]; }
    _comparisonOwnsInput = NO;
    if (!_comparisonNatural || !_comparisonEnhanced) { return nil; }
    size_t bytes = atomic_load_explicit(&_comparisonFrames, memory_order_acquire) * sizeof(int16_t);
    CaperAudioComparison *result = [[CaperAudioComparison alloc]
        initWithNatural:[NSData dataWithBytes:_comparisonNatural length:bytes]
        enhanced:[NSData dataWithBytes:_comparisonEnhanced length:bytes]
        sampleRate:_comparisonSampleRate];
    free(_comparisonNatural); free(_comparisonEnhanced);
    _comparisonNatural = NULL; _comparisonEnhanced = NULL;
    return result;
}

- (double)deviceInputSampleRate {
    double rate = atomic_load(&_inputRate);
    if (rate > 0) { return rate; }
    rate = CaperHardwareRate(CaperResolveRoute(_inputUID, YES));
    return rate > 0 ? rate : kCaperSampleRate; // report a valid idle contract; opening an absent device still fails
}
- (NSTimeInterval)inputIOBufferDuration { return 0.02; }
- (NSInteger)inputNumberOfChannels { return 1; }
- (NSTimeInterval)inputLatency { return 0; }
- (double)deviceOutputSampleRate {
    double rate = atomic_load(&_outputRate);
    if (rate > 0) { return rate; }
    rate = CaperHardwareRate(CaperResolveRoute(_outputUID, NO));
    return rate > 0 ? rate : kCaperSampleRate;
}
- (NSTimeInterval)outputIOBufferDuration { return 0.02; }
- (NSInteger)outputNumberOfChannels { return 1; }
- (NSTimeInterval)outputLatency { return 0; }
- (BOOL)isInitialized { return _delegate != nil; }
- (BOOL)initializeWithDelegate:(id<RTCAudioDeviceDelegate>)delegate {
    _delegate = delegate;
    AudioObjectPropertyAddress input = CaperAddress(kAudioHardwarePropertyDefaultInputDevice, kAudioObjectPropertyScopeGlobal);
    AudioObjectPropertyAddress output = CaperAddress(kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal);
    _inputListener = AudioObjectAddPropertyListener(kAudioObjectSystemObject, &input, CaperDefaultRouteChanged, (__bridge void *)self) == noErr;
    _outputListener = AudioObjectAddPropertyListener(kAudioObjectSystemObject, &output, CaperDefaultRouteChanged, (__bridge void *)self) == noErr;
    if (_inputListener && _outputListener) { return YES; }
    [self terminateDevice];
    return NO;
}
- (BOOL)terminateDevice {
    [self endComparison];
    AudioObjectPropertyAddress input = CaperAddress(kAudioHardwarePropertyDefaultInputDevice, kAudioObjectPropertyScopeGlobal);
    AudioObjectPropertyAddress output = CaperAddress(kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal);
    if (_inputListener) { AudioObjectRemovePropertyListener(kAudioObjectSystemObject, &input, CaperDefaultRouteChanged, (__bridge void *)self); _inputListener = NO; }
    if (_outputListener) { AudioObjectRemovePropertyListener(kAudioObjectSystemObject, &output, CaperDefaultRouteChanged, (__bridge void *)self); _outputListener = NO; }
    [self disposeInput:YES]; [self disposeInput:NO];
    _recording = NO; _playing = NO; _delegate = nil;
    return YES;
}
- (BOOL)isPlayoutInitialized { return _outputUnit != NULL; }
- (BOOL)initializePlayout { return _outputUnit || [self createUnitForInput:NO]; }
- (BOOL)isPlaying { return _playing; }
- (BOOL)startPlayout {
    if (![self initializePlayout] || AudioOutputUnitStart(_outputUnit) != noErr) { return NO; }
    _playing = YES; return YES;
}
- (BOOL)stopPlayout { [self disposeInput:NO]; _playing = NO; return YES; }
- (BOOL)isRecordingInitialized { return _inputUnit != NULL; }
- (BOOL)initializeRecording { return _inputUnit || [self createUnitForInput:YES]; }
- (BOOL)isRecording { return _recording; }
- (BOOL)startRecording {
    if (![self initializeRecording] || AudioOutputUnitStart(_inputUnit) != noErr) { return NO; }
    _recording = YES; return YES;
}
- (BOOL)stopRecording {
    _recording = NO;
    if (!self.isComparing) { [self disposeInput:YES]; }
    return YES;
}
@end
