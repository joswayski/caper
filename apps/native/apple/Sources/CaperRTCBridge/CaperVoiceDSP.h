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

#endif
