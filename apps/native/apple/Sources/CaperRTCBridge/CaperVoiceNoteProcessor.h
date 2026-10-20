#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

/// A voice message's microphone chain outside the call device: the call's
/// bundled DPDFNet-8 worker (RNNoise when the model can't run), then the call's
/// voice contour at the saved strength, at unity input gain. Local only:
/// nothing is published. Create and release off the capture thread;
/// -capture:frameCount:stride: belongs to one capture callback and
/// -read:maxFrames: to one writer. Counters only, never samples, are exposed.
@interface CaperVoiceNoteProcessor : NSObject
/// Nil when neither denoiser can start at `sampleRate` (8...192 kHz).
- (nullable instancetype)initWithSampleRate:(double)sampleRate strength:(NSInteger)strength
    NS_SWIFT_NAME(init(sampleRate:strength:));
- (instancetype)init NS_UNAVAILABLE;
@property(nonatomic, readonly) double sampleRate;
/// 0...100, as the call's "Voice enhancement".
@property(nonatomic, readonly) NSInteger strength;
/// 1: DPDFNet-8, 2: RNNoise fallback, 0: failed (output is silence).
@property(nonatomic, readonly) NSInteger mode;
/// Frames handed to -capture since creation.
@property(nonatomic, readonly) uint64_t capturedFrames;
/// Processed frames the writer didn't collect in time.
@property(nonatomic, readonly) uint64_t droppedFrames;
/// Realtime-safe: denoises and shapes one buffer of float capture (channel 0
/// of `stride` interleaved channels, or one deinterleaved channel with
/// stride 1) and queues the processed 16-bit mono PCM for the writer.
- (void)capture:(const float *)samples frameCount:(NSUInteger)frames stride:(NSUInteger)stride
    NS_SWIFT_NAME(capture(_:frameCount:stride:));
/// Moves up to `maxFrames` processed frames into `destination`; returns how many.
- (NSUInteger)read:(int16_t *)destination maxFrames:(NSUInteger)maxFrames
    NS_SWIFT_NAME(read(into:maxFrames:));
@end

/// No microphone: synthetic speech through the processor at 44.1 and 48 kHz
/// must come back processed, non-silent and in order, with nothing dropped.
FOUNDATION_EXPORT BOOL CaperVoiceNoteProcessorWorksWithoutHardware(void);

NS_ASSUME_NONNULL_END
