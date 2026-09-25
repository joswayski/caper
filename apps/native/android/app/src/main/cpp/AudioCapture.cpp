#include <jni.h>
#include <algorithm>
#include <array>
#include <cstdint>
#include <cstring>
#include "CaperDenoisePipeline.h"
#include "CaperVoiceDSP.h"

struct Capture {
    CaperDenoisePipeline *pipeline;
    CaperVoiceDSP contour{};
    uint32_t contourEpoch = 0;
    int rate;
};

extern "C" JNIEXPORT jlong JNICALL
Java_chat_caper_android_voice_AudioCapture_create(JNIEnv *env, jobject, jstring path, jint rate) {
    const char *name = env->GetStringUTFChars(path, nullptr);
    auto *pipeline = CaperDenoisePipelineCreate(name, rate);
    env->ReleaseStringUTFChars(path, name);
    return pipeline ? reinterpret_cast<jlong>(new Capture{pipeline, {}, 0, rate}) : 0;
}

extern "C" JNIEXPORT void JNICALL
Java_chat_caper_android_voice_AudioCapture_destroy(JNIEnv *, jobject, jlong handle) {
    auto *capture = reinterpret_cast<Capture *>(handle);
    if (capture) { CaperDenoisePipelineDestroy(capture->pipeline); delete capture; }
}

// Returns zero on overload/error. The Java capture callback always clears the
// input buffer itself if the native result or publication epoch is invalid.
extern "C" JNIEXPORT jint JNICALL
Java_chat_caper_android_voice_AudioCapture_process(JNIEnv *env, jobject, jlong handle, jobject buffer, jobject natural,
                                                     jint frames, jint gain, jint strength, jint epoch) {
    auto *capture = reinterpret_cast<Capture *>(handle);
    auto *bytes = static_cast<int16_t *>(env->GetDirectBufferAddress(buffer));
    auto *denoised = static_cast<int16_t *>(env->GetDirectBufferAddress(natural));
    if (!capture || !bytes || !denoised || frames < 1 || frames > 4096 ||
        env->GetDirectBufferCapacity(buffer) < frames * 2 ||
        env->GetDirectBufferCapacity(natural) < frames * 2) return 0;
    std::array<uint32_t, 4096> epochs{};
    int ok = CaperDenoisePipelineProcess(capture->pipeline, bytes, bytes, epochs.data(), frames, gain, epoch);
    // Gain zero overrides queued model/OLA samples and contour history, not
    // just future model input. Both local versions must be exactly silent.
    if (!ok || gain == 0) {
        std::memset(bytes, 0, frames * 2);
        std::memset(denoised, 0, frames * 2);
        capture->contour = {};
        capture->contourEpoch = 0;
        return ok ? CaperDenoisePipelineMode(capture->pipeline) : 0;
    }
    for (int i = 0; i < frames; ++i) if (epochs[i] != static_cast<uint32_t>(epoch)) bytes[i] = 0;
    std::memcpy(denoised, bytes, frames * 2);
    CaperProcessVoiceEpochs(bytes, epochs.data(), frames, capture->rate, 100, strength,
                            &capture->contour, &capture->contourEpoch);
    // A stale hop may be queued from a muted/previous comparison generation.
    for (int i = 0; i < frames; ++i) if (epochs[i] != static_cast<uint32_t>(epoch)) bytes[i] = 0;
    return CaperDenoisePipelineMode(capture->pipeline);
}

extern "C" JNIEXPORT jlongArray JNICALL
Java_chat_caper_android_voice_AudioCapture_diagnostics(JNIEnv *env, jobject, jlong handle) {
    auto *capture = reinterpret_cast<Capture *>(handle);
    CaperDenoiseStatistics stats = CaperDenoisePipelineStatistics(capture ? capture->pipeline : nullptr);
    jlong values[] = {stats.mode, static_cast<jlong>(stats.processedHops),
        static_cast<jlong>(stats.totalProcessingMicros), static_cast<jlong>(stats.maxProcessingMicros),
        static_cast<jlong>(stats.queuedInputFrames)};
    jlongArray result = env->NewLongArray(5);
    env->SetLongArrayRegion(result, 0, 5, values);
    return result;
}
