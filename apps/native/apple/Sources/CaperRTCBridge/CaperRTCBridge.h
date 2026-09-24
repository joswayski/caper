#import <Foundation/Foundation.h>
#import <WebRTC/WebRTC.h>

NS_ASSUME_NONNULL_BEGIN

/// Exact M153 public protocol is vendored because stasel's macOS framework omits its header.
FOUNDATION_EXPORT BOOL CaperCustomAudioFactoryAvailable(void);
FOUNDATION_EXPORT RTCPeerConnectionFactory * _Nullable CaperCreateAudioPeerFactory(NSObject *device);

/// A no-I/O probe. Never opens hardware or returns audio from a callback.
@interface CaperSyntheticAudioDevice : NSObject
@end

NS_ASSUME_NONNULL_END
