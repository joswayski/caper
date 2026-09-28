//! Unpacking a verified archive and swapping it in for the installed app.
//!
//! The install target is what the app passes as `--install`:
//! - macOS: the `Caper.app` bundle. Archives are `ditto` zips of `Caper.app`.
//! - Windows: the folder holding `Caper.exe`. Archives hold its files at the root.
//! - Linux: the folder holding `caper-desktop`. Archives are tarballs with one
//!   top-level folder (`Caper-linux-x64/`).
//!
//! The new copy is staged next to the install so the final step is two renames on
//! one filesystem. If the second rename fails, the first is undone.

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    MacBundle,
    WindowsFolder,
    LinuxTarball,
}

impl Layout {
    pub fn for_platform(platform: &str) -> Option<Self> {
        match platform {
            "macos-arm64" | "macos-x64" => Some(Self::MacBundle),
            "windows-x64" => Some(Self::WindowsFolder),
            "linux-x64" => Some(Self::LinuxTarball),
            _ => None,
        }
    }

    /// The program to start after the swap.
    pub fn launch_target(self, install: &Path) -> PathBuf {
        match self {
            Self::MacBundle => install.to_path_buf(),
            Self::WindowsFolder => install.join("Caper.exe"),
            Self::LinuxTarball => install.join("caper-desktop"),
        }
    }
}

/// Whether `install` is a self-contained Caper install this updater may replace:
/// it holds both the app and the updater, as every archive does. A package
/// manager's layout (the .deb puts `caper-desktop` in `/usr/bin` and the updater
/// in `/usr/lib/caper-desktop`) fails this, so `/usr/bin` is never swapped.
pub fn is_self_contained(layout: Layout, install: &Path) -> bool {
    let updater = match layout {
        Layout::MacBundle => install.join("Contents/MacOS/caper-updater"),
        Layout::WindowsFolder => install.join("caper-updater.exe"),
        Layout::LinuxTarball => install.join("caper-updater"),
    };
    let app = match layout {
        Layout::MacBundle => install.join("Contents/Info.plist"),
        _ => layout.launch_target(install),
    };
    updater.is_file() && app.is_file()
}

/// `<parent>/.<name>.<suffix>`: a sibling on the same filesystem, hidden on Unix.
pub fn sibling(install: &Path, suffix: &str) -> io::Result<PathBuf> {
    let parent = install
        .parent()
        .ok_or_else(|| io::Error::other("install path has no parent folder"))?;
    let name = install
        .file_name()
        .ok_or_else(|| io::Error::other("install path has no name"))?
        .to_string_lossy();
    Ok(parent.join(format!(".{name}.{suffix}")))
}

/// Whether the updater can replace `install` without elevated rights.
pub fn can_replace(layout: Layout, install: &Path) -> bool {
    if !is_self_contained(layout, install) {
        return false;
    }
    let Some(parent) = install.parent() else {
        return false;
    };
    let probe = parent.join(format!(".caper-update-probe-{}", std::process::id()));
    let writable = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    writable
}

/// Unpacks `archive` into a fresh `stage` folder and returns the new app root
/// inside it, which replaces `install`.
pub fn unpack(layout: Layout, archive: &Path, stage: &Path) -> io::Result<PathBuf> {
    remove_if_present(stage)?;
    fs::create_dir_all(stage)?;
    match layout {
        Layout::MacBundle => {
            // ditto preserves the bundle's symlinks, modes and signature exactly.
            run(
                "/usr/bin/ditto",
                &[
                    "-x".as_ref(),
                    "-k".as_ref(),
                    archive.as_os_str(),
                    stage.as_os_str(),
                ],
            )?;
            only_child(stage, Some("app"))
        }
        Layout::WindowsFolder => {
            unzip(archive, stage)?;
            if !stage.join("Caper.exe").is_file() {
                return Err(io::Error::other("update archive has no Caper.exe"));
            }
            Ok(stage.to_path_buf())
        }
        Layout::LinuxTarball => {
            let file = fs::File::open(archive)?;
            let mut tarball = tar::Archive::new(flate2::read::GzDecoder::new(file));
            tarball.set_preserve_permissions(true);
            // unpack() refuses entries that would escape `stage`.
            tarball.unpack(stage)?;
            let root = only_child(stage, None)?;
            if !root.join("caper-desktop").is_file() {
                return Err(io::Error::other("update archive has no caper-desktop"));
            }
            Ok(root)
        }
    }
}

fn unzip(archive: &Path, stage: &Path) -> io::Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?).map_err(io::Error::other)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(io::Error::other)?;
        // enclosed_name() rejects absolute paths and `..` components.
        let Some(relative) = entry.enclosed_name() else {
            return Err(io::Error::other("update archive has an unsafe path"));
        };
        let target = stage.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::File::create(&target)?;
        io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

fn only_child(folder: &Path, extension: Option<&str>) -> io::Result<PathBuf> {
    let mut children = fs::read_dir(folder)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            extension.is_none_or(|extension| path.extension().is_some_and(|ext| ext == extension))
        });
    let child = children
        .next()
        .ok_or_else(|| io::Error::other("update archive has no app folder"))?;
    if children.next().is_some() {
        return Err(io::Error::other(
            "update archive has more than one app folder",
        ));
    }
    Ok(child)
}

/// Replaces `install` with `replacement`, keeping the old copy until the new one
/// is in place. Returns the old copy's path so the caller can delete it.
pub fn swap(install: &Path, replacement: &Path) -> io::Result<PathBuf> {
    let old = sibling(install, "old")?;
    remove_if_present(&old)?;
    fs::rename(install, &old)?;
    if let Err(error) = fs::rename(replacement, install) {
        // Put the working copy back before reporting the failure.
        fs::rename(&old, install)?;
        return Err(error);
    }
    Ok(old)
}

pub fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// macOS: the staged app must carry a valid signature from the same team as the
/// installed one, so a build can only update to another build from the same
/// Developer ID.
pub fn check_mac_signature(installed: &Path, staged: &Path) -> io::Result<()> {
    run(
        "/usr/bin/codesign",
        &[
            "--verify".as_ref(),
            "--deep".as_ref(),
            "--strict".as_ref(),
            staged.as_os_str(),
        ],
    )?;
    let installed_team = team_identifier(installed)?;
    let staged_team = team_identifier(staged)?;
    if installed_team != staged_team {
        return Err(io::Error::other(format!(
            "update is signed by team {staged_team}, not {installed_team}"
        )));
    }
    Ok(())
}

fn team_identifier(app: &Path) -> io::Result<String> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .args(["-d".as_ref(), "--verbose=2".as_ref(), app.as_os_str()])
        .output()?;
    // codesign writes its details to stderr.
    let details = String::from_utf8_lossy(&output.stderr);
    details
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|team| !team.is_empty() && *team != "not set")
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other(format!("{} has no signing team", app.display())))
}

fn run<S: AsRef<std::ffi::OsStr>>(program: &str, args: &[S]) -> io::Result<()> {
    let status = std::process::Command::new(program).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} failed with {status}")))
    }
}

/// SHA-256 of a file as lowercase hex.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn swap_replaces_install_and_keeps_the_old_copy_aside() {
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("Caper");
        let replacement = root.path().join("new");
        fs::create_dir_all(&install).unwrap();
        fs::write(install.join("Caper.exe"), "old").unwrap();
        fs::create_dir_all(&replacement).unwrap();
        fs::write(replacement.join("Caper.exe"), "new").unwrap();

        let old = swap(&install, &replacement).unwrap();
        assert_eq!(
            fs::read_to_string(install.join("Caper.exe")).unwrap(),
            "new"
        );
        assert_eq!(fs::read_to_string(old.join("Caper.exe")).unwrap(), "old");
        assert!(!replacement.exists());
    }

    #[test]
    fn failed_swap_restores_the_working_install() {
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("Caper");
        fs::create_dir_all(&install).unwrap();
        fs::write(install.join("Caper.exe"), "old").unwrap();
        let missing = root.path().join("does-not-exist");

        assert!(swap(&install, &missing).is_err());
        assert_eq!(
            fs::read_to_string(install.join("Caper.exe")).unwrap(),
            "old"
        );
    }

    #[test]
    fn unzips_windows_archives_and_requires_caper_exe() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("update.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("Caper.exe", options).unwrap();
        zip.write_all(b"exe").unwrap();
        zip.start_file("licenses/LICENSE", options).unwrap();
        zip.write_all(b"license").unwrap();
        zip.finish().unwrap();

        let stage = root.path().join("stage");
        let app = unpack(Layout::WindowsFolder, &archive, &stage).unwrap();
        assert_eq!(app, stage);
        assert_eq!(fs::read(stage.join("Caper.exe")).unwrap(), b"exe");
        assert_eq!(
            fs::read(stage.join("licenses/LICENSE")).unwrap(),
            b"license"
        );

        let empty = root.path().join("empty.zip");
        zip::ZipWriter::new(fs::File::create(&empty).unwrap())
            .finish()
            .unwrap();
        assert!(unpack(Layout::WindowsFolder, &empty, &stage).is_err());
    }

    #[test]
    fn rejects_zip_entries_that_escape_the_stage() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("evil.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("../escaped.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        assert!(unpack(Layout::WindowsFolder, &archive, &root.path().join("stage")).is_err());
        assert!(!root.path().join("escaped.txt").exists());
    }

    #[test]
    fn untars_linux_archives_to_their_single_top_level_folder() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("update.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            fs::File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        );
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "Caper-linux-x64/caper-desktop", &b"bin"[..])
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let stage = root.path().join("stage");
        let app = unpack(Layout::LinuxTarball, &archive, &stage).unwrap();
        assert_eq!(app, stage.join("Caper-linux-x64"));
        assert_eq!(fs::read(app.join("caper-desktop")).unwrap(), b"bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(app.join("caper-desktop"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "the launcher stays executable");
        }
    }

    #[test]
    fn hashes_files_and_names_hidden_siblings() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("a");
        fs::write(&file, "abc").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sibling(&root.path().join("Caper.app"), "old").unwrap(),
            root.path().join(".Caper.app.old")
        );
    }

    #[test]
    fn only_replaces_folders_that_hold_both_the_app_and_the_updater() {
        let root = tempfile::tempdir().unwrap();
        let tarball = root.path().join("Caper-linux-x64");
        fs::create_dir_all(&tarball).unwrap();
        fs::write(tarball.join("caper-desktop"), "app").unwrap();
        // Like /usr/bin from the .deb: the app is there, the updater is not.
        assert!(!can_replace(Layout::LinuxTarball, &tarball));
        fs::write(tarball.join("caper-updater"), "updater").unwrap();
        assert!(can_replace(Layout::LinuxTarball, &tarball));
        assert!(!can_replace(Layout::WindowsFolder, &tarball));
        assert!(!can_replace(
            Layout::LinuxTarball,
            &root.path().join("missing")
        ));

        let bundle = root.path().join("Caper.app");
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(bundle.join("Contents/Info.plist"), "plist").unwrap();
        assert!(!is_self_contained(Layout::MacBundle, &bundle));
        fs::write(bundle.join("Contents/MacOS/caper-updater"), "updater").unwrap();
        assert!(is_self_contained(Layout::MacBundle, &bundle));
    }
}
