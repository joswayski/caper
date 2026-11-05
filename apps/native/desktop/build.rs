fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=resources/caper.ico");
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("resources/caper.ico")
        .compile()
        .expect("embed Caper executable icon");
}
