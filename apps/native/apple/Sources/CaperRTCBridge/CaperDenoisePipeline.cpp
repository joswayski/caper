#include "CaperDenoisePipeline.h"
#include "CaperDpdfnet.h"
#include <rnnoise.h>
#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <thread>

namespace {
constexpr unsigned kCapacity = 16384, kMask = kCapacity - 1, kHop = 480;
struct Ring {
    std::array<float, kCapacity> values{};
    std::array<uint32_t, kCapacity> epochs{};
    std::atomic<unsigned> head{0}, tail{0};
    bool push(float sample, uint32_t epoch) {
        unsigned h = head.load(std::memory_order_relaxed);
        if (h - tail.load(std::memory_order_acquire) >= kCapacity) return false;
        values[h & kMask] = sample;
        epochs[h & kMask] = epoch;
        head.store(h + 1, std::memory_order_release);
        return true;
    }
    bool pop(float &sample, uint32_t &epoch) {
        unsigned t = tail.load(std::memory_order_relaxed);
        if (t == head.load(std::memory_order_acquire)) return false;
        sample = values[t & kMask];
        epoch = epochs[t & kMask];
        tail.store(t + 1, std::memory_order_release);
        return true;
    }
    unsigned count() const {
        return head.load(std::memory_order_acquire) - tail.load(std::memory_order_acquire);
    }
};
}

struct CaperDenoisePipeline {
    CaperDpdfnet *model;
    DenoiseState *fallback;
    const double rate;
    Ring incoming, outgoing;
    std::atomic<bool> failed{false}, stopping{false};
    std::atomic<int> mode{0};
    std::atomic<unsigned> completedHops{0};
    std::atomic<uint64_t> totalProcessingMicros{0}, maxProcessingMicros{0};
    std::thread worker;

    CaperDenoisePipeline(const char *path, double sampleRate)
        : model(CaperDpdfnetCreate(path)), fallback(rnnoise_create(nullptr)), rate(sampleRate) {
        mode.store(model ? 1 : fallback ? 2 : 0, std::memory_order_release);
        if (mode.load(std::memory_order_acquire)) worker = std::thread([this] { run(); });
    }
    ~CaperDenoisePipeline() {
        stopping.store(true, std::memory_order_release);
        if (worker.joinable()) worker.join();
        CaperDpdfnetDestroy(model);
        rnnoise_destroy(fallback);
    }

    void run() {
        // Two adjacent hardware samples and a fractional source position form
        // the input resampler. Both rings remain single-producer/single-consumer.
        float left = 0, right = 0;
        uint32_t leftEpoch = 0, rightEpoch = 0;
        bool haveLeft = false, haveRight = false;
        uint32_t hopEpoch = 0, previousHopEpoch = 0;
        double inputPosition = 0, outputPosition = 0;
        std::array<float, kHop> hop{}, denoised{};
        unsigned filled = 0;
        while (!stopping.load(std::memory_order_acquire) && !failed.load(std::memory_order_acquire)) {
            if (incoming.count() > rate * .08) {
                if (fallback) mode.store(2, std::memory_order_release);
                else { failed.store(true, std::memory_order_release); break; }
            }
            if (!haveLeft) haveLeft = incoming.pop(left, leftEpoch);
            if (haveLeft && !haveRight) haveRight = incoming.pop(right, rightEpoch);
            if (!haveRight) { std::this_thread::sleep_for(std::chrono::milliseconds(1)); continue; }
            // Preserve the fractional position when waiting for the next sample.
            while (inputPosition >= 1) {
                float next;
                uint32_t nextEpoch;
                if (!incoming.pop(next, nextEpoch)) break;
                left = right; leftEpoch = rightEpoch;
                right = next; rightEpoch = nextEpoch;
                inputPosition -= 1;
            }
            if (inputPosition >= 1) { std::this_thread::sleep_for(std::chrono::milliseconds(1)); continue; }
            if (!filled) hopEpoch = leftEpoch && leftEpoch == rightEpoch ? leftEpoch : 0;
            else if (hopEpoch != leftEpoch || hopEpoch != rightEpoch) hopEpoch = 0;
            hop[filled++] = static_cast<float>(left + (right - left) * inputPosition);
            inputPosition += rate / 48000.;
            if (filled != kHop) continue;
            filled = 0;
            auto started = std::chrono::steady_clock::now();
            if (hopEpoch && hopEpoch != previousHopEpoch) {
                // Reopening publication cannot carry recurrent/overlap state
                // trained on private muted or local-comparison capture.
                CaperDpdfnetReset(model);
                if (mode.load(std::memory_order_acquire) == 2) {
                    rnnoise_destroy(fallback);
                    fallback = rnnoise_create(nullptr);
                    if (!fallback) { failed.store(true, std::memory_order_release); break; }
                }
            }
            if (mode.load(std::memory_order_acquire) == 1 && !CaperDpdfnetProcess(model, hop.data(), denoised.data())) {
                mode.store(fallback ? 2 : 0, std::memory_order_release);
            }
            if (mode.load(std::memory_order_acquire) == 2) {
                for (auto &sample : hop) sample *= 32768.f;
                (void)rnnoise_process_frame(fallback, denoised.data(), hop.data());
                for (auto &sample : denoised) sample /= 32768.f;
            } else if (!mode.load(std::memory_order_acquire)) {
                failed.store(true, std::memory_order_release); break;
            }
            // Resample the model's 48 kHz result back to the AUHAL client's
            // *actual* rate. No AudioUnit sample-rate conversion is assumed.
            while (outputPosition < kHop) {
                int index = static_cast<int>(outputPosition);
                float fraction = static_cast<float>(outputPosition - index);
                float value = denoised[index] + (denoised[std::min(index + 1, 479)] - denoised[index]) * fraction;
                if (!std::isfinite(value)) { failed.store(true, std::memory_order_release); break; }
                if (!outgoing.push(value, previousHopEpoch == hopEpoch ? hopEpoch : 0)) {
                    failed.store(true, std::memory_order_release); break;
                }
                outputPosition += 48000. / rate;
            }
            outputPosition -= kHop;
            previousHopEpoch = hopEpoch;
            hopEpoch = 0;
            auto micros = std::chrono::duration_cast<std::chrono::microseconds>(
                std::chrono::steady_clock::now() - started).count();
            totalProcessingMicros.fetch_add(micros, std::memory_order_relaxed);
            auto maximum = maxProcessingMicros.load(std::memory_order_relaxed);
            while (maximum < static_cast<uint64_t>(micros) &&
                   !maxProcessingMicros.compare_exchange_weak(maximum, micros, std::memory_order_relaxed)) {}
            completedHops.fetch_add(1, std::memory_order_release);
        }
    }
};

extern "C" CaperDenoisePipeline *CaperDenoisePipelineCreate(const char *path, double rate) {
    if (!std::isfinite(rate) || rate < 8000 || rate > 192000) return nullptr;
    try {
        auto *pipeline = new CaperDenoisePipeline(path, rate);
        if (!pipeline->mode.load(std::memory_order_acquire)) { delete pipeline; return nullptr; }
        return pipeline;
    } catch (...) { return nullptr; }
}
extern "C" void CaperDenoisePipelineDestroy(CaperDenoisePipeline *pipeline) { delete pipeline; }
extern "C" int CaperDenoisePipelineFailed(const CaperDenoisePipeline *pipeline) {
    return !pipeline || pipeline->failed.load(std::memory_order_acquire);
}
extern "C" int CaperDenoisePipelineMode(const CaperDenoisePipeline *pipeline) {
    return pipeline && !pipeline->failed.load(std::memory_order_acquire)
        ? pipeline->mode.load(std::memory_order_acquire) : 0;
}
extern "C" CaperDenoiseStatistics CaperDenoisePipelineStatistics(const CaperDenoisePipeline *pipeline) {
    if (!pipeline) return {};
    return {pipeline->completedHops.load(std::memory_order_acquire),
        pipeline->totalProcessingMicros.load(std::memory_order_relaxed),
        pipeline->maxProcessingMicros.load(std::memory_order_relaxed),
        pipeline->incoming.count(), CaperDenoisePipelineMode(pipeline)};
}
extern "C" int CaperDenoisePipelineProcess(CaperDenoisePipeline *pipeline,
                                             const int16_t *input, int16_t *output, uint32_t *epochs,
                                             unsigned frames, int gain, uint32_t publicationEpoch) {
    if (!pipeline || !input || !output || !epochs || pipeline->failed.load(std::memory_order_acquire)) {
        if (output) std::memset(output, 0, frames * sizeof(int16_t));
        if (epochs) std::memset(epochs, 0, frames * sizeof(uint32_t));
        return 0;
    }
    // Switch to RNNoise at eight hops on the worker; allow a bounded further
    // reserve for it to catch up. A truly stalled worker still fails closed.
    if (pipeline->incoming.count() + frames > pipeline->rate * .16) {
        pipeline->failed.store(true, std::memory_order_release);
        std::memset(output, 0, frames * sizeof(int16_t));
        std::memset(epochs, 0, frames * sizeof(uint32_t));
        return 0;
    }
    const float scale = std::clamp(gain, 0, 200) / 100.f / 32768.f;
    for (unsigned i = 0; i < frames; ++i) {
        if (!pipeline->incoming.push(input[i] * scale, publicationEpoch)) {
            pipeline->failed.store(true, std::memory_order_release);
            std::memset(output, 0, frames * sizeof(int16_t));
            std::memset(epochs, 0, frames * sizeof(uint32_t));
            return 0;
        }
    }
    // The caller may pass the same buffer for input and output. Never leave
    // raw capture in a short read or startup frame.
    std::memset(output, 0, frames * sizeof(int16_t));
    std::memset(epochs, 0, frames * sizeof(uint32_t));
    // Web reserves three completed 10 ms hops before exposing any output.
    if (pipeline->completedHops.load(std::memory_order_acquire) < 3) {
        return 1;
    }
    for (unsigned i = 0; i < frames; ++i) {
        float sample = 0;
        uint32_t epoch = 0;
        if (!pipeline->outgoing.pop(sample, epoch)) break;
        output[i] = static_cast<int16_t>(std::lrint(std::clamp(sample, -1.f, .999969f) * 32767.f));
        epochs[i] = epoch;
    }
    return 1;
}
