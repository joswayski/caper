#include "CaperVoiceDSP.h"
#include <math.h>

static float limit(float value, float low, float high) {
    return fmaxf(low, fminf(high, value));
}

void CaperProcessVoice(int16_t *samples, unsigned frames, double sampleRate, int inputGain, int strength, CaperVoiceDSP *state) {
    const float amount = limit(strength / 100.f, 0, 1);
    const float input = limit(inputGain / 100.f, 0, 2);
    const float warmth = powf(10.f, 2.f * amount / 20.f) - 1.f;
    const float presence = powf(10.f, 1.5f * amount / 20.f) - 1.f;
    const float makeup = powf(1.35f, amount);
    const float rate = (float)sampleRate;
    const float hp = 1.f / (1.f + 2.f * 3.14159265f * 75.f * amount / rate);
    const float low180 = 1.f - expf(-2.f * 3.14159265f * 180.f / rate);
    const float low1000 = 1.f - expf(-2.f * 3.14159265f * 1000.f / rate);
    const float low3000 = 1.f - expf(-2.f * 3.14159265f * 3000.f / rate);
    for (unsigned i = 0; i < frames; i++) {
        float sample = samples[i] / 32768.f * input;
        if (amount > 0) {
            float filtered = hp * (state->hpOutput + sample - state->hpInput);
            state->hpInput = sample; state->hpOutput = filtered;
            state->low180 += low180 * (filtered - state->low180);
            state->low1000 += low1000 * (filtered - state->low1000);
            state->low3000 += low3000 * (filtered - state->low3000);
            sample = filtered + warmth * state->low180 + presence * (state->low3000 - state->low1000);

            float magnitude = fabsf(sample);
            float attack = 1.f / (rate * 0.008f), release = 1.f / (rate * 0.18f);
            state->compressorEnvelope += (magnitude - state->compressorEnvelope) *
                (magnitude > state->compressorEnvelope ? attack : release);
            const float threshold = 0.0630957f; // -24 dBFS
            if (state->compressorEnvelope > threshold) {
                float ratio = 1.f + 2.f * amount;
                float desired = threshold + (state->compressorEnvelope - threshold) / ratio;
                sample *= desired / state->compressorEnvelope;
            }
            sample *= makeup;
            magnitude = fabsf(sample);
            attack = 1.f / (rate * 0.002f); release = 1.f / (rate * 0.08f);
            state->limiterEnvelope += (magnitude - state->limiterEnvelope) *
                (magnitude > state->limiterEnvelope ? attack : release);
            const float ceiling = 0.794328f; // -2 dBFS
            if (state->limiterEnvelope > ceiling) { sample *= ceiling / state->limiterEnvelope; }
        }
        samples[i] = (int16_t)lrintf(limit(sample, -1.f, 0.999969f) * 32767.f);
    }
}

void CaperProcessVoiceEpochs(int16_t *samples, const uint32_t *epochs, unsigned frames,
                             double sampleRate, int inputGain, int strength,
                             CaperVoiceDSP *state, uint32_t *lastEpoch) {
    unsigned first = 0;
    while (first < frames) {
        uint32_t epoch = epochs[first];
        unsigned end = first + 1;
        while (end < frames && epochs[end] == epoch) end++;
        if (*lastEpoch != epoch) {
            *state = (CaperVoiceDSP){0};
            *lastEpoch = epoch;
        }
        CaperProcessVoice(samples + first, end - first, sampleRate, inputGain, strength, state);
        first = end;
    }
}
