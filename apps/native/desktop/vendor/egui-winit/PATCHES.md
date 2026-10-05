# Caper's egui-winit patch

This is the crates.io `egui-winit` 0.33.3 source, from upstream commit
`44cdd653e2317d300fb8a6c9c36b03f23991e803` in `emilk/egui`.
The normalized crate manifest is retained; license paths now refer to the
unchanged upstream licenses copied into this directory.

The only production source change is in `ViewportCommand::Icon`: on Windows,
call winit's safe `WindowExtWindows::set_taskbar_icon` as well as
`Window::set_window_icon`. Upstream only updates `ICON_SMALL`, leaving the
taskbar/Alt-Tab `ICON_BIG` stale. winit owns both icon handles and their lifetimes;
the app adds no unsafe Win32 calls. Non-Windows behavior is unchanged.

The desktop manifest patches this exact version through `[patch.crates-io]`.
Remove the patch when upstream updates both icon surfaces. Re-check the startup
icon helper as well when upgrading eframe: it can override the first dynamic icon
on the first frame. Caper applies its persisted daily icon in the first App update,
after eframe's `pre_update` startup helper. The application has no unsafe FFI.
