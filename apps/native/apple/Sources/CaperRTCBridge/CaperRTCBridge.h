#import <Foundation/Foundation.h>
#import <WebRTC/WebRTC.h>
#import "CaperMacAudioDevice.h"

NS_ASSUME_NONNULL_BEGIN

/// Exact M153 public protocol is vendored because stasel's macOS framework omits its header.
FOUNDATION_EXPORT BOOL CaperCustomAudioFactoryAvailable(void);
FOUNDATION_EXPORT RTCPeerConnectionFactory * _Nullable CaperCreateAudioPeerFactory(NSObject *device);
FOUNDATION_EXPORT BOOL CaperSyntheticAudioCallbacksWork(void);
FOUNDATION_EXPORT BOOL CaperSyntheticVoiceDSPWorks(void);
FOUNDATION_EXPORT BOOL CaperSyntheticComparisonStopWorks(void);
FOUNDATION_EXPORT BOOL CaperSyntheticPublicationGateWorks(void);
/// Runs the bundled native ONNX model without microphone, speaker or peer I/O.
FOUNDATION_EXPORT BOOL CaperNativeDpdfnetModelWorks(void);

/// A no-I/O probe. Never opens hardware or returns audio from a callback.
@interface CaperSyntheticAudioDevice : NSObject
@end

NS_ASSUME_NONNULL_END
