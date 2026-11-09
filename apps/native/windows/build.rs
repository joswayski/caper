fn main() {
    // Caper.exe's icon (Explorer, the Start menu shortcut, the taskbar and
    // the window), the same file the egui client embeds.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../desktop/resources/caper.ico");
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("../desktop/resources/caper.ico")
        .compile()
        .expect("embed Caper executable icon");
}
