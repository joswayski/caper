fn main() {
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("resources/caper.ico")
        .compile()
        .expect("embed Caper executable icon");
}
