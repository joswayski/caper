# Third-party software

Caper Desktop is distributed with Rust dependencies listed exactly in `Cargo.lock`.
Their package metadata and license identifiers are available from crates.io and
the corresponding source repositories. The application does not bundle a browser,
WebView, or JavaScript runtime. It statically links the verified Google WebRTC
archive distributed by LiveKit, without using LiveKit rooms or servers. The
unchanged `libwebrtc-LICENSE.md` and `webrtc-sys-NOTICE.md` accompany both
desktop packages.

Principal UI/network dependencies are eframe/egui/egui_extras (MIT OR Apache-2.0), wgpu
(MIT OR Apache-2.0), reqwest (MIT OR Apache-2.0), tungstenite (MIT OR
Apache-2.0), and keyring (MIT OR Apache-2.0).

## Native audio

The bundled DPDFNet-8 HR model is the same Apache-2.0 asset used by the web
client (`DPDFNET-LICENSE`). Native ONNX Runtime 1.23.2 uses MIT; packages carry
`ONNX-RUNTIME-LICENSE` and `ONNX-RUNTIME-THIRD-PARTY-NOTICES.txt`. The `ort` Rust
wrapper is MIT/Apache-2.0. The pure-Rust RNNoise fallback, nnnoiseless 0.5.2, uses
BSD-3-Clause (`nnnoiseless-COPYING.md`) and includes rustfft
(MIT OR Apache-2.0). Native effects playback uses rodio and CPAL
(MIT OR Apache-2.0) with the existing Caper WAV assets.

Windows packages include Microsoft-signed app-local VC++ runtime DLLs from the
Visual Studio build toolchain under Microsoft's redistributable terms. These
are not part of Rust's statically linked runtime or covered by Caper's license.

## Lucide icons

The bundled vectors are exported from the web client's pinned lucide-react
0.468.0 package by `scripts/native-icons.mjs`. They render locally through egui's
SVG loader; no browser or JavaScript runtime is used by the application.

ISC License

Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part of Feather (MIT). All other copyright (c) for Lucide are held by Lucide Contributors 2022.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
