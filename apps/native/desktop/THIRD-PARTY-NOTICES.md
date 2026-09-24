# Third-party software

Caper Desktop is distributed with Rust dependencies listed exactly in `Cargo.lock`.
Their package metadata and license identifiers are available from crates.io and
the corresponding source repositories. The application does not bundle a browser,
WebView, or JavaScript runtime. It statically links the verified Google WebRTC
archive distributed by LiveKit, without using LiveKit rooms or servers. The
unchanged `libwebrtc-LICENSE.md` and `webrtc-sys-NOTICE.md` accompany both
desktop packages.

Principal UI/network dependencies are eframe/egui (MIT OR Apache-2.0), wgpu
(MIT OR Apache-2.0), reqwest (MIT OR Apache-2.0), tungstenite (MIT OR
Apache-2.0), and keyring (MIT OR Apache-2.0).
