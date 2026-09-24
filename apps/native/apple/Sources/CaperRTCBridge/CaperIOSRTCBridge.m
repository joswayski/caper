#import "CaperIOSRTCBridge.h"
#import "CaperDenoisePipeline.h"
#import "sdk/objc/components/audio/RTCAudioDevice.h"
#import <math.h>
#import <unistd.h>

@interface RTCPeerConnectionFactory (CaperIOSAudioFactory)
- (instancetype)initWithEncoderFactory:(nullable id<RTCVideoEncoderFactory>)encoderFactory
                         decoderFactory:(nullable id<RTCVideoDecoderFactory>)decoderFactory
                            audioDevice:(nullable id<RTCAudioDevice>)audioDevice;
@end

BOOL CaperIOSCustomAudioFactoryAvailable(void) {
    return [[RTCPeerConnectionFactory alloc] respondsToSelector:
            @selector(initWithEncoderFactory:decoderFactory:audioDevice:)];
}

RTCPeerConnectionFactory *CaperCreateIOSAudioPeerFactory(CaperIOSAudioDevice *device) {
    if (!CaperIOSCustomAudioFactoryAvailable() || ![device conformsToProtocol:@protocol(RTCAudioDevice)]) return nil;
    return [[RTCPeerConnectionFactory alloc] initWithEncoderFactory:nil decoderFactory:nil audioDevice:(id<RTCAudioDevice>)device];
}

BOOL CaperIOSNativeDenoiseWorkersRunWithoutHardware(void) {
    NSURL *model = [[NSBundle bundleForClass:CaperIOSAudioDevice.class]
        URLForResource:@"dpdfnet8_48khz_hr" withExtension:@"onnx"];
    if (!model) return NO;
    for (int which = 0; which < 2; ++which) {
        double rate = which ? 44100 : 48000;
        CaperDenoisePipeline *pipeline = CaperDenoisePipelineCreate(which ? NULL : model.fileSystemRepresentation, rate);
        if (!pipeline || CaperDenoisePipelineMode(pipeline) != (which ? 2 : 1)) {
            CaperDenoisePipelineDestroy(pipeline); return NO;
        }
        int16_t input[480], output[480];
        uint32_t epochs[480];
        unsigned frames = (unsigned)rate / 100;
        for (int hop = 0; hop < 20; ++hop) {
            for (unsigned i = 0; i < frames; ++i)
                input[i] = (int16_t)(6000 * sin(2 * M_PI * (hop * frames + i) * 180 / rate));
            if (!CaperDenoisePipelineProcess(pipeline, input, output, epochs, frames, 100, 1)) {
                CaperDenoisePipelineDestroy(pipeline); return NO;
            }
            usleep(30000);
        }
        BOOL worked = CaperDenoisePipelineStatistics(pipeline).processedHops >= 10 && !CaperDenoisePipelineFailed(pipeline);
        CaperDenoisePipelineDestroy(pipeline);
        if (!worked) return NO;
    }
    return YES;
}
