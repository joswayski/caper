fn main() {
    // Caper.exe's icon (Explorer, the Start menu shortcut, the taskbar and
    // the window), the same file the egui client embeds, and its name: Task
    // Manager shows FileDescription, not the window title or the file name.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../desktop/resources/caper.ico");
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("../desktop/resources/caper.ico")
        .set("FileDescription", "Caper")
        .set("ProductName", "Caper")
        .compile()
        .expect("embed Caper executable icon");
}
