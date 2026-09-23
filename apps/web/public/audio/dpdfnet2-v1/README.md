# DPDFNet-2 48 kHz HR fallback

CEVA Apache-2.0 model, pinned Hugging Face revision
`dd6818d00f50c836fed43a6243ebe49116de5964`, `onnx/dpdfnet2_48khz_hr.onnx`.
SHA-256: `7f0575a5cec0ba4ffd8f8bd657e06d007e4ccdd955d76faab922b9d3291dc14b`.
Size: 10,493,337 bytes. Reproduce with `node scripts/vendor-dpdfnet.mjs 2`
(Python `onnx` required). Metadata is exported from this model, including its
56,436-value recurrent state; the 8 HR state is not interchangeable.

Uses the existing DSP, worklet-v3 and ONNX Runtime 1.23.2 binaries in
`../dpdfnet8-v2/`; see that directory for runtime checksums and provenance.
Both models use 48 kHz mono, 960-point FFT and 480-sample hops. Upstream lists
2.42G MACs for 2 HR versus 7.17G for 8 HR; this is not measured device performance.

Loaded only after DPDFNet-8 initialization failure, processor failure or sustained
backlog. If 2 HR fails, RNNoise is next. Each capture starts with 8 HR; recovery
does not automatically upgrade mid-capture. All processing stays on-device.
