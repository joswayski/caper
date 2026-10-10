# Third-party software

## Twemoji / emoji picker data

The embedded emoji sprite sheets use Twemoji graphics © Twitter, Inc. and other
contributors under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/).
Picker names and keywords are derived from emoji-picker-react under the MIT
license. Full source notices are maintained in `shared/emoji/NOTICE.txt`,
`shared/emoji/PACKAGE-LICENSE.txt`, and `shared/emoji/PICKER-LICENSE.txt`.

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

## Attachments

Attachment images are decoded with the pure-Rust `image` crate codecs
(`png`, `zune-jpeg`, `image-webp`, `gif`, `tiff`, BMP; MIT OR Apache-2.0,
zune-jpeg also Zlib) and encoded with `png` (MIT OR Apache-2.0),
`jpeg-encoder` ((MIT OR Apache-2.0) AND IJG: this software is based in part
on the work of the Independent JPEG Group), WebP through
[libwebp](https://chromium.googlesource.com/webm/libwebp) 1.6.0
(BSD-3-Clause, Copyright (c) 2010, Google Inc.; with Google's additional
WebM patent grant), compiled from the source vendored in `libwebp-sys` 0.14.4
(MIT) and called through `webpx` 0.4.0 (MIT OR Apache-2.0), and AVIF through
[rav1e](https://github.com/xiph/rav1e) 0.8.1 (BSD-2-Clause, Copyright (c)
2017-2023, the rav1e contributors; with the Alliance for Open Media Patent
License 1.0) and `avif-serialize` 0.8.9 (BSD-3-Clause, Copyright (c) 2020,
Cloudflare, Inc.). Their full licence texts ship beside the app as
`libwebp-LICENSE.txt`, `rav1e-LICENSE.txt` and `avif-serialize-LICENSE.txt`. The
exact-palette indexed PNG writer and JPEG settings are adapted from Caper's
sibling project Captures (`joswayski/captures`, Apache-2.0). The file dialog
uses `rfd` (MIT).

## FFmpeg (video compression, playback and AVIF/HEIC decoding)

Desktop packages include a separate `ffmpeg` executable: a static, trimmed
[FFmpeg](https://ffmpeg.org) 9.0.2 with [x264](https://code.videolan.org/videolan/x264),
[dav1d](https://code.videolan.org/videolan/dav1d) 1.5.4 and
[zimg](https://github.com/sekrit-twc/zimg) 3.0.6, built from pinned sources by
`joswayski/ffmpeg-desktop`. Because it includes x264 it is licensed under the
GNU GPL version 2 or later; Caper runs it as a separate program over pipes and
does not link it. Its licence texts (`FFmpeg-COPYING.GPLv2`,
`FFmpeg-LICENSE.md`, `x264-COPYING`, `dav1d-COPYING` (BSD-2-Clause),
`zimg-COPYING` (WTFPL)), the exact source links and the build recipe ship
beside it in `ffmpeg-licenses/`. On Windows it may use the operating system's
Media Foundation H.264 encoder.

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

## Invitation artwork

Twemoji graphics © Twitter, Inc. and other contributors, licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
Sources: https://github.com/twitter/twemoji and https://github.com/jdecked/twemoji.
The incoming-envelope illustration is Twemoji 15.0.0 from `@twemoji/svg` 15.0.0,
rasterized into a sprite and cropped without changing its pixels (sheet 6,
x=832, y=832, 64×64) from
[Caper's published emoji assets](https://github.com/joswayski/caper/commit/54910cbf71cb84b8e1419972b837ec6723f68757).
Desktop embeds the canonical `apps/web/public/images/invitation/1f4e8.png`
directly; it does not rely on a system emoji font.
