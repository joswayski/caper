# DPDFNet-2 48 kHz HR experimental runtime

On-device streaming inference for CEVA's `dpdfnet2_48khz_hr.onnx`. The model and
reference code are Apache-2.0 licensed. Model revision:
`dd6818d00f50c836fed43a6243ebe49116de5964`, SHA-256
`7f0575a5cec0ba4ffd8f8bd657e06d007e4ccdd955d76faab922b9d3291dc14b` (10,493,337 bytes).

The runtime follows CEVA's `package/src/dpdfnet/{audio,stream,onnx_backend}.py` at
commit `1333776d470f01ecf4a533f098f4e8aeb3d00b89`: unscaled 960-point real FFT,
480-sample hop, Vorbis window, model-managed normalization, and windowed OLA.
`metadata.json` contains the model's non-zero normalization state exported by
`scripts/vendor-dpdfnet.mjs`; it is not guessed or replaced with zeros.

ONNX Runtime Web 1.23.2 is pinned in `package.json`; its MIT-licensed browser files
are vendored here for lazy, same-origin loading. One WASM thread avoids requiring
cross-origin isolation. AudioWorklet callbacks only frame/copy audio. A Worker
runs FFT and inference, transfers blocks, prebuffers three output hops, and fails
over on underrun or an eight-hop backlog rather than accumulating unbounded latency.
Warm-up happens before readiness and resets state before capture. Caper now uses
this runtime by default following owner listening tests; device performance remains
experimental. Synthetic tests establish execution and continuity, not speech quality.

Reproduce from repository root with `npm ci`, Python's `onnx` package installed,
then `node scripts/vendor-dpdfnet.mjs`. Normal builds do not download anything.
The script verifies the model and license checksums, copies the lockfile-pinned
runtime, and exports normalization state from the model. Distributed notices:
`LICENSE-APACHE-2.0` (CEVA), `ORT-LICENSE`, `ORT-ThirdPartyNotices.txt`.
