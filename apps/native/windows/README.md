# Caper for Windows (native)

A native Windows client built on Win32, DirectComposition, Direct2D,
DirectWrite and UI Automation through the `windows` crate (0.62). It is the
Windows app: `apps/native/desktop/build.ps1` packages it as `Caper.exe` in
the installer and self-update archive, in the egui client's layout and with
its identity, so an egui install moves over on its next self-update and
stays signed in:

- the session is the same Credential Manager entry (`chat.caper.desktop`,
  per API origin) and launch at login the same `Run` value, both through
  the shared core;
- it registers itself for `caper://` links on launch and receives links
  from later launches through egui's handoff file
  (`%APPDATA%\Caper\data\link-handoff`), opening Join with invite link;
- it checks `caper-updater` beside it for signed releases; "Update
  available" above the account footer opens the review, and Restart to
  update hands over to the updater (`src/updates.rs`);
- it keeps egui's executable name, icon and (unset) AppUserModelID, so
  shortcuts, pins and notifications (`chat.caper.desktop`) stay the same;
- its saved audio devices and volumes start from egui's (`app.ron`).

Release builds talk to `https://caper.chat` unless `--api-url` or
`CAPER_API_URL` names another server, and open maximized. Development builds
without either show a labelled mock conversation (3000 generated messages,
48 members). Connected, it signs in (or restores the stored session) and
runs on the shared client core (`apps/native/client`):
the account's spaces and channels, DMs and message requests, live gateway
updates, replies, threads, reactions, edits and their history, pins, unread
state, invitations and invite links, space and channel settings, settings
and the notification inbox, system notifications (toasts), voice calls,
dictation and voice messages, KLIPY GIFs, and attachments.
Files attach from the composer's Attach files button (the system "Upload a
file" dialog), by dropping them on the window or by pasting copied files or
a picture; each uploads at once and the message waits for them. Clicking an
image or video opens the in-app media viewer with the rest of the message's
media (arrows, Left/Right or a touch swipe, with a swipe down to close;
wheel or double-click zoom, drag to pan; Open
in browser, Save, Copy image and Copy media link; videos play with
play/pause, seek and mute through the bundled FFmpeg). A link preview's
images and direct videos open there too. Its YouTube, Vimeo and Twitch
players play Caper's `play` page in WebView2 over the card, as the egui
client does (`src/web_player.rs`): one at a time, hidden with the poster
marked "Playing" while a menu, dialog or the viewer covers it or it is
scrolled partly out of view, and closed by another Play, a conversation
change or its card staying out of view. Without the WebView2 runtime, Play
opens the card's link in the browser. Debug builds load the `play` page from
`CAPER_DEV_PLAY_ORIGIN` (`http://127.0.0.1:PORT`), such as the parity
fixture's stand-in.

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
  matched per grapheme like the egui client, in messages, the composer and
  its placeholder, names, titles and avatar initials; any other colour-font glyph is
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
text, and selecting a range selects it in the history. A selection's
formatting buttons support Invoke and Toggle (on while the selection has
that style), and a message's hidden spoiler is a button named "Spoiler,
reveal".

## Build and run

Satoshi must be in `shared/fonts/cache` first (`python3 scripts/native_fonts.py`).

It is a member of the desktop workspace (`apps/native/desktop/Cargo.toml`),
which needs the same environment as `apps/native/desktop/build.ps1`. That
script (PowerShell 7 in an x64 Visual Studio developer environment) checks
and builds it into `dist/Caper-windows-x64-Setup.exe` and
`dist/Caper-windows-x64.zip`.

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
