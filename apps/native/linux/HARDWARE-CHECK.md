# Linux hardware check

`hardware-check.sh` covers what a headless sandbox can't: Wayland on GNOME and
KDE, GPU rendering, input methods, Orca, and real voice with a microphone and
speakers. It writes everything to one report, `~/caper-hardware-check-<time>.md`
(`REPORT` picks another path).

## Before you start

- A desktop session (Wayland or X11) on the machine itself, not SSH.
- A checkout of this repository with Node.js, which runs the local fixture.
- Release binaries, one of three ways:
  - Built from source (the default): Rust, `libgtk-4-dev`, Clang (export
    `CC=clang CXX=clang++` if libwebrtc fails to build with GCC) and Python 3.
    Set `SKIP_BUILD=1` if the binaries in `apps/native/desktop/target/release`
    are current.
  - `FROM_CI=1`: downloads the newest nightly `caper-linux` tester tarball and
    the egui desktop's development build with the GitHub CLI (run
    `gh auth login` first). Only GTK 4.8 or later is needed then.
  - `CAPER_LINUX_BUILD=<path>/caper-linux-x64.tar.gz`: a tester tarball you
    already downloaded. The egui comparison then uses an installed
    `caper-desktop` if there is one, and its benchmarks are skipped otherwise.
- For the IME questions, an input method set up (ibus or fcitx5 with, for
  example, Mozc or Pinyin). For the Orca questions, `orca` installed. For
  voice, your caper.chat account signed in on Linux and a second device to
  call.
- Optional: `glxinfo` (mesa-utils) and `vulkaninfo` (vulkan-tools) so the
  report names the GPU driver.

## Run it

```sh
apps/native/linux/hardware-check.sh
FROM_CI=1 apps/native/linux/hardware-check.sh   # nightly builds, no compiling
```

1. It records the machine (session, desktop, GTK, GPU, IME, audio server).
2. It starts the local fixture with 3,000 messages and opens each client once.
   If one shows sign-in, use `fixture@example.test` and code `ABC234`. The
   fixture account uses its own profile, and your caper.chat session isn't
   touched.
3. Performance: don't touch the mouse or keyboard. Every scenario (startup,
   idle, scroll, typing) runs three times on each GTK renderer (`gl`, `ngl`,
   `vulkan`, `cairo`), on Wayland and on XWayland, and on the egui desktop
   for comparison. Each run opens and closes Caper by itself. This takes about
   20 minutes. `RUNS=1` makes it shorter.
4. Manual checks: answer `y`, `n` or `s` (skip) to each question, with an
   optional note. They cover display and scaling, clipboard, drag and drop,
   notifications, IME, hand-typed formatting markers, drafts, Orca, and a real
   voice call (Audio test, call quality, mute and deafen, device switching,
   reconnecting, voice messages).

Run it once in a GNOME session and once in a KDE Plasma session (Wayland for
both), then send both reports.
