//! `caper-updater`: the self-updater shipped inside the Caper Mac, Windows and
//! Linux apps.
//!
//! The app runs `caper-updater check --current-build N` in the background and
//! reads one JSON line from stdout. When an update exists and the user accepts,
//! the app starts `caper-updater apply --current-build N --install <path>
//! --wait-pid <app pid>` and quits. `apply` copies itself out of the install,
//! downloads and verifies the archive, waits for the app to exit, swaps the new
//! copy in, and reopens Caper.
//!
//! Release tooling uses `sign`, `public-key` and `verify` with the Ed25519 key
//! kept in AWS Secrets Manager. The public key is compiled in from
//! `CAPER_UPDATE_PUBLIC_KEY`; builds without it report updates as unavailable.

mod install;
mod manifest;

use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    thread,
    time::{Duration, Instant},
};

use ed25519_dalek::{SigningKey, pkcs8::DecodePrivateKey};
use serde::Serialize;

use install::Layout;
use manifest::Manifest;

const MANIFEST_URL: &str =
    "https://github.com/joswayski/caper/releases/download/native-latest/latest.json";
const PUBLIC_KEY: Option<&str> = option_env!("CAPER_UPDATE_PUBLIC_KEY");
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const EXIT_WAIT: Duration = Duration::from_secs(60);
/// Set on the relocated copy so it does not relocate again.
const RELOCATED_FLAG: &str = "--relocated";

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log(&format!("error: {error}"));
            eprintln!("caper-updater: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[OsString]) -> Result<()> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return Err(usage());
    };
    let options = Options::parse(&args[1..])?;
    match command {
        "check" => check(&options),
        "apply" => apply(&options, args),
        "sign" => sign(&options),
        "public-key" => public_key(&options),
        "verify" => verify_file(&options),
        _ => Err(usage()),
    }
}

fn usage() -> Box<dyn std::error::Error> {
    "usage: caper-updater check --current-build N | apply --current-build N --install PATH --wait-pid PID | sign --key PEM --manifest FILE | public-key --key PEM | verify --public-key B64 --manifest FILE".into()
}

#[derive(Default)]
struct Options {
    current_build: Option<u64>,
    install: Option<PathBuf>,
    wait_pid: Option<u32>,
    key: Option<PathBuf>,
    manifest: Option<PathBuf>,
    public_key: Option<String>,
    relocated: bool,
}

impl Options {
    fn parse(args: &[OsString]) -> Result<Self> {
        let mut options = Self::default();
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let flag = flag.to_str().ok_or("arguments must be UTF-8 flags")?;
            if flag == RELOCATED_FLAG {
                options.relocated = true;
                continue;
            }
            let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
            match flag {
                "--current-build" => {
                    options.current_build = Some(value.to_str().ok_or("bad build")?.parse()?);
                }
                "--install" => options.install = Some(PathBuf::from(value)),
                "--wait-pid" => options.wait_pid = Some(value.to_str().ok_or("bad pid")?.parse()?),
                "--key" => options.key = Some(PathBuf::from(value)),
                "--manifest" => options.manifest = Some(PathBuf::from(value)),
                "--public-key" => {
                    options.public_key = Some(value.to_str().ok_or("bad key")?.to_owned());
                }
                _ => return Err(format!("unknown flag {flag}").into()),
            }
        }
        Ok(options)
    }
}

/// What `check` prints for the app, one JSON object on one line.
#[derive(Serialize)]
struct CheckResult<'a> {
    update: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<&'a str>,
    /// False when the install cannot be replaced without admin rights, for
    /// example a Linux .deb under /usr; the app offers a download link instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    can_apply: Option<bool>,
}

fn check(options: &Options) -> Result<()> {
    let current = options.current_build.ok_or("check needs --current-build")?;
    let (Some(key), Some(platform)) = (PUBLIC_KEY, manifest::current_platform()) else {
        return print_json(&CheckResult {
            update: false,
            build: None,
            version: None,
            notes: None,
            can_apply: None,
        });
    };
    let manifest = fetch_manifest(key)?;
    let can_apply = options.install.as_deref().map(|install| {
        Layout::for_platform(platform).is_some_and(|layout| install::can_replace(layout, install))
    });
    match manifest.update_for(platform, current) {
        Some(_) => print_json(&CheckResult {
            update: true,
            build: Some(manifest.build),
            version: Some(&manifest.version),
            notes: Some(&manifest.notes),
            can_apply,
        }),
        None => print_json(&CheckResult {
            update: false,
            build: None,
            version: None,
            notes: None,
            can_apply: None,
        }),
    }
}

fn print_json(value: &impl Serialize) -> Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    writeln!(stdout)?;
    Ok(())
}

fn apply(options: &Options, args: &[OsString]) -> Result<()> {
    let current = options.current_build.ok_or("apply needs --current-build")?;
    let install = options.install.as_deref().ok_or("apply needs --install")?;
    let install = fs::canonicalize(install)?;
    let key = PUBLIC_KEY.ok_or("this build has no update key")?;
    let platform = manifest::current_platform().ok_or("unsupported platform")?;
    let layout = Layout::for_platform(platform).ok_or("unsupported platform")?;
    if !install::is_self_contained(layout, &install) {
        return Err(format!(
            "{} is not a self-contained Caper install; update it with its package manager",
            install.display()
        )
        .into());
    }

    // Windows cannot rename a folder while a program inside it runs, so run the
    // update from a copy outside the install.
    if !options.relocated && env::current_exe()?.starts_with(&install) {
        return relocate(args);
    }
    log(&format!(
        "updating {} from build {current}",
        install.display()
    ));

    let manifest = fetch_manifest(key)?;
    let artifact = manifest
        .update_for(platform, current)
        .ok_or("no newer build for this platform")?;
    let archive = env::temp_dir().join(format!("caper-update-{}", manifest.build));
    download(&artifact.url, artifact.size, &archive)?;
    let digest = install::sha256_file(&archive)?;
    if digest != artifact.sha256 {
        fs::remove_file(&archive).ok();
        return Err(
            format!("download checksum {digest} does not match the signed manifest").into(),
        );
    }

    let stage = install::sibling(&install, "update")?;
    let replacement = install::unpack(layout, &archive, &stage)?;
    if layout == Layout::MacBundle {
        install::check_mac_signature(&install, &replacement)?;
    }
    if let Some(pid) = options.wait_pid {
        wait_for_exit(pid, EXIT_WAIT)?;
    }
    let old = install::swap(&install, &replacement)?;
    log(&format!("installed build {}", manifest.build));
    install::remove_if_present(&stage).ok();
    install::remove_if_present(&old).ok();
    fs::remove_file(&archive).ok();
    relaunch(layout, &install)
}

fn relocate(args: &[OsString]) -> Result<()> {
    let source = env::current_exe()?;
    let name = source.file_name().ok_or("updater has no file name")?;
    let folder = env::temp_dir().join(format!("caper-updater-{}", std::process::id()));
    fs::create_dir_all(&folder)?;
    let copy = folder.join(name);
    fs::copy(&source, &copy)?;
    Command::new(&copy).args(args).arg(RELOCATED_FLAG).spawn()?;
    Ok(())
}

fn relaunch(layout: Layout, install: &Path) -> Result<()> {
    let target = layout.launch_target(install);
    if layout == Layout::MacBundle {
        Command::new("/usr/bin/open").arg(&target).spawn()?;
    } else {
        Command::new(&target).current_dir(install).spawn()?;
    }
    Ok(())
}

fn http() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("caper-updater/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15 * 60))
        .build()?)
}

fn fetch_manifest(public_key: &str) -> Result<Manifest> {
    let client = http()?;
    let bytes = fetch_limited(&client, MANIFEST_URL, manifest::MAX_MANIFEST_BYTES as u64)?;
    let signature = fetch_limited(
        &client,
        &format!("{MANIFEST_URL}.sig"),
        manifest::MAX_MANIFEST_BYTES as u64,
    )?;
    let key = manifest::decode_public_key(public_key)?;
    Ok(manifest::verify(
        &bytes,
        &String::from_utf8(signature)?,
        &key,
    )?)
}

fn fetch_limited(client: &reqwest::blocking::Client, url: &str, limit: u64) -> Result<Vec<u8>> {
    let response = client.get(url).send()?.error_for_status()?;
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("{url} is larger than expected").into());
    }
    Ok(bytes)
}

fn download(url: &str, size: u64, target: &Path) -> Result<()> {
    if size > MAX_ARCHIVE_BYTES {
        return Err("update archive is too large".into());
    }
    let response = http()?.get(url).send()?.error_for_status()?;
    let mut file = fs::File::create(target)?;
    let copied = io::copy(&mut response.take(size + 1), &mut file)?;
    if copied != size {
        return Err(format!("downloaded {copied} bytes, expected {size}").into());
    }
    Ok(())
}

fn sign(options: &Options) -> Result<()> {
    let key = read_signing_key(options)?;
    let path = options.manifest.as_deref().ok_or("sign needs --manifest")?;
    let bytes = fs::read(path)?;
    let mut signature_path = path.as_os_str().to_owned();
    signature_path.push(".sig");
    fs::write(signature_path, manifest::sign(&bytes, &key))?;
    Ok(())
}

fn public_key(options: &Options) -> Result<()> {
    let key = read_signing_key(options)?;
    println!("{}", manifest::encode_public_key(&key.verifying_key()));
    Ok(())
}

fn verify_file(options: &Options) -> Result<()> {
    let key = options
        .public_key
        .as_deref()
        .ok_or("verify needs --public-key")?;
    let path = options
        .manifest
        .as_deref()
        .ok_or("verify needs --manifest")?;
    let mut signature_path = path.as_os_str().to_owned();
    signature_path.push(".sig");
    let manifest = manifest::verify(
        &fs::read(path)?,
        &fs::read_to_string(PathBuf::from(signature_path))?,
        &manifest::decode_public_key(key)?,
    )?;
    println!("verified build {} ({})", manifest.build, manifest.version);
    Ok(())
}

fn read_signing_key(options: &Options) -> Result<SigningKey> {
    let path = options.key.as_deref().ok_or("needs --key PEM")?;
    Ok(SigningKey::from_pkcs8_pem(&fs::read_to_string(path)?)?)
}

fn wait_for_exit(pid: u32, limit: Duration) -> Result<()> {
    let deadline = Instant::now() + limit;
    while process_running(pid) {
        if Instant::now() >= deadline {
            return Err(format!("Caper (pid {pid}) did not quit").into());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

#[cfg(unix)]
fn process_running(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // Signal 0 only checks whether the process exists.
    // SAFETY: kill with signal 0 has no effect beyond the existence check.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(windows)]
fn process_running(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    // SAFETY: the handle is checked, waited on without blocking, and closed.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        CloseHandle(handle);
        running
    }
}

/// Appends to `caper-updater.log` in the temp folder; failures are ignored.
fn log(message: &str) {
    let path = env::temp_dir().join("caper-updater.log");
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_and_rejects_unknown_ones() {
        let args: Vec<OsString> = [
            "--current-build",
            "12",
            "--install",
            "/Applications/Caper.app",
            "--wait-pid",
            "99",
            "--relocated",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        let options = Options::parse(&args).unwrap();
        assert_eq!(options.current_build, Some(12));
        assert_eq!(
            options.install.as_deref(),
            Some(Path::new("/Applications/Caper.app"))
        );
        assert_eq!(options.wait_pid, Some(99));
        assert!(options.relocated);

        assert!(Options::parse(&[OsString::from("--nope"), OsString::from("1")]).is_err());
        assert!(Options::parse(&[OsString::from("--current-build")]).is_err());
    }

    #[test]
    fn sign_public_key_and_verify_round_trip_through_files() {
        let folder = tempfile::tempdir().unwrap();
        let pem = folder.path().join("key.pem");
        let key = SigningKey::from_bytes(&[3; 32]);
        use ed25519_dalek::pkcs8::{EncodePrivateKey, spki::der::pem::LineEnding};
        fs::write(&pem, key.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes()).unwrap();
        let manifest_path = folder.path().join("latest.json");
        fs::write(
            &manifest_path,
            r#"{"schema":1,"build":5,"version":"0.1.5","commit":"c","platforms":{}}"#,
        )
        .unwrap();

        let options = Options {
            key: Some(pem),
            manifest: Some(manifest_path.clone()),
            ..Options::default()
        };
        sign(&options).unwrap();
        let verify_options = Options {
            manifest: Some(manifest_path),
            public_key: Some(manifest::encode_public_key(&key.verifying_key())),
            ..Options::default()
        };
        verify_file(&verify_options).unwrap();
    }

    #[test]
    fn a_process_that_exited_is_not_waited_on() {
        let mut child = Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                &["/C", "exit"][..]
            } else {
                &[][..]
            })
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        wait_for_exit(pid, Duration::from_secs(5)).unwrap();
        assert!(process_running(std::process::id()));
    }
}
