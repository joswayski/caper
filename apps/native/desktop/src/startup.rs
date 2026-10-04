//! Per-user, opt-in launch at login. The OS registration is the source of truth;
//! restoring app preferences must never silently register Caper again.

#[cfg(target_os = "linux")]
mod platform {
    use std::io;
    use std::path::{Path, PathBuf};

    fn entry_path() -> io::Result<PathBuf> {
        config_home(
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        )
        .map(|directory| directory.join("autostart/chat.caper.desktop"))
    }

    fn config_home(xdg: Option<PathBuf>, home: Option<PathBuf>) -> io::Result<PathBuf> {
        xdg.filter(|path| path.is_absolute())
            .or_else(|| {
                home.filter(|path| path.is_absolute())
                    .map(|path| path.join(".config"))
            })
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "No user configuration directory")
            })
    }

    fn exec(executable: &Path) -> io::Result<String> {
        let path = executable
            .to_str()
            .filter(|path| !path.contains(['\n', '\r', '\0', '=']) && executable.is_absolute())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Unsupported executable path")
            })?;
        // Desktop Entry string escaping happens before Exec argument unquoting.
        let mut quoted = String::from("\"");
        for character in path.chars() {
            match character {
                '\\' => quoted.push_str("\\\\\\\\"),
                '"' | '`' | '$' => {
                    quoted.push_str("\\\\");
                    quoted.push(character);
                }
                '%' => quoted.push_str("%%"),
                '\t' => quoted.push_str("\\t"),
                _ => quoted.push(character),
            }
        }
        quoted.push('"');
        // GLib checks the executable before expanding %% into %. Keep that
        // lookup field-code-free; env executes the decoded path without a shell.
        Ok(if path.contains('%') {
            format!("/usr/bin/env {quoted}")
        } else {
            quoted
        })
    }

    fn enabled_at(entry: &Path, executable: &Path) -> io::Result<bool> {
        let contents = match std::fs::read_to_string(entry) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let command = format!("Exec={}", exec(executable)?);
        Ok(contents.lines().any(|line| line == command)
            && !contents.lines().any(|line| {
                matches!(
                    line.trim(),
                    "Hidden=true" | "X-GNOME-Autostart-enabled=false"
                )
            }))
    }

    fn set_enabled_at(entry: &Path, executable: &Path, enabled: bool) -> io::Result<()> {
        if !enabled {
            return match std::fs::remove_file(entry) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            };
        }
        let contents = format!(
            "[Desktop Entry]\nType=Application\nName=Caper\nComment=Conversations with your people\nExec={}\nTerminal=false\n",
            exec(executable)?
        );
        std::fs::create_dir_all(entry.parent().expect("autostart entry has a parent"))?;
        std::fs::write(entry, contents)
    }

    pub fn enabled() -> io::Result<bool> {
        enabled_at(&entry_path()?, &std::env::current_exe()?)
    }

    pub fn set_enabled(enabled: bool) -> io::Result<()> {
        set_enabled_at(&entry_path()?, &std::env::current_exe()?, enabled)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn xdg_requires_absolute_paths_and_falls_back_to_home() {
            assert_eq!(
                config_home(Some("/custom".into()), Some("/home/person".into())).unwrap(),
                PathBuf::from("/custom")
            );
            for xdg in [None, Some("".into()), Some("relative".into())] {
                assert_eq!(
                    config_home(xdg, Some("/home/person".into())).unwrap(),
                    PathBuf::from("/home/person/.config")
                );
            }
            assert!(config_home(None, None).is_err());
        }

        #[test]
        fn exec_escapes_both_desktop_entry_and_command_layers() {
            assert_eq!(
                exec(Path::new("/home/José/Caper app/caper")).unwrap(),
                "\"/home/José/Caper app/caper\""
            );
            assert_eq!(
                exec(Path::new("/tmp/$`\"\\%f/caper")).unwrap(),
                "/usr/bin/env \"/tmp/\\\\$\\\\`\\\\\"\\\\\\\\%%f/caper\""
            );
            for path in ["relative/caper", "/tmp/line\nbreak", "/tmp/a=b/caper"] {
                assert!(exec(Path::new(path)).is_err());
            }
        }

        #[test]
        fn registration_round_trips_and_respects_external_disabling() {
            let directory =
                std::env::temp_dir().join(format!("caper-startup-{}", uuid::Uuid::new_v4()));
            let entry = directory.join("autostart/chat.caper.desktop");
            let executable = Path::new("/opt/Caper app/caper-desktop");
            assert!(!enabled_at(&entry, executable).unwrap());
            set_enabled_at(&entry, executable, false).unwrap();
            set_enabled_at(&entry, executable, true).unwrap();
            assert!(enabled_at(&entry, executable).unwrap());
            assert!(!enabled_at(&entry, Path::new("/other/caper-desktop")).unwrap());
            let contents = std::fs::read_to_string(&entry).unwrap();
            for disabled in ["Hidden=true", "X-GNOME-Autostart-enabled=false"] {
                std::fs::write(&entry, format!("{contents}{disabled}\n")).unwrap();
                assert!(!enabled_at(&entry, executable).unwrap());
            }
            set_enabled_at(&entry, executable, true).unwrap();
            assert!(enabled_at(&entry, executable).unwrap());
            set_enabled_at(&entry, executable, false).unwrap();
            assert!(!entry.exists());
            std::fs::remove_dir_all(directory).unwrap();
        }

        #[test]
        fn failed_registration_is_reported_without_creating_an_entry() {
            let directory =
                std::env::temp_dir().join(format!("caper-startup-{}", uuid::Uuid::new_v4()));
            std::fs::write(&directory, "not a directory").unwrap();
            let entry = directory.join("chat.caper.desktop");
            assert!(set_enabled_at(&entry, Path::new("/opt/caper"), true).is_err());
            assert!(!entry.exists());
            std::fs::remove_file(directory).unwrap();
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::io;
    use std::path::Path;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "Caper";

    fn command(executable: &Path) -> String {
        // Windows executable names cannot contain quotes. Quote the full path so
        // an install under a directory containing spaces remains one argument.
        format!("\"{}\"", executable.display())
    }

    fn enabled_at(path: &str, command: &str) -> io::Result<bool> {
        let user = RegKey::predef(HKEY_CURRENT_USER);
        let value = user
            .open_subkey_with_flags(path, KEY_READ)
            .and_then(|key| key.get_value::<String, _>(VALUE));
        match value {
            Ok(value) => Ok(value == command),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn set_enabled_at(path: &str, command: &str, enabled: bool) -> io::Result<()> {
        let user = RegKey::predef(HKEY_CURRENT_USER);
        if enabled {
            let (key, _) = user.create_subkey_with_flags(path, KEY_SET_VALUE)?;
            key.set_value(VALUE, &command)
        } else {
            match user
                .open_subkey_with_flags(path, KEY_SET_VALUE)
                .and_then(|key| key.delete_value(VALUE))
            {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            }
        }
    }

    pub fn enabled() -> io::Result<bool> {
        enabled_at(RUN_KEY, &command(&std::env::current_exe()?))
    }

    pub fn set_enabled(enabled: bool) -> io::Result<()> {
        set_enabled_at(RUN_KEY, &command(&std::env::current_exe()?), enabled)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn quoted_registration_round_trips_without_touching_other_values() {
            let path = format!(r"Software\Caper\StartupTests\{}", uuid::Uuid::new_v4());
            let command = command(Path::new(r"C:\Users\José Valerio\Caper\app\Caper.exe"));
            assert_eq!(
                command,
                "\"C:\\Users\\José Valerio\\Caper\\app\\Caper.exe\""
            );
            assert!(!enabled_at(&path, &command).unwrap());
            set_enabled_at(&path, &command, false).unwrap();
            set_enabled_at(&path, &command, true).unwrap();
            assert!(enabled_at(&path, &command).unwrap());
            assert!(!enabled_at(&path, "\"C:\\other\\Caper.exe\"").unwrap());
            let user = RegKey::predef(HKEY_CURRENT_USER);
            let key = user
                .open_subkey_with_flags(&path, winreg::enums::KEY_ALL_ACCESS)
                .unwrap();
            key.set_value("OtherApp", &"untouched").unwrap();
            set_enabled_at(&path, &command, false).unwrap();
            assert!(!enabled_at(&path, &command).unwrap());
            assert_eq!(key.get_value::<String, _>("OtherApp").unwrap(), "untouched");
            drop(key);
            user.delete_subkey_all(&path).unwrap();
        }
    }
}

pub use platform::{enabled, set_enabled};
