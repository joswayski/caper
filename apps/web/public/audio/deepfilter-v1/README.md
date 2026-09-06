# Vendored DeepFilterNet3 browser runtime

Caper serves these files from its own origin. No runtime CDN, LiveKit client,
API key, license server, or audio upload is used for noise suppression.

- Model: Rikorose/DeepFilterNet `models/DeepFilterNet3_onnx.tar.gz`.
  Verified byte-identical to the upstream main-branch download on 2026-09-06.
  Stored as `DeepFilterNet3.bin` because Nitro treats `.gz` as HTTP compression
  and browsers would decompress it; the model loader needs the gzip bytes intact.
- WASM: MezonAI's prebuilt DeepFilterNet3 Rust/Tract runtime, downloaded from
  `https://cdn.mezon.ai/AI/models/datas/noise_suppression/deepfilternet3/v3/pkg/df_bg.wasm`.
- `df.js`: unmodified generated bindings (including AudioWorklet UTF-8 shims)
  from `mezonai/mezon-noise-suppression`, commit
  `a5212661245a2184370fc3c3dd1f52dc4dffb2a5`, `src/df3/df.js`.
- `worklet.js`: Caper-owned adapter with readiness/error messages and fixed-delay
  sample buffering, not the MezonAI LiveKit wrapper.
- Selected license: MIT; upstream and MezonAI notices are in `LICENSE-MIT`.

SHA-256:

```text
440b5d12b6ea7d95008736f844221d7874ee15de5cb10d3015002470fdba0432  df_bg.wasm
c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616  DeepFilterNet3.bin
db53ee40f42143a0077905f73c6255b272a651f36f451fca6a2bde980c749f71  df.js
```

The third-party prebuilt WASM is pinned by bytes; its exact compiler/toolchain
provenance is not supplied by the distributor. It is not claimed to be a
reproducible Caper build. Upstream's source build entry point is
`wasm-pack build libDF --target no-modules --features wasm`; matching generated
JS bindings and WASM must be updated together and verified in a real worklet.

The model uses 48 kHz mono / 480-sample hops. Attenuation is capped at 40 dB.
The worklet adds 480 samples (10 ms) of adapter delay, in addition to the model's
STFT/lookahead and browser/network delay. This is not a 10 ms end-to-end promise.
The upstream raw-pointer API has no destructor; Caper closes a dedicated
AudioContext to release the associated worklet/WASM instance on replacement/leave.

Update the directory version, microphone asset URL, and test hashes together
when replacing runtime assets. Keep model weights and runtime out of the initial
JavaScript bundle; they load only when enhanced microphone capture is requested.

Reference: Hendrik Schröter et al., “DeepFilterNet: Perceptually Motivated
Real-Time Speech Enhancement”, INTERSPEECH 2023.
