#include "CaperDpdfnet.h"
#include <onnxruntime_cxx_api.h>
#include <algorithm>
#include <array>
#include <cmath>
#include <complex>
#include <cstdlib>
#include <memory>
#include <sstream>
#include <string>
#include <vector>

namespace {
constexpr int kHop = 480, kN = 960, kBins = 481, kState = 90228;
using Complex = std::complex<double>;

// Same mixed-radix transform as the browser DPDFNet stream. The 960-point
// transform is not a power of two; plans allocate once, never per inference.
struct Plan {
    int n, radix, size;
    std::vector<Complex> roots, output;
    std::vector<std::unique_ptr<Plan>> children;
    Plan(int length, bool inverse) : n(length),
        radix(length % 2 == 0 ? 2 : length % 3 == 0 ? 3 : length % 5 == 0 ? 5 : length),
        size(length / radix), roots(length * radix), output(length) {
        if (length == 1) return;
        for (int index = 0; index < length; ++index)
            for (int r = 0; r < radix; ++r) {
                double angle = (inverse ? 1 : -1) * 2 * std::acos(-1.) * r * index / length;
                roots[index * radix + r] = {std::cos(angle), std::sin(angle)};
            }
        for (int r = 0; r < radix; ++r) children.emplace_back(std::make_unique<Plan>(size, inverse));
    }
    void execute(const Complex *input, int offset = 0, int stride = 1) {
        if (n == 1) { output[0] = input[offset]; return; }
        for (int r = 0; r < radix; ++r) children[r]->execute(input, offset + r * stride, stride * radix);
        for (int q = 0; q < size; ++q)
            for (int s = 0; s < radix; ++s) {
                int index = q + size * s;
                Complex sum = 0;
                for (int r = 0; r < radix; ++r)
                    sum += children[r]->output[q] * roots[index * radix + r];
                output[index] = sum;
            }
    }
};

std::vector<float> parseState(const std::string &csv) {
    std::vector<float> values;
    std::istringstream input(csv);
    std::string token;
    while (std::getline(input, token, ',')) values.push_back(std::stof(token));
    return values;
}
}

struct CaperDpdfnet {
    Ort::Env env{ORT_LOGGING_LEVEL_WARNING, "Caper microphone"};
    Ort::SessionOptions options;
    Ort::Session session{nullptr};
    Ort::MemoryInfo memory{Ort::MemoryInfo::CreateCpu(OrtArenaAllocator, OrtMemTypeDefault)};
    Plan forward{kN, false}, inverse{kN, true};
    std::array<float, kN> analysis{}, ola{}, window{};
    std::array<Complex, kN> time{}, frequency{};
    std::array<float, kBins * 2> spec{};
    std::vector<float> state;
    bool primed = false;

    explicit CaperDpdfnet(const char *modelPath) {
        options.SetGraphOptimizationLevel(GraphOptimizationLevel::ORT_ENABLE_ALL);
        options.SetIntraOpNumThreads(1);
        session = Ort::Session(env, modelPath, options);
        auto metadata = session.GetModelMetadata();
        auto allocator = Ort::AllocatorWithDefaultOptions();
        auto erb = metadata.LookupCustomMetadataMapAllocated("erb_norm_init", allocator);
        auto spectrum = metadata.LookupCustomMetadataMapAllocated("spec_norm_init", allocator);
        auto normSize = metadata.LookupCustomMetadataMapAllocated("erb_norm_state_size", allocator);
        if (!erb || !spectrum || !normSize) throw std::runtime_error("Missing model state metadata");
        auto erbValues = parseState(erb.get()), specValues = parseState(spectrum.get());
        state.assign(kState, 0);
        if (std::stoi(normSize.get()) != static_cast<int>(erbValues.size()) ||
            erbValues.size() + specValues.size() > state.size())
            throw std::runtime_error("Unexpected model state metadata");
        std::copy(erbValues.begin(), erbValues.end(), state.begin());
        std::copy(specValues.begin(), specValues.end(), state.begin() + erbValues.size());
        for (int i = 0; i < kN; ++i) {
            double sine = std::sin(std::acos(-1.) * (i + 0.5) / kN);
            window[i] = std::sin(std::acos(-1.) / 2 * sine * sine);
        }
        // First-run graph compilation must not delay a live capture hop. Warm
        // against a separate initial state so it cannot alter live recurrence.
        std::array<float, kBins * 2> silence{};
        auto warmState = state;
        (void)infer(silence, warmState);
    }

    std::array<float, kBins * 2> infer(const std::array<float, kBins * 2> &spectrum,
                                       std::vector<float> &recurrent) {
        std::array<int64_t, 4> specShape{1, 1, kBins, 2};
        std::array<int64_t, 1> stateShape{kState};
        std::array<Ort::Value, 2> inputs{
            Ort::Value::CreateTensor<float>(memory, const_cast<float *>(spectrum.data()), spectrum.size(), specShape.data(), specShape.size()),
            Ort::Value::CreateTensor<float>(memory, recurrent.data(), recurrent.size(), stateShape.data(), stateShape.size())};
        const char *inputNames[] = {"spec", "state_in"}, *outputNames[] = {"spec_e", "state_out"};
        auto outputs = session.Run(Ort::RunOptions{nullptr}, inputNames, inputs.data(), 2, outputNames, 2);
        if (outputs.size() != 2 || outputs[0].GetTensorTypeAndShapeInfo().GetElementCount() != kBins * 2 ||
            outputs[1].GetTensorTypeAndShapeInfo().GetElementCount() != kState)
            throw std::runtime_error("Unexpected model output shape");
        std::array<float, kBins * 2> enhanced;
        std::copy_n(outputs[0].GetTensorData<float>(), enhanced.size(), enhanced.begin());
        std::copy_n(outputs[1].GetTensorData<float>(), recurrent.size(), recurrent.begin());
        return enhanced;
    }

    void process(const float input[kHop], float output[kHop]) {
        std::move(analysis.begin() + kHop, analysis.end(), analysis.begin());
        std::copy_n(input, kHop, analysis.begin() + kHop);
        for (int i = 0; i < kN; ++i) time[i] = {analysis[i] * window[i], 0};
        forward.execute(time.data());
        for (int i = 0; i < kBins; ++i) {
            spec[i * 2] = static_cast<float>(forward.output[i].real());
            spec[i * 2 + 1] = static_cast<float>(forward.output[i].imag());
        }
        auto enhanced = infer(spec, state);
        frequency.fill(0);
        for (int i = 0; i < kBins; ++i) frequency[i] = {enhanced[i * 2], enhanced[i * 2 + 1]};
        for (int i = 1; i < kBins - 1; ++i) frequency[kN - i] = std::conj(frequency[i]);
        inverse.execute(frequency.data());
        for (int i = 0; i < kHop; ++i) {
            output[i] = ola[i] + static_cast<float>(inverse.output[i].real() / kN) * window[i];
            ola[i] = static_cast<float>(inverse.output[i + kHop].real() / kN) * window[i + kHop];
            if (!std::isfinite(output[i])) throw std::runtime_error("Invalid denoised sample");
            if (!primed) output[i] = 0;
        }
        primed = true;
    }
};

extern "C" CaperDpdfnet *CaperDpdfnetCreate(const char *modelPath) {
    try { return modelPath ? new CaperDpdfnet(modelPath) : nullptr; }
    catch (...) { return nullptr; }
}
extern "C" void CaperDpdfnetDestroy(CaperDpdfnet *engine) { delete engine; }
extern "C" int CaperDpdfnetProcess(CaperDpdfnet *engine, const float input[480], float output[480]) {
    if (!engine || !input || !output) return 0;
    try { engine->process(input, output); return 1; }
    catch (...) { return 0; }
}
