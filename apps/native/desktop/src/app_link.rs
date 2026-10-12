//! Opening invite links in this app. Browsers hand `caper://invite/<code>`
//! (the web invite page's "Open in the Caper app") to whichever program the OS
//! registered for the scheme, starting it with the link as an argument. When
//! Caper is already open, that new process hands the code to the running app
//! over loopback and exits, so a click never opens a second copy.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// The invite code in launch arguments. Only URL-shaped arguments count, so
/// option values such as a fixture name are never read as codes.
pub fn launch_code(args: &[String]) -> Option<String> {
    args.iter()
        .skip(1)
        .filter(|arg| arg.contains("://"))
        .find_map(|arg| crate::invites::invite_code(arg))
}

fn handoff_path() -> Option<PathBuf> {
    eframe::storage_dir("Caper").map(|directory| directory.join("link-handoff"))
}

/// Gives `code` to an already-running Caper. False when none accepted it.
pub fn hand_off(code: &str) -> bool {
    handoff_path().is_some_and(|path| hand_off_at(&path, code))
}

fn hand_off_at(path: &Path, code: &str) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    let Some((port, token)) = contents.trim().split_once(' ') else {
        return false;
    };
    let Ok(port) = port.parse::<u16>() else {
        return false;
    };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) else {
        // A stale file from a Caper that quit without cleaning up.
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if writeln!(stream, "{token} {code}").is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Receives codes handed off by later launches; `wake` runs after each one so
/// the UI can repaint and pick it up.
pub fn listen(wake: impl Fn() + Send + 'static) -> Option<mpsc::Receiver<String>> {
    listen_at(&handoff_path()?, wake)
}

fn listen_at(path: &Path, wake: impl Fn() + Send + 'static) -> Option<mpsc::Receiver<String>> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    // Only processes that can read this user's file may hand off links.
    let token = uuid::Uuid::new_v4().simple().to_string();
    std::fs::create_dir_all(path.parent()?).ok()?;
    write_private(path, &format!("{port} {token}\n")).ok()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("caper-link-handoff".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if BufReader::new(&stream)
                    .take(256)
                    .read_line(&mut line)
                    .is_err()
                {
                    continue;
                }
                let Some((given, code)) = line.trim().split_once(' ') else {
                    continue;
                };
                if given != token || !crate::invites::valid_code(code) {
                    continue;
                }
                let _ = (&stream).write_all(b"ok\n");
                if sender.send(code.to_owned()).is_err() {
                    return;
                }
                wake();
            }
        })
        .ok()?;
    Some(receiver)
}

fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(contents.as_bytes())
}

/// Registers this executable as the per-user handler for `caper://` links,
/// keeping the registration pointed at the current install after updates.
pub fn register() -> io::Result<()> {
    platform::register(&std::env::current_exe()?)
}

#[cfg(target_os = "linux")]
mod platform {
    use std::io;
    use std::path::{Path, PathBuf};

    const ENTRY: &str = "chat.caper.links.desktop";

    fn applications_dir() -> io::Result<PathBuf> {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .map(|path| path.join(".local/share"))
            })
            .map(|path| path.join("applications"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No user data directory"))
    }

    pub(super) fn entry(executable: &Path) -> io::Result<String> {
        Ok(format!(
            "[Desktop Entry]\nType=Application\nName=Caper\nComment=Open Caper invite links\nExec={} %u\nIcon=caper\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/caper;\n",
            crate::startup::desktop_exec(executable)?
        ))
    }

    pub fn register(executable: &Path) -> io::Result<()> {
        // The .deb's system caper.desktop already declares the scheme.
        if executable.starts_with("/usr/") {
            return Ok(());
        }
        let directory = applications_dir()?;
        let path = directory.join(ENTRY);
        let contents = entry(executable)?;
        if std::fs::read_to_string(&path).is_ok_and(|current| current == contents) {
            return Ok(());
        }
        std::fs::create_dir_all(&directory)?;
        std::fs::write(&path, contents)?;
        // Desktops without xdg-utils still find the entry's MimeType.
        let _ = std::process::Command::new("xdg-mime")
            .args(["default", ENTRY, "x-scheme-handler/caper"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::io;
    use std::path::Path;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    pub fn register(executable: &Path) -> io::Result<()> {
        let user = RegKey::predef(HKEY_CURRENT_USER);
        let (scheme, _) = user.create_subkey(r"Software\Classes\caper")?;
        scheme.set_value("", &"URL:Caper")?;
        scheme.set_value("URL Protocol", &"")?;
        let executable = executable.display();
        let (icon, _) = scheme.create_subkey("DefaultIcon")?;
        icon.set_value("", &format!("\"{executable}\",0"))?;
        let (command, _) = scheme.create_subkey(r"shell\open\command")?;
        command.set_value("", &format!("\"{executable}\" \"%1\""))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod platform {
    pub fn register(_: &std::path::Path) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn launch_codes_come_from_link_arguments_only() {
        assert_eq!(
            launch_code(&args(&["caper", "caper://invite/AbCdE12345"])),
            Some("AbCdE12345".into())
        );
        assert_eq!(
            launch_code(&args(&["caper", "https://caper.chat/invite/AbCdE12345/"])),
            Some("AbCdE12345".into())
        );
        for arguments in [
            &["caper"][..],
            &["caper", "--fixture", "AbCdE12345"],
            &["caper", "--api-url", "http://127.0.0.1:3001"],
            &["caper", "caper://invite/short"],
            &["AbCdE12345://invite/AbCdE12345"],
        ] {
            assert_eq!(launch_code(&args(arguments)), None, "{arguments:?}");
        }
    }

    #[test]
    fn a_later_launch_hands_its_code_to_the_running_app() {
        let directory = std::env::temp_dir().join(format!("caper-link-{}", uuid::Uuid::new_v4()));
        let path = directory.join("link-handoff");
        assert!(!hand_off_at(&path, "AbCdE12345"), "no running app yet");
        let (woke, wakes) = mpsc::channel();
        let codes = listen_at(&path, move || {
            let _ = woke.send(());
        })
        .unwrap();
        assert!(hand_off_at(&path, "AbCdE12345"));
        assert_eq!(
            codes.recv_timeout(Duration::from_secs(5)).unwrap(),
            "AbCdE12345"
        );
        wakes.recv_timeout(Duration::from_secs(5)).unwrap();
        // Without the token from the file, nothing is accepted.
        let contents = std::fs::read_to_string(&path).unwrap();
        let (port, _) = contents.trim().split_once(' ').unwrap();
        std::fs::write(&path, format!("{port} wrong-token\n")).unwrap();
        assert!(!hand_off_at(&path, "AbCdE12345"));
        assert!(codes.recv_timeout(Duration::from_millis(200)).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_handler_entry_passes_the_link_and_stays_out_of_menus() {
        let entry = platform::entry(Path::new("/opt/Caper app/caper-desktop")).unwrap();
        assert!(entry.contains("Exec=\"/opt/Caper app/caper-desktop\" %u\n"));
        assert!(entry.contains("MimeType=x-scheme-handler/caper;\n"));
        assert!(entry.contains("NoDisplay=true\n"));
    }
}
