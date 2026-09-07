# DPDFNet-8 48 kHz HR runtime

CEVA Apache-2.0 model, pinned Hugging Face revision
`dd6818d00f50c836fed43a6243ebe49116de5964`.
SHA-256: `7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631`.
Size: 14,857,107 bytes. Reproduce with `node scripts/vendor-dpdfnet.mjs 8`
(Python `onnx` required). Metadata is exported from this model.

The ONNX runtime, DSP and worklet are vendored alongside the model. It uses 48 kHz,
960-point FFT, 480-sample hops, Vorbis window, normalization initialization and
bounded-backlog bypass when the device cannot sustain real-time inference. The
fallback attempts browser noise suppression and otherwise remains unprocessed.
Voice-page preparation initializes one unused worker
for exclusive handoff to microphone capture. No paid API or inference service.

Asset version v2 restores the intact ONNX Runtime 1.23.2 WASM binary after the v1
relocation corrupted it. The model and DSP are unchanged. The new directory avoids
reusing immutable cached v1 responses. Runtime WASM: 11,905,541 bytes; SHA-256
`45eaee27761ad883742a8d4b8fce1538d60ce43b51adf1726fafccc59b8c1a15`.
Use binary-safe copies when moving WASM or ONNX files; never round-trip through text.

DPDFNet-8 is the default and only DPDFNet microphone filter. Check its active
status after joining. Startup failure blocks publication; runtime overload keeps the
microphone live, attempts browser suppression and reports the resulting state in the UI.
