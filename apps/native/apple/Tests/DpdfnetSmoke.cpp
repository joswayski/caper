#include "../Sources/CaperRTCBridge/CaperDpdfnet.h"
#include <algorithm>
#include <array>
#include <cassert>
#include <cmath>
#include <cstdio>
#include <vector>

int main(int argc, char **argv) {
    if (argc != 3) return 2;
    FILE *fixture = std::fopen(argv[2], "rb");
    if (!fixture) return 2;
    auto *engine = CaperDpdfnetCreate(argv[1]);
    if (!engine) return 3;
    std::array<float, 480> input{}, output{};
    int nonzero = 0;
    for (int hop = 0; std::fread(input.data(), sizeof(float), input.size(), fixture) == input.size(); ++hop) {
        assert(CaperDpdfnetProcess(engine, input.data(), output.data()));
        for (float value : output) {
            assert(std::isfinite(value));
            if (hop == 0) assert(value == 0);
            if (hop > 2 && std::abs(value) > .0001f) ++nonzero;
        }
    }
    std::fclose(fixture);
    CaperDpdfnetDestroy(engine);
    std::printf("native ONNX 48 kHz PCM: %d nonzero samples after warmup\n", nonzero);
    return nonzero > 100 ? 0 : 4;
}
