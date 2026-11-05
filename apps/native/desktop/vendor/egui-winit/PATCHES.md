# Caper patch to egui-winit 0.37.0

Copied from the published `egui-winit` 0.37.0 crate (upstream
[revision](https://github.com/emilk/egui/commit/7c0b40402271e8d40cda69f87cf3c2617c2e4382)).
The MIT and Apache licenses are included from the `0.37.0` tag.

The only Rust change is in `State::on_keyboard_input`: emit `Event::Paste("")`
when the paste shortcut has no clipboard text, instead of swallowing it or
decoding an image-only clipboard into `Event::PasteImage`. Caper's focused
composer then reads the native file/image clipboard formats, keeping encoded
bytes such as animated GIFs. Nonempty text still gets CRLF normalization;
ordinary text fields treat an empty paste as a no-op.

Keep this patch when upgrading egui, or remove the override once upstream emits
paste intent for non-text clipboards.
