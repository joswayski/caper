# Native voice integration probe

This source tree pins the exact raw `libwebrtc` revision used by Caper. The
desktop app imports its media, gateway and state modules directly; the
standalone probe remains for low-level audio device module (ADM) checks. It
does not use LiveKit rooms or servers.

The modules provide:

- Native WASAPI/PulseAudio/ALSA ADM acquisition, microphone and speaker
  enumeration/selection, Opus publication and native playout.
- Account-channel join, publish, subscribe, negotiate, state, snapshot lease,
  close and leave against the Cloudflare SFU contract.
- Account credentials only in `Authorization` and the media capability only in
  `x-caper-media-token`; redirects disabled and no capability URLs.
- Roster/track reconciliation, local remote-track mute, mute/deafen with prior
  intent restored, synchronous local teardown, and setup rollback.
- Unified-gateway media snapshots with bounded cancellation, reconnect, terminal
  access denial and capability-safe framing. Gateway DNS/TCP/TLS/upgrade setup
  has a total five-second deadline; upgrade redirects are refused.
- TURN credential refresh followed by replay-safe ICE restart and ACK: a pending
  restart keeps its generation, sequence and SDP bytes, lost answers replay the
  same bytes, and failed ACKs retry without a new offer.
- Structured HTTP errors: terminal account or channel denial closes capture and
  playout immediately; a provider `ice_restart_retry` code stays retryable even
  with HTTP 403.
- Native connection diagnostics for bitrate, loss, jitter, RTT and relay route.

ICE gathering uses GatherOnce: the wrapper's default GatherContinually never
reaches Complete in Caper's non-trickle SDP setup.

## Vendored binding

The source under `vendor/` derives from `livekit/rust-sdks` commit
`5a656c4bc4bc5c4f7fad17ab69044db3b487dfeb` (`libwebrtc 0.3.49`, `webrtc-sys
0.3.46`, build helper `0.3.19`). It adds a narrow Rust/CXX binding for WebRTC's
existing pending `local_description()` getter, read immediately after
set-local to match the web's non-trickle Cloudflare flow (the current-only
getter is empty before the first answer and stale during ICE restart).

## Build

Linux x64:

```sh
WEBRTC="$(python3 fetch_libwebrtc.py --platform linux)"
CC=clang-21 CXX=clang++-21 LK_CUSTOM_WEBRTC="$WEBRTC" \
  CARGO_BUILD_JOBS=2 cargo run --locked
```

Build packages are `build-essential`, `pkg-config`, `libglib2.0-dev`,
`clang-21`, and `lld-21`. The archive's Chromium hermetic libc++ headers
require Clang. Runtime audio backends load dynamically and need `libpulse0`
and/or `libasound2`.

Windows x64, from a VS 2022 developer PowerShell:

```powershell
$WebRtc = python .\fetch_libwebrtc.py --platform windows
$env:LK_CUSTOM_WEBRTC = $WebRtc
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo run --locked --target x86_64-pc-windows-msvc
```

The probe prints device names locally; they must not be sent to Caper servers
or diagnostic logs. A machine without audio devices may report that ADM is
unavailable.

The fetched archive is pinned by SHA256 and bounded before extraction. The
unchanged upstream `libwebrtc` license and `webrtc-sys` NOTICE are kept in
`licenses/` and must be copied into final packages.
