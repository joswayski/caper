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

/// Independent per-call CoreAudio routes. An empty UID follows the current system default.
@interface CaperMacAudioDevice : NSObject
@property(nonatomic, copy, readonly) NSString *inputUID;
@property(nonatomic, copy, readonly) NSString *outputUID;
@property(nonatomic, readonly) uint32_t resolvedOutputDeviceID;
@property(nonatomic, assign) NSInteger inputGain; // 0...200; processed before WebRTC's encoder
@property(nonatomic, assign) NSInteger processingStrength; // 0...100
/// Synchronous gate on PCM delivered to WebRTC. Local comparison retains raw and processed PCM.
@property(nonatomic, assign) BOOL publicationEnabled;
+ (NSArray<CaperAudioRoute *> *)inputRoutes;
+ (NSArray<CaperAudioRoute *> *)outputRoutes;
- (BOOL)selectInputUID:(NSString *)uid;
- (BOOL)selectOutputUID:(NSString *)uid;
- (BOOL)beginComparison;
- (nullable CaperAudioComparison *)endComparison;
@property(nonatomic, readonly) BOOL isComparing;
@end

NS_ASSUME_NONNULL_END
