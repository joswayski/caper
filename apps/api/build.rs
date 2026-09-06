fn main() {
    let migrations =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("migrations");
    std::fs::create_dir_all(&migrations).expect("create migrations directory");
    println!("cargo:rerun-if-changed=migrations");
}
