# Native voice integration probe

This source tree pins the exact raw `libwebrtc` revision selected for Caper.
The desktop app imports its media, gateway and state modules directly; the
standalone probe remains for low-level ADM checks. It does not use LiveKit
rooms or servers.

Implemented in the snapshot:

- Native WASAPI/PulseAudio/ALSA ADM acquisition, microphone and speaker
  enumeration/selection, Opus publication and automatic native playout.
- Caper general/account-channel join, publish, subscribe, negotiate, state,
  snapshot lease, close and leave operations against the existing Cloudflare
  SFU contract.
- Account credentials only in `Authorization`; the media capability only in
  `x-caper-media-token`; redirects disabled and no capability URLs.
- Roster/track reconciliation, local remote-track mute, mute/deafen with prior
  mute intent restored, synchronous local teardown, and setup rollback.
- Unified-gateway media snapshots with bounded socket cancellation, reconnect,
  terminal access denial, capability-safe framing and real activity age.
- TURN credential refresh followed by replay-safe ICE restart and ACK.
- A narrow pinned Rust/CXX binding extension for WebRTC's existing
  `local_description()` getter. The current-only getter is empty before the
  initial answer and stale during ICE restart; the pending getter is read
  immediately after set-local, matching the web's non-trickle Cloudflare flow.
  A separate native regression waits for completed candidate gathering. The
  source under `vendor/` derives from
  `livekit/rust-sdks` commit `5a656c4bc4bc5c4f7fad17ab69044db3b487dfeb`
  (`libwebrtc 0.3.49`, `webrtc-sys 0.3.46`, build helper `0.3.19`).
- Async HTTP with structured status/machine-code errors. A terminal account or
  channel denial closes local capture and playout immediately; a provider
  `ice_restart_retry` code remains retryable even if its HTTP status is 403.
  A pending restart retains exact generation, sequence and pending SDP bytes; lost
  answers replay identical bytes and failed ACKs retry without a new offer.
- A pre-join cancellation handle drops in-flight HTTP and closes any acquired
  native capture/peer via the setup guard. Remote leave is dispatched off the
  dropping thread after local teardown; a join accepted by the server but
  cancelled before its response remains subject to the server's lease expiry.
- Gateway DNS/TCP/TLS/upgrade setup has a total five-second deadline and
  prompt stop: a watchdog shuts down a cloned TCP handle even while a peer
  trickles TLS/HTTP handshake bytes within individual read timeouts. Upgrade
  redirects are refused rather than forwarding the Authorization header. DNS
  runs in an uncredentialed resolver thread because synchronous system
  resolution cannot be interrupted.
- Native connection diagnostics for bitrate, loss, jitter, RTT and relay route.
- Generation-fenced state semantics where browsing does not end active voice,
  while explicit Join, logout and access revocation replace/end it.

Desktop integration and limitations:

- The desktop shell has explicit Join/Leave, a voice roster, mute/deafen and
  device selection. It does not claim live/physical voice acceptance.
- The pinned safe binding exposes no ADM input/output gain or per-track gain
  API, and no device-capture monitor tap for natural/enhanced mic playback.
  Those web controls must remain absent rather than inert until the binding is
  extended and verified. Physical Windows/Linux audio and live SFU acceptance
  also remain separate from the virtual-device compile/runtime proof.
- The wrapper defaults to GatherContinually, which never reaches Complete in
  non-trickle SDP setup. The desktop explicitly uses GatherOnce; a native test
  now confirms Complete and a candidate-bearing pending offer in this orb.
  A silent virtual-device public General smoke connected and published two
  sessions, but snapshot/reconciliation returned HTTP 502 or `unauthorized`
  before two-way subscription could be confirmed. Live SFU, separate-network
  ICE/TURN and physical audio acceptance remain open.

Linux x64:

```sh
WEBRTC="$(python3 fetch_libwebrtc.py --platform linux)"
CC=clang-21 CXX=clang++-21 LK_CUSTOM_WEBRTC="$WEBRTC" \
  CARGO_BUILD_JOBS=2 cargo run --locked
```

Verified build packages are `build-essential`, `pkg-config`, `libglib2.0-dev`,
`clang-21`, and `lld-21`. Set `CC=clang-21` and `CXX=clang++-21`; the archive's
Chromium hermetic libc++ headers require Clang. Runtime audio backends are
loaded dynamically and require `libpulse0` and/or `libasound2`.

Windows x64, from a VS 2022 developer PowerShell:

```powershell
$WebRtc = python .\fetch_libwebrtc.py --platform windows
$env:LK_CUSTOM_WEBRTC = $WebRtc
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo run --locked --target x86_64-pc-windows-msvc
```

The probe prints device names locally. They must not be sent to Caper servers or
diagnostic logs. A machine without audio devices may report that ADM is
unavailable; compilation/linking remains the CI acceptance check. The Linux
runtime check was also exercised against a PulseAudio null sink and virtual
source, proving ADM acquisition, enumeration, PeerConnection creation and
teardown without claiming physical hardware acceptance.

The fetched archive is pinned by SHA256 and bounded before extraction. The
unchanged upstream `libwebrtc` license and `webrtc-sys` NOTICE are retained in
`licenses/` and must be copied into final packages.
