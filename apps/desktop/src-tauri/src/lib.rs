#![deny(unsafe_code)]

mod updates;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            updates::initialize(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Caper");
}
