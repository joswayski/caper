#ifndef CAPER_DPDFNET_H
#define CAPER_DPDFNET_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct CaperDpdfnet CaperDpdfnet;

// Create/warm the CPU model off the audio callback. A new instance owns fresh
// recurrent state and overlap-add history for exactly one microphone session.
CaperDpdfnet *CaperDpdfnetCreate(const char *modelPath);
void CaperDpdfnetDestroy(CaperDpdfnet *engine);
void CaperDpdfnetReset(CaperDpdfnet *engine);
// One 10 ms hop, normalized mono PCM at 48 kHz. Call only from the engine worker.
// On failure, the caller must stop publication; raw capture is not a fallback.
int CaperDpdfnetProcess(CaperDpdfnet *engine, const float input[480], float output[480]);

#ifdef __cplusplus
}
#endif
#endif
