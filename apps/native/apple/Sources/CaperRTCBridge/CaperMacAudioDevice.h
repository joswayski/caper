#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface CaperAudioRoute : NSObject
@property(nonatomic, copy, readonly) NSString *uid;
@property(nonatomic, copy, readonly) NSString *name;
- (instancetype)initWithUID:(NSString *)uid name:(NSString *)name;
@end

@interface CaperAudioComparison : NSObject
@property(nonatomic, copy, readonly) NSData *natural;
@property(nonatomic, copy, readonly) NSData *enhanced;
@property(nonatomic, readonly) double sampleRate;
- (instancetype)initWithNatural:(NSData *)natural enhanced:(NSData *)enhanced;
- (instancetype)initWithNatural:(NSData *)natural enhanced:(NSData *)enhanced sampleRate:(double)sampleRate;
@end

/// Local numeric counters only; no device IDs, routes, SDP or audio samples.
@interface CaperAudioProcessingReport : NSObject
@property(nonatomic, readonly) NSInteger mode;
@property(nonatomic, readonly) uint64_t processedHops;
@property(nonatomic, readonly) double meanProcessingMs;
@property(nonatomic, readonly) double maxProcessingMs;
@property(nonatomic, readonly) double queuedInputMs;
@end

/// Independent per-call CoreAudio routes. An empty UID follows the current system default.
@interface CaperMacAudioDevice : NSObject
@property(nonatomic, copy, readonly) NSString *inputUID;
@property(nonatomic, copy, readonly) NSString *outputUID;
@property(nonatomic, readonly) uint32_t resolvedOutputDeviceID;
@property(nonatomic, assign) NSInteger inputGain; // 0...200; processed before WebRTC's encoder
@property(nonatomic, assign) NSInteger processingStrength; // 0...100
/// Synchronous gate on PCM delivered to WebRTC. Local comparison retains
/// denoised natural PCM and the same stream with the live contour applied.
@property(nonatomic, assign) BOOL publicationEnabled;
+ (NSArray<CaperAudioRoute *> *)inputRoutes;
+ (NSArray<CaperAudioRoute *> *)outputRoutes;
- (BOOL)selectInputUID:(NSString *)uid;
- (BOOL)selectOutputUID:(NSString *)uid;
- (BOOL)beginComparison;
- (nullable CaperAudioComparison *)endComparison;
@property(nonatomic, readonly) BOOL isComparing;
/// RMS (0...1) of the latest natural comparison chunk, for the local input meter.
@property(nonatomic, readonly) float comparisonLevel;
/// Warm the bundled DPDFNet model before starting capture. Never call on HAL.
- (BOOL)prepareDenoise;
@property(nonatomic, readonly) BOOL denoiseFailed;
@property(nonatomic, readonly) NSInteger denoiseMode;
@property(nonatomic, readonly, nullable) CaperAudioProcessingReport *audioProcessingReport;
/// Test-only synthetic I/O through the production capture/ADM callback; never opens CoreAudio.
+ (instancetype)syntheticTestDevice;
@property(nonatomic, readonly) BOOL syntheticRecordingActive;
@property(nonatomic, readonly) BOOL syntheticPlayoutActive;
@property(nonatomic, readonly) NSInteger syntheticLastPublishedPeak;
- (BOOL)injectSyntheticPCM:(NSData *)pcm;
- (nullable NSData *)pullSyntheticPlayoutFrames:(uint32_t)frames;
@end

NS_ASSUME_NONNULL_END
