# DPDFNet-8 48 kHz HR runtime

CEVA Apache-2.0 model, pinned Hugging Face revision
`dd6818d00f50c836fed43a6243ebe49116de5964`.
SHA-256: `7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631`.
Size: 14,857,107 bytes. Reproduce with `node scripts/vendor-dpdfnet.mjs 8`
(Python `onnx` required). Metadata is exported from this model.

The ONNX Runtime 1.23.2 WASM, DSP and worklet are vendored alongside the model.
Runtime WASM: 11,905,541 bytes; SHA-256
`45eaee27761ad883742a8d4b8fce1538d60ce43b51adf1726fafccc59b8c1a15`.
Use binary-safe copies when moving WASM or ONNX files; never round-trip through
text. Asset directories are immutable once served: a changed file gets a new
directory version.

Processing uses 48 kHz, a 960-point FFT, 480-sample hops, a Vorbis window,
normalization initialization, and bounded-backlog bypass when the device cannot
sustain real-time inference. Voice-page preparation initializes one unused
worker for exclusive handoff to microphone capture. No paid API or inference
service.

DPDFNet-8 is the preferred microphone filter. On startup failure, runtime
failure or overload it falls back to DPDFNet-2 HR (`../dpdfnet2-v1/`), then
RNNoise, with browser suppression during transitions; startup fallbacks finish
before publication, runtime ones keep tracks live. Each engine gets one error
retry per capture; sustained overload skips retries.
