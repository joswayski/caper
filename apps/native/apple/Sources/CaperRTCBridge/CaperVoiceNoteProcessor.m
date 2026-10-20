#import "CaperVoiceNoteProcessor.h"
#import "CaperDenoisePipeline.h"
#import "CaperVoiceDSP.h"
#import <math.h>
#import <stdatomic.h>
#import <stdlib.h>
#import <unistd.h>

// One capture callback's slice; HAL and voice-processing buffers are 5-20 ms.
enum { kCaperNoteChunk = 1024 };
// About 2.7 s at 48 kHz between the capture thread and the writer.
static const uint64_t kCaperNoteRing = 1u << 17;

@implementation CaperVoiceNoteProcessor {
    CaperDenoisePipeline *_pipeline;
    CaperVoiceDSP _contour;
    int16_t _scratch[kCaperNoteChunk];
    uint32_t _epochs[kCaperNoteChunk];
    int16_t *_ring;
    _Atomic uint64_t _head, _tail, _captured, _dropped;
    _Atomic bool _failed;
}

- (instancetype)initWithSampleRate:(double)sampleRate strength:(NSInteger)strength {
    if (!isfinite(sampleRate) || sampleRate < 8000 || sampleRate > 192000) return nil;
    if (!(self = [super init])) return nil;
    _sampleRate = sampleRate;
    _strength = MAX(0, MIN(100, strength));
    _ring = calloc(kCaperNoteRing, sizeof(int16_t));
    // The same bundled model and worker a call warms; nil path means RNNoise.
    NSURL *model = [[NSBundle bundleForClass:self.class] URLForResource:@"dpdfnet8_48khz_hr" withExtension:@"onnx"];
    _pipeline = CaperDenoisePipelineCreate(model.fileSystemRepresentation, sampleRate);
    if (!_ring || !_pipeline) return nil;
    atomic_init(&_head, 0); atomic_init(&_tail, 0);
    atomic_init(&_captured, 0); atomic_init(&_dropped, 0);
    atomic_init(&_failed, false);
    return self;
}

- (void)dealloc {
    CaperDenoisePipelineDestroy(_pipeline);
    free(_ring);
}

- (NSInteger)mode { return atomic_load(&_failed) ? 0 : CaperDenoisePipelineMode(_pipeline); }
- (uint64_t)capturedFrames { return atomic_load_explicit(&_captured, memory_order_acquire); }
- (uint64_t)droppedFrames { return atomic_load_explicit(&_dropped, memory_order_relaxed); }

- (void)capture:(const float *)samples frameCount:(NSUInteger)frames stride:(NSUInteger)stride {
    if (!samples || !frames) return;
    if (!stride) stride = 1;
    NSUInteger done = 0;
    while (done < frames) {
        unsigned count = (unsigned)MIN((NSUInteger)kCaperNoteChunk, frames - done);
        for (unsigned i = 0; i < count; i++) {
            float sample = samples[(done + i) * stride];
            if (!isfinite(sample)) sample = 0;
            _scratch[i] = (int16_t)lrintf(fmaxf(-1.f, fminf(.999969f, sample)) * 32767.f);
        }
        // Publication epoch 1 throughout: one continuous private stream. The
        // worker returns post-DPDFNet PCM (silence while it primes or sheds
        // an overload), never raw capture, exactly as in a call.
        if (CaperDenoisePipelineProcess(_pipeline, _scratch, _scratch, _epochs, count, 100, 1)) {
            CaperProcessVoice(_scratch, count, _sampleRate, 100, (int)_strength, &_contour);
        } else {
            atomic_store(&_failed, true);
        }
        uint64_t head = atomic_load_explicit(&_head, memory_order_relaxed);
        uint64_t tail = atomic_load_explicit(&_tail, memory_order_acquire);
        unsigned stored = 0;
        for (; stored < count && head - tail < kCaperNoteRing; stored++, head++) {
            _ring[head & (kCaperNoteRing - 1)] = _scratch[stored];
        }
        atomic_store_explicit(&_head, head, memory_order_release);
        if (stored < count) atomic_fetch_add_explicit(&_dropped, count - stored, memory_order_relaxed);
        done += count;
    }
    atomic_fetch_add_explicit(&_captured, frames, memory_order_release);
}

- (NSUInteger)read:(int16_t *)destination maxFrames:(NSUInteger)maxFrames {
    if (!destination) return 0;
    uint64_t tail = atomic_load_explicit(&_tail, memory_order_relaxed);
    uint64_t head = atomic_load_explicit(&_head, memory_order_acquire);
    NSUInteger count = (NSUInteger)MIN((uint64_t)maxFrames, head - tail);
    for (NSUInteger i = 0; i < count; i++) destination[i] = _ring[(tail + i) & (kCaperNoteRing - 1)];
    atomic_store_explicit(&_tail, tail + count, memory_order_release);
    return count;
}
@end

BOOL CaperVoiceNoteProcessorWorksWithoutHardware(void) {
    for (int which = 0; which < 2; which++) {
        double rate = which ? 44100 : 48000;
        CaperVoiceNoteProcessor *processor = [[CaperVoiceNoteProcessor alloc] initWithSampleRate:rate strength:25];
        if (!processor || processor.mode == 0) return NO;
        unsigned frames = (unsigned)rate / 100;
        float input[480];
        int16_t output[4096];
        uint64_t read = 0;
        // Feed 10 ms of a loud 180 Hz tone every 10 ms for half a second.
        for (unsigned hop = 0; hop < 50; hop++) {
            for (unsigned i = 0; i < frames; i++)
                input[i] = (float)(0.5 * sin(2 * M_PI * (hop * frames + i) * 180 / rate));
            [processor capture:input frameCount:frames stride:1];
            NSUInteger count;
            while ((count = [processor read:output maxFrames:4096])) {
                // The worker primes on three hops: the first block is silence,
                // never raw capture.
                if (hop == 0) for (NSUInteger i = 0; i < count; i++) if (output[i]) return NO;
                read += count;
            }
            usleep(10000);
        }
        // Every captured frame reaches the writer, in order and none dropped.
        if (read != processor.capturedFrames || read != 50 * frames || processor.droppedFrames) return NO;
        if (processor.mode == 0) return NO;
    }
    return YES;
}
