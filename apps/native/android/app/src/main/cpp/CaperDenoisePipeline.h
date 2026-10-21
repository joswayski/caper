#ifndef CAPER_DENOISE_PIPELINE_H
#define CAPER_DENOISE_PIPELINE_H

#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

typedef struct CaperDenoisePipeline CaperDenoisePipeline;
typedef struct {
    uint64_t processedHops;
    uint64_t totalProcessingMicros;
    uint64_t maxProcessingMicros;
    unsigned queuedInputFrames;
    int mode;
} CaperDenoiseStatistics;
// Create/warm on a non-realtime thread before capture. Every pipeline owns a
// model session and a serial worker; no inference or allocation on HAL.
CaperDenoisePipeline *CaperDenoisePipelineCreate(const char *modelPath, double hardwareRate);
void CaperDenoisePipelineDestroy(CaperDenoisePipeline *pipeline);
// Called only by the single capture callback. Returns zero until three hops
// are ready and on overload/failure; never falls through to unprocessed audio.
int CaperDenoisePipelineProcess(CaperDenoisePipeline *pipeline,
                                const int16_t *input, int16_t *output, uint32_t *epochs,
                                unsigned frames, int gain, uint32_t publicationEpoch);
// Local recordings (voice messages) only: nothing here is ever published, so
// there are no publication epochs and no fixed output reserve. Queues the
// block when the worker's backlog has room (1), leaves it to the caller for
// later (0), or fails (-1). A recording can catch up faster than real time,
// never faster than the worker, and stays clear of the RNNoise switch.
int CaperDenoisePipelineQueueLocal(CaperDenoisePipeline *pipeline, const int16_t *input,
                                   unsigned frames, int gain);
// Copies up to `capacity` samples the worker has finished; -1 after a failure.
int CaperDenoisePipelineTakeLocal(CaperDenoisePipeline *pipeline, int16_t *output, unsigned capacity);
int CaperDenoisePipelineFailed(const CaperDenoisePipeline *pipeline);
// 0: unavailable, 1: DPDFNet-8, 2: RNNoise fallback.
int CaperDenoisePipelineMode(const CaperDenoisePipeline *pipeline);
CaperDenoiseStatistics CaperDenoisePipelineStatistics(const CaperDenoisePipeline *pipeline);

#ifdef __cplusplus
}
#endif
#endif
