#include "../Sources/CaperRTCBridge/CaperDenoisePipeline.h"
#include <algorithm>
#include <array>
#include <cassert>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <thread>

int main(int argc, char **argv) {
    if (argc != 4) return 2;
    const int rate = std::atoi(argv[2]);
    const unsigned frames = rate / 100;
    if (frames > 1920 || frames < 80) return 2;
    const bool fallbackOnly = argv[1][0] == '-';
    auto *engine = CaperDenoisePipelineCreate(fallbackOnly ? nullptr : argv[1], rate);
    if (!engine) return 3;
    assert(CaperDenoisePipelineMode(engine) == (fallbackOnly ? 2 : 1));
    FILE *inputFile = std::fopen(argv[3], "rb");
    if (!inputFile) return 2;
    std::array<int16_t, 1920> input{};
    std::array<uint32_t, 1920> epochs{};
    int audible = 0, hops = 0;
    while (std::fread(input.data(), sizeof(int16_t), frames, inputFile) == frames) {
        // In-place processing must never leak raw input while the worker is
        // still warming or during a bounded output-ring underrun.
        assert(CaperDenoisePipelineProcess(engine, input.data(), input.data(), epochs.data(), frames, 150, 11));
        if (hops == 0) for (unsigned i = 0; i < frames; ++i) assert(input[i] == 0);
        if (hops > 5) for (unsigned i = 0; i < frames; ++i) audible += std::abs(int(input[i])) > 10;
        hops++;
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    int failed = CaperDenoisePipelineFailed(engine);
    auto stats = CaperDenoisePipelineStatistics(engine);
    assert(!failed);
    CaperDenoisePipelineDestroy(engine);

    // A previous public frame may be buffered when the user mutes or begins
    // comparison. On reopening, neither that old epoch nor private comparison
    // input is eligible; only post-transition capture may publish. Run against
    // both model and fallback at each tested hardware rate.
    engine = CaperDenoisePipelineCreate(fallbackOnly ? nullptr : argv[1], rate);
    assert(engine);
    std::rewind(inputFile);
    int reopened = 0, privateNatural = 0, transitionSilence = 0;
    for (int hop = 0; hop < 105; ++hop) {
        if (std::fread(input.data(), sizeof(int16_t), frames, inputFile) != frames) {
            std::rewind(inputFile);
            assert(std::fread(input.data(), sizeof(int16_t), frames, inputFile) == frames);
        }
        const uint32_t epoch = hop < 45 ? 11 : hop < 60 ? 0 : 13;
        assert(CaperDenoisePipelineProcess(engine, input.data(), input.data(), epochs.data(), frames, 150, epoch));
        for (unsigned i = 0; i < frames; ++i) {
            // Natural comparison remains available locally even when private.
            if (!epoch && std::abs(int(input[i])) > 10) ++privateNatural;
            if (hop >= 60 && epochs[i] != 13) ++transitionSilence;
            if (hop >= 60 && epochs[i] == 13 && std::abs(int(input[i])) > 10) ++reopened;
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(12));
    }
    std::fclose(inputFile);
    assert(!CaperDenoisePipelineFailed(engine));
    assert(privateNatural > 100 && reopened > 100 && transitionSilence > 0);
    CaperDenoisePipelineDestroy(engine);

    engine = CaperDenoisePipelineCreate(fallbackOnly ? nullptr : argv[1], rate);
    assert(engine);
    // A stalled worker must fail closed rather than leaking unprocessed PCM.
    for (int i = 0; i < 100 && !CaperDenoisePipelineFailed(engine); ++i) {
        input.fill(3000);
        CaperDenoisePipelineProcess(engine, input.data(), input.data(), epochs.data(), frames, 150, 13);
    }
    input.fill(3000);
    assert(!CaperDenoisePipelineProcess(engine, input.data(), input.data(), epochs.data(), frames, 150, 13));
    for (unsigned i = 0; i < frames; ++i) assert(input[i] == 0 && epochs[i] == 0);
    CaperDenoisePipelineDestroy(engine);
    std::printf("native pipeline %d Hz: %d hops, %d audible samples, failed=%d, mode=%d, mean=%.2f ms, max=%.2f ms\n",
        rate, hops, audible, failed, stats.mode,
        stats.processedHops ? double(stats.totalProcessingMicros) / stats.processedHops / 1000 : 0,
        double(stats.maxProcessingMicros) / 1000);
    return hops > 20 && audible > 100 && !failed ? 0 : 4;
}
