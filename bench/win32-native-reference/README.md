# Win32 native reference (benchmark build)

A benchmark recreation of Caper's conversation screen, built directly on Win32,
DirectComposition, Direct2D, DirectWrite and UI Automation. It is a reference for
the native Windows clients, not a product: it runs only against a generated
in-process fixture, has no network code, and is not part of any build or release.

## Layout

- `native-chat/`: the app (Rust, `windows` 0.62.2, `windows-core` 0.62,
  `windows-numerics` 0.3; features listed in its `Cargo.toml`).
  - `gfx.rs`: D3D11 device, D2D device context, DirectComposition desktop device,
    DirectWrite factory with Satoshi loaded into an in-memory font collection,
    brushes, text formats; a custom `IDWriteTextRenderer` and the colour-emoji
    bitmap cache (each colour glyph rasterized once per size and scale).
  - `main.rs`: window, message loop, one retained composition surface per panel,
    the message list as one tall virtual surface moved by visual offset, hover
    toolbar layer, text selection, clipboard, IME, compositor-clock pacing.
  - `rows.rs`: per-message DirectWrite layouts measured on demand.
  - `composer.rs`: UTF-16 buffer, caret/anchor selection, IME composition.
  - `uia.rs`: UI Automation fragment tree, Invoke, Value, Text and Text2 for the
    composer.
  - `bench_probe.rs`: optional per-frame timing lines (`CAPER_BENCH_FRAMES`).
- `fixture/`: deterministic chat data and the scripted scenarios (`BENCH=...`);
  writes one JSON line per run with cycle-based CPU, frames, private bytes and
  working set.
- `scripts/`: PowerShell drivers.

## Build

The Satoshi fonts are not in the repository. Fetch them first with
`python3 scripts/native_fonts.py` from the repository root (see
`shared/fonts/README.md`). Then:

```powershell
cd bench\win32-native-reference\native-chat
cargo build --release
```

## Run and measure

```powershell
$env:MESSAGES = "3000"   # the fixture size used in the reports
powershell -ExecutionPolicy Bypass -File scripts\bench-ui.ps1 -Out out\run.jsonl -Repeat 3
python -I scripts\summarize-native.py out\run.jsonl
```

`bench-ui.ps1` launches the app once per scenario (startup, idle, scroll, burst,
resize, typing, hover), places the window, sweeps the real cursor for hover, and
samples the process's `GPU Engine` and `GPU Process Memory` counters into
`out\run-gpu.jsonl`. `lib.ps1` calibrates the TSC and passes it as `TSC_HZ`;
CPU comes from `QueryProcessCycleTime`. Running the app with no `BENCH` variable
gives an interactive window. `uia-dump.ps1 -ProcessId N -Out f.txt` dumps the
UIA tree; `uia-native.ps1` checks the composer's value, caret and selection.
Hover moves the real cursor; keep the machine idle while measuring.

## Things to know

- DPI: per-monitor v2 awareness. Layout is in DIPs; surfaces are allocated in
  physical pixels and drawn with a scale transform. Scroll offsets are snapped to
  device pixels so the composition offset and drawn rows agree. `WM_DPICHANGED`
  rebuilds every surface.
- The message list keeps a quarter viewport drawn beyond each edge and draws
  ahead in half-viewport chunks, trimming the virtual surface when it grows
  past six viewports. After a rewrap only the viewport is drawn that frame.
- Resize keeps existing text layouts and calls `SetMaxWidth` instead of
  rebuilding them.
- `WM_SYSCOMMAND`/`SC_KEYMENU` is swallowed so an Alt tap (used by the drivers to
  take the foreground) does not enter menu mode.
- Benchmark-only: the fixture, `bench_probe.rs`, the `NATIVE_PHASES` timers in
  `main.rs`, and the scripted scenarios.
- Known gaps: message rows expose only a name to UIA (no Text pattern); emoji are
  drawn from the system colour font, not Caper's Twemoji sheets; no animation,
  RTL caret editing, scrollbar or focus ring; the image bitmap cache is unbounded;
  IME code has not been tested with a real IME.
