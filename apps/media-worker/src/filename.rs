//! Download names: sanitised like the API, with the extension matching the
//! stored type, and the `Content-Disposition` header R2 stores.

use std::fmt::Write as _;

const MAX_CHARS: usize = 255;

/// Mirrors the API's `filename()` rules, repairing instead of rejecting: no
/// path components, no control characters, at most 255 characters.
pub fn sanitize(value: &str) -> String {
    let name = value.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let name: String = name
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .take(MAX_CHARS)
        .collect();
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        "file".to_owned()
    } else {
        name.to_owned()
    }
}

/// Extensions accepted for a stored content type; the first is canonical.
pub fn extensions(content_type: &str) -> &'static [&'static str] {
    match content_type {
        "image/webp" => &["webp"],
        "image/avif" => &["avif"],
        "image/jpeg" => &["jpg", "jpeg", "jpe", "jfif"],
        "image/png" => &["png"],
        "video/mp4" => &["mp4", "m4v"],
        "audio/flac" => &["flac"],
        "audio/mpeg" => &["mp3"],
        "audio/mp4" => &["m4a", "mp4", "m4b", "aac"],
        "audio/ogg" => &["ogg", "oga", "opus"],
        "audio/webm" => &["webm", "weba"],
        "audio/wav" => &["wav", "wave"],
        "audio/aiff" => &["aiff", "aif", "aifc"],
        "audio/aac" => &["aac"],
        "audio/amr" => &["amr"],
        "audio/x-caf" => &["caf"],
        _ => &[],
    }
}

/// Keeps the user's base name and makes the extension match the stored
/// type: `photo.HEIC` → `photo.avif`, `clip.gif` → `clip.mp4`. Names that
/// already carry an accepted extension (any case) are kept as they are.
pub fn with_extension(name: &str, content_type: &str) -> String {
    let name = sanitize(name);
    let accepted = extensions(content_type);
    let Some(canonical) = accepted.first() else {
        return name;
    };
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], Some(&name[i + 1..])),
        _ => (name.as_str(), None),
    };
    if ext.is_some_and(|ext| accepted.iter().any(|a| a.eq_ignore_ascii_case(ext))) {
        return name;
    }
    let budget = MAX_CHARS - canonical.len() - 1;
    let stem: String = stem.chars().take(budget).collect();
    let stem = stem.trim_end();
    let stem = if stem.is_empty() { "file" } else { stem };
    format!("{stem}.{canonical}")
}

fn uri_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// RFC 6266 download name, identical to the API's `disposition()`.
pub fn disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| match c {
            ' '..='~' if c != '"' && c != '\\' => c,
            _ => '_',
        })
        .collect();
    format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{}",
        uri_encode(name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_mapping() {
        assert_eq!(with_extension("photo.HEIC", "image/avif"), "photo.avif");
        assert_eq!(with_extension("clip.gif", "video/mp4"), "clip.mp4");
        assert_eq!(with_extension("notes.wav", "audio/flac"), "notes.flac");
        assert_eq!(with_extension("shot.png", "image/webp"), "shot.webp");
        assert_eq!(with_extension("IMG_1.JPG", "image/jpeg"), "IMG_1.JPG");
        assert_eq!(with_extension("scan.jpeg", "image/jpeg"), "scan.jpeg");
        assert_eq!(
            with_extension("mislabeled.png", "image/jpeg"),
            "mislabeled.jpg"
        );
        assert_eq!(with_extension("movie.MOV", "video/mp4"), "movie.mp4");
        assert_eq!(
            with_extension("no extension", "image/avif"),
            "no extension.avif"
        );
        assert_eq!(with_extension(".hidden", "image/webp"), ".hidden.webp");
        assert_eq!(
            with_extension("archive.tar.gz", "image/webp"),
            "archive.tar.webp"
        );
        assert_eq!(
            with_extension("report.pdf", "application/pdf"),
            "report.pdf"
        );
        assert_eq!(with_extension("dir/sub\\x.bmp", "image/webp"), "x.webp");
    }

    #[test]
    fn long_names_stay_within_255_characters() {
        let long = format!("{}.heic", "é".repeat(300));
        let out = with_extension(&long, "image/avif");
        assert_eq!(out.chars().count(), 255);
        assert!(out.ends_with(".avif"));
        assert_eq!(sanitize(&"x".repeat(400)).chars().count(), 255);
    }

    #[test]
    fn sanitizing() {
        assert_eq!(sanitize("../../etc/passwd"), "passwd");
        assert_eq!(sanitize("a\u{0}b\nc"), "a_b_c");
        assert_eq!(sanitize("  spaced  "), "spaced");
        assert_eq!(sanitize(".."), "file");
        assert_eq!(sanitize("dir/"), "file");
        assert_eq!(sanitize(""), "file");
    }

    #[test]
    fn disposition_matches_the_api() {
        assert_eq!(
            disposition("café \"x\".avif"),
            "attachment; filename=\"caf_ _x_.avif\"; filename*=UTF-8''caf%C3%A9%20%22x%22.avif"
        );
        assert_eq!(
            disposition("a/b.txt"),
            "attachment; filename=\"a/b.txt\"; filename*=UTF-8''a%2Fb.txt"
        );
    }
}
