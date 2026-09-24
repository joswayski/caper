// Match the framework module name so Xcode generates its umbrella module map.
#import <Foundation/Foundation.h>
#import <WebRTC/WebRTC.h>
#import "CaperIOSAudioDevice.h"

NS_ASSUME_NONNULL_BEGIN
FOUNDATION_EXPORT BOOL CaperIOSCustomAudioFactoryAvailable(void);
FOUNDATION_EXPORT RTCPeerConnectionFactory * _Nullable CaperCreateIOSAudioPeerFactory(CaperIOSAudioDevice *device);
FOUNDATION_EXPORT BOOL CaperIOSNativeDenoiseWorkersRunWithoutHardware(void);
NS_ASSUME_NONNULL_END
