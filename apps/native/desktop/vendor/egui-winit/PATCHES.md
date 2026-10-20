# Caper patch to egui-winit 0.33.3

Copied from the published `egui-winit` 0.33.3 crate (upstream
[revision](https://github.com/emilk/egui/commit/44cdd653e2317d300fb8a6c9c36b03f23991e803)).
The MIT and Apache licenses are included from the `0.33.3` tag.

The only Rust change is in `State::on_keyboard_input`: emit `Event::Paste("")`
when the paste shortcut has no clipboard text. Caper's focused composer then
reads the native file/image formats. Previously the shortcut was swallowed and
our key-up fallback missed pastes when Ctrl was released before V. Nonempty text
still gets CRLF normalization; ordinary text fields treat an empty paste as a no-op.

Keep this patch when upgrading egui, or remove the override once upstream emits
paste intent for non-text clipboards. No Caper key-release fallback is needed.
Validate with a real X11/Wayland/Windows clipboard: GIF and binary-only content,
both modifier-release orders, ordinary text, and no duplicate attachment.
