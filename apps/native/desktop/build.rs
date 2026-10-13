fn main() {
    println!("cargo:rerun-if-env-changed=CAPER_RELEASE_SHA");
    println!("cargo:rerun-if-changed=build.rs");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .output()
            .expect("git is required unless CAPER_RELEASE_SHA is set");
        assert!(output.status.success(), "could not resolve source revision");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let revision = match std::env::var("CAPER_RELEASE_SHA") {
        Ok(revision) => revision,
        Err(std::env::VarError::NotPresent) => {
            // Rebuild when HEAD moves, including commits on the same branch.
            for name in ["HEAD", "packed-refs"] {
                println!(
                    "cargo:rerun-if-changed={}",
                    git(&["rev-parse", "--git-path", name])
                );
            }
            let branch = std::process::Command::new("git")
                .args(["symbolic-ref", "-q", "HEAD"])
                .output()
                .unwrap();
            if branch.status.success() {
                let branch = String::from_utf8(branch.stdout).unwrap();
                println!(
                    "cargo:rerun-if-changed={}",
                    git(&["rev-parse", "--git-path", branch.trim()])
                );
            }
            git(&["rev-parse", "HEAD"])
        }
        Err(_) => panic!("CAPER_RELEASE_SHA must be valid Unicode"),
    };
    assert!(
        revision.len() == 40
            && revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "CAPER_RELEASE_SHA/source revision must be 40 lowercase hexadecimal characters"
    );
    println!("cargo:rustc-env=CAPER_SOURCE_REVISION={revision}");

    println!("cargo:rerun-if-changed=resources/caper.ico");
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("resources/caper.ico")
        .compile()
        .expect("embed Caper executable icon");
}
