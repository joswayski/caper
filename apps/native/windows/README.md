# Caper for Windows (native)

A native Windows client built on Win32, DirectComposition, Direct2D,
DirectWrite and UI Automation through the `windows` crate (0.62). It is not
shipped: it is outside the release workflow and the installers, and the egui
client (`apps/native/desktop`) stays the Windows app until this one reaches
feature, screenshot, performance and accessibility parity.

By default it shows a labelled development mock conversation (3000 generated
messages, 48 members). With `CAPER_API_URL` set it signs in (or restores the
stored session) and runs on the shared client core (`apps/native/client`):
the account's spaces and channels, DMs and message requests, live gateway
updates, replies, threads, reactions, edits and their history, pins, unread
state, invitations and invite links, space and channel settings, settings
and the notification inbox, system notifications (toasts), voice calls,
dictation and voice messages, KLIPY GIFs, and attachments.
Files attach from the composer's Attach files button (the system "Upload a
file" dialog), by dropping them on the window or by pasting copied files or
a picture; each uploads at once and the message waits for them. Clicking an
image or video opens the in-app media viewer with the rest of the message's
media (arrows or Left/Right; wheel or double-click zoom, drag to pan; Open
in browser and Copy image; videos play with play/pause, seek and mute
through the bundled FFmpeg). A link preview's images open there too; its
videos and players open in the browser.

## How it draws

- Each panel (space rail, channel sidebar, header, members, composer, hover
  toolbar) is a retained DirectComposition surface, redrawn only when its own
  state changes.
- The message history is one tall `IDCompositionVirtualSurface`. Scrolling
  moves the visual; only newly exposed strips are drawn, a quarter viewport
  ahead of each edge in half-viewport chunks, and far-off content is trimmed.
  Hover and selection redraw just the rows they touch.
- Rows are measured lazily from the newest message upward: startup lays out
  only the rows in the first viewport. A width change rewraps existing
  layouts (`SetMaxWidth`) instead of rebuilding them.
- Text goes through a custom `IDWriteTextRenderer`. Emoji in the Twemoji
  catalog (`shared/emoji`) are inline objects drawing their sheet cell,
  matched per grapheme like the egui client; any other colour-font glyph is
  rasterized once per face, glyph, size and scale and blitted from a cache.
- Frames are paced to the compositor clock and only produced when something
  changed.

## Accessibility

The UI Automation tree exposes the rail, channels, visible message rows,
members and the composer with names and roles. Spaces and channels support
Invoke and SelectionItem. The media viewer replaces the tree while open: its
window, title, image and buttons are named, and its buttons support Invoke.
The composer supports Value and Text/Text2 (value,
caret, selection, bounds); each message row supports Text over its plain
text, and selecting a range selects it in the history.

## Build and run

Satoshi must be in `shared/fonts/cache` first (`python3 scripts/native_fonts.py`).

It is a member of the desktop workspace (`apps/native/desktop/Cargo.toml`),
which needs the same environment as `apps/native/desktop/build.ps1`; the
desktop builds and the release workflow build only `caper-desktop`.

```powershell
cd apps/native/desktop
cargo build --package caper-windows --release
./target/release/caper-windows.exe
```

Check it with `cargo fmt --package caper-windows -- --check`,
`cargo clippy --package caper-windows --all-targets --no-deps -- -D warnings`
and `cargo test --package caper-windows`.

## Scripted scenarios

Setting `BENCH` to `startup`, `idle`, `scroll`, `burst`, `resize`, `typing`
or `hover` runs a scripted scenario after a 2 s settle and then quits,
appending one JSON line (CPU per frame, frame times, working set, private
bytes, first frame) to `BENCH_OUT`. `TSC_HZ` is the calibrated cycle rate
used to turn process cycles into CPU time. `CAPER_BENCH_FRAMES=<file>` logs
each frame's duration. `MESSAGES` sets the mock's size.
