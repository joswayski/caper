# DPDFNet-8 48 kHz HR runtime

CEVA Apache-2.0 model, pinned Hugging Face revision
`dd6818d00f50c836fed43a6243ebe49116de5964`.
SHA-256: `7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631`.
Size: 14,857,107 bytes. Reproduce with `node scripts/vendor-dpdfnet.mjs 8`
(Python `onnx` required). Metadata is exported from this model.

The ONNX runtime, DSP and worklet are vendored alongside the model. It uses 48 kHz,
960-point FFT, 480-sample hops, Vorbis window, normalization initialization and
bounded-backlog fallback. The model loads lazily for each microphone capture. No
paid API or inference service.

DPDFNet-8 is the default and only DPDFNet microphone filter. Check its active
status after joining; startup failure does not silently fall back to another filter.
