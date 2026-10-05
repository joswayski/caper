//! Transparent daily artwork for native window icons and Linux launchers.
use crate::avatar_images;
use eframe::egui;

const SIDE: usize = 512;

pub fn apply(context: &egui::Context, index: usize) {
    let svg = fitted_svg(avatar_images::BRANDING[index]).expect("bundled character is not blank");
    context.send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(
        icon_data(&svg),
    ))));
    #[cfg(target_os = "linux")]
    if linux::publish(&svg).is_err() {
        eprintln!("Caper could not update its per-user launcher icon; window icon still applied");
    }
}

fn render(svg: &[u8]) -> egui::ColorImage {
    egui_extras::image::load_svg_bytes_with_size(
        svg,
        egui::SizeHint::Width(SIDE as u32),
        &Default::default(),
    )
    .expect("bundled Caper character is valid SVG")
}

fn fitted_svg(svg: &[u8]) -> Option<String> {
    // Use the renderer's vector bounds: some accessories extend just outside
    // the original canvas, so measuring a clipped raster loses those edges.
    let tree = usvg::Tree::from_data(svg, &Default::default())
        .expect("bundled Caper character is valid SVG");
    let bounds = tree.root().abs_stroke_bounding_box();
    let longest = bounds.width().max(bounds.height());
    if longest == 0.0 {
        return None;
    }
    // Longest visible dimension fills 480px, with 16px clear on either side.
    let side = f64::from(longest) * SIDE as f64 / 480.0;
    let x = f64::from(bounds.x()) + f64::from(bounds.width()) / 2.0 - side / 2.0;
    let y = f64::from(bounds.y()) + f64::from(bounds.height()) / 2.0 - side / 2.0;
    let source = std::str::from_utf8(svg).expect("bundled SVG is UTF-8");
    assert!(source.contains("viewBox=\"0 0 256 256\""));
    Some(source.replacen(
        "viewBox=\"0 0 256 256\"",
        &format!("viewBox=\"{x} {y} {side} {side}\""),
        1,
    ))
}

fn icon_data(svg: &str) -> egui::IconData {
    let image = render(svg.as_bytes());
    egui::IconData {
        rgba: image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_srgba_unmultiplied())
            .collect(),
        width: image.width() as u32,
        height: image.height() as u32,
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        fs, io,
        path::{Path, PathBuf},
    };

    const MARKER: &str = "<!-- Caper daily launcher icon -->";

    fn data_home(xdg: Option<PathBuf>, home: Option<PathBuf>) -> io::Result<PathBuf> {
        xdg.filter(|path| path.is_absolute())
            .or_else(|| {
                home.filter(|path| path.is_absolute())
                    .map(|path| path.join(".local/share"))
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No user data directory"))
    }

    pub fn publish(svg: &str) -> io::Result<()> {
        let data = data_home(
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        )?;
        write(&data, svg)
    }

    fn write(data: &Path, svg: &str) -> io::Result<()> {
        // A user-scoped theme override; never edit the installed package or
        // require root. Matching Icon=caper and Wayland app_id=caper is essential.
        let theme = data.join("icons/hicolor");
        let directory = theme.join("scalable/apps");
        fs::create_dir_all(&directory)?;
        let destination = directory.join("caper.svg");
        let svg = format!("{MARKER}\n{svg}");
        match fs::read(&destination) {
            Ok(previous) if previous == svg.as_bytes() => return Ok(()),
            Ok(previous) if !previous.starts_with(MARKER.as_bytes()) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "Existing custom icon",
                ));
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        let temporary = directory.join(format!(".caper-{}.svg", uuid::Uuid::new_v4()));
        let result = fs::write(&temporary, svg).and_then(|()| fs::rename(&temporary, &destination));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        // Icon-theme caches use the theme directory's modification time.
        // Refreshing an already displayed launcher remains shell-dependent.
        if let Ok(directory) = fs::File::open(theme) {
            let _ = directory.set_modified(std::time::SystemTime::now());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn uses_only_absolute_xdg_or_home_paths() {
            assert_eq!(
                data_home(Some("/xdg".into()), Some("/home/test".into())).unwrap(),
                PathBuf::from("/xdg")
            );
            for xdg in [None, Some("relative".into()), Some("".into())] {
                assert_eq!(
                    data_home(xdg, Some("/home/test".into())).unwrap(),
                    PathBuf::from("/home/test/.local/share")
                );
            }
            assert!(data_home(Some("relative".into()), None).is_err());
            assert!(data_home(None, Some("relative".into())).is_err());
        }

        #[test]
        fn replaces_only_the_user_icon_and_retains_it_on_failure() {
            let data =
                std::env::temp_dir().join(format!("caper-icon-test-{}", uuid::Uuid::new_v4()));
            let destination = data.join("icons/hicolor/scalable/apps/caper.svg");
            write(&data, "first").unwrap();
            assert_eq!(
                fs::read_to_string(&destination).unwrap(),
                "<!-- Caper daily launcher icon -->\nfirst"
            );
            let modified = fs::metadata(&destination).unwrap().modified().unwrap();
            write(&data, "first").unwrap();
            assert_eq!(
                fs::metadata(&destination).unwrap().modified().unwrap(),
                modified
            );
            write(&data, "second").unwrap();
            assert_eq!(
                fs::read_to_string(&destination).unwrap(),
                "<!-- Caper daily launcher icon -->\nsecond"
            );
            assert_eq!(
                fs::read_dir(destination.parent().unwrap()).unwrap().count(),
                1
            );
            fs::write(&destination, "user-supplied icon").unwrap();
            assert_eq!(
                write(&data, "third").unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(
                fs::read_to_string(&destination).unwrap(),
                "user-supplied icon"
            );
            // A blocked destination also leaves no partial files behind.
            fs::remove_file(&destination).unwrap();
            fs::create_dir(&destination).unwrap();
            assert!(write(&data, "third").is_err());
            assert!(destination.is_dir());
            assert_eq!(
                fs::read_dir(destination.parent().unwrap()).unwrap().count(),
                1
            );
            fs::remove_dir_all(data).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(icon: &egui::IconData) -> (usize, usize, usize, usize) {
        let (mut left, mut top, mut right, mut bottom) = (SIDE, SIDE, 0, 0);
        for (offset, pixel) in icon.rgba.chunks_exact(4).enumerate() {
            if pixel[3] > 0 {
                let (x, y) = (offset % SIDE, offset / SIDE);
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
        (left, top, right, bottom)
    }

    #[test]
    fn all_daily_characters_are_transparent_centered_and_not_clipped() {
        for (index, source) in avatar_images::BRANDING.iter().enumerate() {
            let icon = icon_data(&fitted_svg(source).unwrap());
            assert_eq!((icon.width, icon.height), (512, 512));
            let (left, top, right, bottom) = bounds(&icon);
            assert!(
                (477..=482).contains(&(right - left).max(bottom - top)),
                "bad fit {index}"
            );
            assert!((left + right).abs_diff(SIDE) <= 3, "off-center X {index}");
            assert!((top + bottom).abs_diff(SIDE) <= 3, "off-center Y {index}");
            assert!(
                left >= 15 && top >= 15 && right <= 497 && bottom <= 497,
                "clipped {index}: {left}, {top}, {right}, {bottom}"
            );
            assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] == 255));
            if index == 0 {
                assert_eq!(icon.rgba[(144 * SIDE + 440) * 4 + 3], 0, "old tile remains");
            }
        }
    }

    #[test]
    fn preserves_asymmetric_geometry_colors_and_straight_alpha() {
        let source = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256"><rect x="143.5" y="20.5" width="60" height="30" fill="#00FF00"/><rect x="163.5" y="20.5" width="40" height="30" fill="#FF0000"/><rect x="163.5" y="40.5" width="40" height="10" fill="#0000FF"/></svg>"##;
        let icon = icon_data(&fitted_svg(source).unwrap());
        assert_eq!(bounds(&icon), (16, 136, 496, 376));
        for (x, y, expected) in [
            (56, 256, [0, 255, 0, 255]),
            (455, 175, [255, 0, 0, 255]),
            (455, 335, [0, 0, 255, 255]),
        ] {
            assert_eq!(
                &icon.rgba[(y * SIDE + x) * 4..(y * SIDE + x + 1) * 4],
                &expected
            );
        }
        let circle = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256"><circle cx="170" cy="70" r="35" fill="#A4CA51"/></svg>"##;
        let icon = icon_data(&fitted_svg(circle).unwrap());
        let edges: Vec<_> = icon
            .rgba
            .chunks_exact(4)
            .filter(|pixel| (64..=192).contains(&pixel[3]))
            .collect();
        assert!(!edges.is_empty());
        for pixel in edges {
            assert!(
                pixel[..3]
                    .iter()
                    .zip([164, 202, 81])
                    .all(|(&channel, expected)| channel.abs_diff(expected) <= 3),
                "premultiplied edge {pixel:?}"
            );
        }
        let empty = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256"/>"#;
        assert!(fitted_svg(empty).is_none());
    }
}
