# DPDFNet-8 48 kHz HR comparison model

CEVA Apache-2.0 model, pinned Hugging Face revision
`dd6818d00f50c836fed43a6243ebe49116de5964`.
SHA-256: `7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631`.
Size: 14,857,107 bytes. Reproduce with `node scripts/vendor-dpdfnet.mjs 8`
(Python `onnx` required). Metadata is exported from this model, not model 2.

Shares the immutable DPDFNet-2 v1 ONNX runtime, DSP and worklet. Same 48 kHz,
960-point FFT, 480-sample hops, Vorbis window, normalization initialization and
bounded-backlog fallback. Only one model runs per microphone capture; selection
loads this model lazily. No paid API or inference service.

This is a listening comparison option, not a claimed quality upgrade. Model 2
remains the default. Check active status before comparing; fallback is not model 8.
