# Shared mobile logic

`caper-mobile` gives the Android and Apple apps the same Rust logic the
desktop clients and the API compile, while their UI stays native (Jetpack
Compose; SwiftUI, UIKit and AppKit through CaperCore). It has its own
workspace, so mobile builds never compile the desktop client's media stack.

| Module | Sources | Android | Apple |
| --- | --- | --- | --- |
| Bold/italic formatting | `shared/messages/formatting.rs` | `data/Formatting.kt` | |
| Links | `shared/messages/links.rs` | `data/Links.kt` | |
| Mention tokens | `shared/messages/mentions.rs` | `MentionAutocomplete.kt` (`mentionSpans`) | |
| Name characters | `apps/native/client/src/names.rs` | `model/Names.kt` | |

The host code keeps its own types and editing helpers and calls Rust for the
parse. Results are flat `i32` arrays of offsets into the caller's string
(UTF-16 code units, plus UTF-8 bytes for Swift), so text never round-trips
through a second encoding; `src/lib.rs` documents each layout.

- **Apple:** the C ABI in `include/caper_mobile.h` (module `CaperMobile`),
  built by `build-apple.sh` into a static `CaperMobile.xcframework` with iOS
  device, iOS simulator (arm64, x86_64) and macOS (arm64, x86_64) slices.
- **Android:** JNI functions for `chat.caper.android.data.CaperMobile`
  (feature `jni`). The app's Gradle build runs cargo-ndk for arm64-v8a,
  armeabi-v7a, x86 and x86_64 (16 KiB-aligned on 64-bit ABIs) and builds the
  library for the host so JVM unit tests run the same code.

Each client's tests run the shared case files (`shared/messages/*-cases.json`,
`shared/names/cases.json`) through its bridge. Here:

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets --features jni -- -D warnings
cargo test --locked --features jni
```
