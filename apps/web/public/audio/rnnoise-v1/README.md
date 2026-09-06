# RNNoise asset provenance

Free, local 48 kHz RNNoise inference. No service or runtime npm dependency.
The binary is extracted from `@shiguredo/rnnoise-wasm@2025.1.5`, built from
[Xiph RNNoise](https://github.com/xiph/rnnoise/tree/70f1d256acd4b34a572f999a05c87bf00b67730d).
Includes its built-in model. RNNoise is BSD-3-Clause (`COPYING`); the packaging
project is Apache-2.0 (`LICENSE-APACHE`). Both notices are distributed here.

Reproduce from repository root: `node scripts/vendor-rnnoise.mjs`.
The script verifies the npm archive integrity, extracted binary, and upstream
license. Normal builds require no external downloads.

WASM SHA-256: `b3b67c9eae8f0791aad468c708659e0850bb37b0fb9c8a8666f2d7b0b6869bc4`.

The upstream JS wrapper assumes Window/WorkerGlobalScope, not AudioWorkletGlobalScope.
`rnnoise.js` is Caper's small host for this exact binary's exports/imports: initialize
stack/constructors, create state, allocate one 480-float buffer, process in place,
destroy/free. Float PCM is scaled by 32768 on input and divided on output as required
by RNNoise. VAD probability is deliberately not used as a speech gate. Memory growth
is disabled on the render thread; allocation failures trigger capture fallback.
Update the URL version, checksums, ABI host and inference tests together when upgrading.

This is a different model, not a guarantee of better speech preservation than
DeepFilterNet or Krisp. Synthetic-noise tests are not speech-quality benchmarks.
