//! Satoshi for Pango, through fontconfig.
//!
//! GTK 4.8's Pango (1.50) cannot add a font file to its font map, and this
//! crate forbids the unsafe fontconfig calls that could. Instead the bundled
//! fonts are written to the cache directory with a fontconfig file that
//! includes the system configuration plus that directory, and the process
//! re-executes itself once with `FONTCONFIG_FILE` pointing at it, before GTK
//! or any thread starts.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

const FONTS: [(&str, &[u8]); 4] = [
    (
        "Satoshi-Regular.otf",
        include_bytes!("../../../../shared/fonts/cache/Satoshi-Regular.otf"),
    ),
    (
        "Satoshi-Medium.otf",
        include_bytes!("../../../../shared/fonts/cache/Satoshi-Medium.otf"),
    ),
    (
        "Satoshi-Bold.otf",
        include_bytes!("../../../../shared/fonts/cache/Satoshi-Bold.otf"),
    ),
    (
        "Satoshi-Black.otf",
        include_bytes!("../../../../shared/fonts/cache/Satoshi-Black.otf"),
    ),
];

/// Set on the re-executed process.
const PREPARED: &str = "CAPER_LINUX_FONTS";

/// Returns only when the fonts are configured, or when they cannot be (the
/// client then falls back to the system's sans-serif).
pub fn prepare() {
    if std::env::var_os(PREPARED).is_some() {
        return;
    }
    let Some(directory) = directory() else {
        return;
    };
    let configuration = match install(&directory) {
        Ok(configuration) => configuration,
        Err(error) => {
            eprintln!("Satoshi is unavailable: {error}");
            return;
        }
    };
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    let error = std::process::Command::new(program)
        .args(std::env::args_os().skip(1))
        .env(PREPARED, "1")
        .env("FONTCONFIG_FILE", configuration)
        .exec();
    eprintln!("Satoshi is unavailable: {error}");
}

fn directory() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))?;
    Some(cache.join("caper-linux").join("fonts"))
}

fn install(directory: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    for (name, bytes) in FONTS {
        let path = directory.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(bytes) {
            std::fs::write(&path, bytes)?;
        }
    }
    let system =
        std::env::var("FONTCONFIG_FILE").unwrap_or_else(|_| "/etc/fonts/fonts.conf".into());
    let configuration = directory.join("fonts.conf");
    let text = format!(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n<fontconfig>\n  <include ignore_missing=\"yes\">{}</include>\n  <dir>{}</dir>\n</fontconfig>\n",
        escape(&system),
        escape(&directory.to_string_lossy()),
    );
    if std::fs::read_to_string(&configuration).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&configuration, text)?;
    }
    Ok(configuration)
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
