#![deny(unsafe_code)]

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to run Caper");
}
