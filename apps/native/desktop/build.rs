fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=resources/caper.ico");
    // Task Manager shows FileDescription rather than the file name.
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("resources/caper.ico")
        .set("FileDescription", "Caper")
        .set("ProductName", "Caper")
        .compile()
        .expect("embed Caper executable icon");
}
