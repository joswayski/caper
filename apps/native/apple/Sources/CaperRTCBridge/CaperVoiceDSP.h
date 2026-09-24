#ifndef CAPER_VOICE_DSP_H
#define CAPER_VOICE_DSP_H

#include <stdint.h>

typedef struct {
    float low180, low1000, low3000;
    float hpInput, hpOutput;
    float compressorEnvelope, limiterEnvelope;
} CaperVoiceDSP;

// Mono signed PCM at the current hardware rate. State is owned by the capture callback thread.
void CaperProcessVoice(int16_t *samples, unsigned frames, double sampleRate, int inputGain, int strength, CaperVoiceDSP *state);
// The worker can return multiple publication epochs in one HAL buffer. Reset
// contour history at each boundary, including private (epoch zero) samples.
void CaperProcessVoiceEpochs(int16_t *samples, const uint32_t *epochs, unsigned frames,
                             double sampleRate, int inputGain, int strength,
                             CaperVoiceDSP *state, uint32_t *lastEpoch);

#endif
