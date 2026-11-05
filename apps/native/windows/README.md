# Caper for Windows (native)

A native Windows client built on Win32, DirectComposition, Direct2D,
DirectWrite and UI Automation through the `windows` crate (0.62). It is not
shipped: it is outside the release workflow and the installers, and the egui
client (`apps/native/desktop`) stays the Windows app until this one reaches
feature, screenshot, performance and accessibility parity.

It currently shows a labelled development mock conversation (3000 generated
messages, 48 members). The shared client core (`apps/native/client`: API,
gateway, model, worker, uploads) replaces the mock once it lands.

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
Invoke and SelectionItem. The composer supports Value and Text/Text2 (value,
caret, selection, bounds); each message row supports Text over its plain
text, and selecting a range selects it in the history.

## Build and run

Satoshi must be in `shared/fonts/cache` first (`python3 scripts/native_fonts.py`).

```powershell
cd apps/native/windows
cargo build --release
./target/release/caper-windows.exe
```

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
`cargo test` run from this directory; the crate is its own workspace.

## Scripted scenarios

Setting `BENCH` to `startup`, `idle`, `scroll`, `burst`, `resize`, `typing`
or `hover` runs a scripted scenario after a 2 s settle and then quits,
appending one JSON line (CPU per frame, frame times, working set, private
bytes, first frame) to `BENCH_OUT`. `TSC_HZ` is the calibrated cycle rate
used to turn process cycles into CPU time. `CAPER_BENCH_FRAMES=<file>` logs
each frame's duration. `MESSAGES` sets the mock's size.
