//! Interface zoom, as egui's Ctrl+= / Ctrl+− / Ctrl+0 on the desktop: GTK
//! has no window zoom, so the app stylesheet is reloaded with every pixel
//! length (type, spacing, corners, minimum sizes) scaled.

/// egui's steps and limits.
const STEP: f64 = 0.1;
const MIN: f64 = 0.2;
const MAX: f64 = 5.0;

pub fn zoom_in(factor: f64) -> f64 {
    round((factor + STEP).clamp(MIN, MAX))
}

pub fn zoom_out(factor: f64) -> f64 {
    round((factor - STEP).clamp(MIN, MAX))
}

/// A saved factor, kept within the limits.
pub fn valid(factor: f64) -> f64 {
    if factor.is_finite() {
        round(factor.clamp(MIN, MAX))
    } else {
        1.0
    }
}

fn round(factor: f64) -> f64 {
    (factor * 10.0).round() / 10.0
}

/// `css` with each `<number>px` multiplied by `factor`. Other numbers, such
/// as colours, alpha values, weights and percentages, are left alone.
pub fn scale_css(css: &str, factor: f64) -> String {
    if factor == 1.0 {
        return css.to_owned();
    }
    let bytes = css.as_bytes();
    let mut scaled = String::with_capacity(css.len());
    let mut index = 0;
    let mut copied = 0;
    while index < bytes.len() {
        let starts_number = bytes[index].is_ascii_digit()
            || (bytes[index] == b'.' && bytes.get(index + 1).is_some_and(u8::is_ascii_digit));
        // A number begins a token: not inside a name or a hex colour.
        let word = index > 0
            && (bytes[index - 1].is_ascii_alphanumeric()
                || bytes[index - 1] == b'#'
                || bytes[index - 1] == b'_'
                || bytes[index - 1] == b'-'
                    && index > 1
                    && bytes[index - 2].is_ascii_alphanumeric());
        if !starts_number || word {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
            index += 1;
        }
        if css[index..].starts_with("px")
            && !bytes
                .get(index + 2)
                .is_some_and(|next| next.is_ascii_alphanumeric())
            && let Ok(value) = css[start..index].parse::<f64>()
        {
            scaled.push_str(&css[copied..start]);
            let value = (value * factor * 100.0).round() / 100.0;
            scaled.push_str(&value.to_string());
            copied = index;
        }
    }
    scaled.push_str(&css[copied..]);
    scaled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_changes_pixel_lengths_only() {
        let css = ".a { font-size: 13px; padding: 4px 0 .5px; margin: -2px; color: #1c1f21; \
                   background: alpha(@caper_terracotta, 0.22); border-radius: 9999px; \
                   font-weight: 700; width: 50%; } .h2px { min-height: 1px; }";
        assert_eq!(
            scale_css(css, 1.5),
            ".a { font-size: 19.5px; padding: 6px 0 0.75px; margin: -3px; color: #1c1f21; \
             background: alpha(@caper_terracotta, 0.22); border-radius: 14998.5px; \
             font-weight: 700; width: 50%; } .h2px { min-height: 1.5px; }"
        );
        assert_eq!(scale_css(css, 1.0), css);
    }

    #[test]
    fn steps_follow_egui() {
        assert_eq!(zoom_in(1.0), 1.1);
        assert_eq!(zoom_out(1.0), 0.9);
        assert_eq!(zoom_in(5.0), 5.0);
        assert_eq!(zoom_out(0.2), 0.2);
        assert_eq!(valid(f64::NAN), 1.0);
        assert_eq!(valid(9.0), 5.0);
    }
}
