#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface CaperIOSAudioComparison : NSObject
@property(nonatomic, copy, readonly) NSData *natural;
@property(nonatomic, copy, readonly) NSData *enhanced;
@property(nonatomic, readonly) double sampleRate;
- (instancetype)initWithNatural:(NSData *)natural enhanced:(NSData *)enhanced sampleRate:(double)rate;
@end

@interface CaperIOSAudioProcessingReport : NSObject
@property(nonatomic, readonly) NSInteger mode;
@property(nonatomic, readonly) uint64_t processedHops;
@property(nonatomic, readonly) double meanProcessingMs;
@property(nonatomic, readonly) double maxProcessingMs;
@property(nonatomic, readonly) double queuedInputMs;
@end

/// Uses the current AVAudioSession route, never sets system default devices.
@interface CaperIOSAudioDevice : NSObject
@property(nonatomic) NSInteger inputGain;
@property(nonatomic) NSInteger processingStrength;
@property(nonatomic) BOOL publicationEnabled;
@property(nonatomic, readonly) NSInteger denoiseMode;
@property(nonatomic, readonly) BOOL denoiseFailed;
@property(nonatomic, readonly, nullable) CaperIOSAudioProcessingReport *audioProcessingReport;
- (BOOL)prepareDenoise;
- (BOOL)beginComparison;
/// RMS (0...1) of the latest natural comparison chunk, for the local input meter.
@property(nonatomic, readonly) float comparisonLevel;
- (nullable CaperIOSAudioComparison *)endComparison;
- (void)audioRouteInterrupted;
/// No hardware. Uses the same capture/ADM gate and worker with supplied PCM.
+ (instancetype)syntheticTestDevice;
@property(nonatomic, readonly) BOOL syntheticRecordingActive;
@property(nonatomic, readonly) BOOL syntheticPlayoutActive;
@property(nonatomic, readonly) NSInteger syntheticLastPublishedPeak;
- (BOOL)injectSyntheticPCM:(NSData *)pcm;
- (nullable NSData *)pullSyntheticPlayoutFrames:(uint32_t)frames;
@end

NS_ASSUME_NONNULL_END
