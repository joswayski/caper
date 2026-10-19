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
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{SigningKey, pkcs8::DecodePrivateKey};
use serde::{Deserialize, Serialize};

use install::Layout;
use manifest::Manifest;

const CACHED_MANIFEST_URL: &str = "https://caper.chat/api/updates/native";
const MANIFEST_URL: &str =
    "https://github.com/joswayski/caper/releases/download/native-latest/latest.json";
const PUBLIC_KEY: Option<&str> = option_env!("CAPER_UPDATE_PUBLIC_KEY");
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const EXIT_WAIT: Duration = Duration::from_secs(60);
/// Set on the relocated copy so it does not relocate again.
const RELOCATED_FLAG: &str = "--relocated";
/// Name prefix of the relocated updater's temporary folder.
const RELOCATION_PREFIX: &str = "caper-updater-";

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
        "check" => match check(&options) {
            Err(error) if error.is::<RateLimited>() => {
                print_json(&serde_json::json!({"update": false, "deferred": true}))
            }
            result => result,
        },
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
    current_revision: Option<String>,
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
                "--current-revision" => {
                    let revision = value.to_str().ok_or("bad revision")?;
                    if !revision_valid(revision) {
                        return Err("current revision must be a full lowercase source SHA".into());
                    }
                    options.current_revision = Some(revision.to_owned());
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
    last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    changelog: Vec<&'a manifest::ChangelogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    history_complete: Option<bool>,
    /// Structured highlights for every release being installed, newest first,
    /// so the app can show titles and bodies with hierarchy. Absent when they
    /// could not be read; empty for a build with no user-facing changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    highlights: Option<&'a [Highlight]>,
    /// The newest highlighted release's date (YYYY-MM-DD), shown instead of a
    /// version number.
    #[serde(skip_serializing_if = "Option::is_none")]
    released: Option<&'a str>,
    /// The update's source commit, so the app can link its version to GitHub.
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<&'a str>,
    /// The update's highlights snapshot sequence. Installing from the review
    /// marks it read, so the new build does not show the same highlights again.
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence: Option<u64>,
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
            last_error: None,
            build: None,
            version: None,
            notes: None,
            changelog: Vec::new(),
            history_complete: None,
            highlights: None,
            released: None,
            revision: None,
            sequence: None,
            can_apply: None,
        });
    };
    let manifest = fetch_manifest(key)?;
    let can_apply = options.install.as_deref().map(|install| {
        Layout::for_platform(platform).is_some_and(|layout| install::can_replace(layout, install))
    });
    // Editorial metadata never controls update eligibility or artifact trust.
    // Older clients omit the revision and keep their existing signed changelog.
    let highlights = if manifest.update_for(platform, current).is_some() {
        options.current_revision.as_deref().map(|revision| {
            let result = (|| {
                let client = reqwest::blocking::Client::builder()
                    .https_only(true)
                    .timeout(Duration::from_secs(5))
                    .user_agent("caper-updater-highlights")
                    .build()?;
                release_highlights_from(
                    &client,
                    "https://caper.chat/api/whats-new",
                    platform.split('-').next().unwrap_or(platform),
                    revision,
                    current,
                    &manifest.commit,
                )
            })();
            result.unwrap_or_else(|_: Box<dyn std::error::Error>| ReleaseHighlights {
                notes:
                    "Release highlights aren’t available yet. You can still install this update."
                        .into(),
                complete: false,
                items: None,
                released: None,
                sequence: None,
            })
        })
    } else {
        None
    };
    match manifest.update_for(platform, current) {
        Some(_) => print_json(&CheckResult {
            update: true,
            last_error: options
                .install
                .as_deref()
                .and_then(|path| last_failure(path, current)),
            build: Some(manifest.build),
            version: Some(&manifest.version),
            notes: Some(
                highlights
                    .as_ref()
                    .map(|highlights| highlights.notes.as_str())
                    .unwrap_or(&manifest.notes),
            ),
            changelog: if highlights.is_some() {
                Vec::new()
            } else {
                manifest.changes_since(current)
            },
            history_complete: Some(
                highlights
                    .as_ref()
                    .map(|highlights| highlights.complete)
                    .unwrap_or_else(|| manifest.history_complete(current)),
            ),
            highlights: highlights
                .as_ref()
                .and_then(|highlights| highlights.items.as_deref()),
            released: highlights
                .as_ref()
                .and_then(|highlights| highlights.released.as_deref()),
            revision: Some(&manifest.commit),
            sequence: highlights
                .as_ref()
                .and_then(|highlights| highlights.sequence),
            can_apply,
        }),
        None => print_json(&CheckResult {
            update: false,
            last_error: None,
            build: None,
            version: None,
            notes: None,
            changelog: Vec::new(),
            history_complete: None,
            highlights: None,
            released: None,
            revision: None,
            sequence: None,
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
    let platform = manifest::current_platform().ok_or("unsupported platform")?;
    let layout = Layout::for_platform(platform).ok_or("unsupported platform")?;
    if !install::is_self_contained(layout, &install) {
        return Err(format!(
            "{} is not a self-contained Caper install; update it with its package manager",
            install.display()
        )
        .into());
    }

    // Always relocate, even if the executable appears to be outside the install:
    // Windows current_exe() and canonicalize() can use different path prefixes,
    // and an external updater can still inherit a working directory inside it.
    if !options.relocated {
        record_failure(
            &install,
            current,
            "The last update did not finish. Try again or download the installer.",
        );
        if let Err(error) = relocate_command(args).and_then(|mut command| Ok(command.spawn()?)) {
            record_failure(
                &install,
                current,
                &format!("Could not start the update: {error}"),
            );
            return Err(error);
        }
        return Ok(());
    }
    log(&format!(
        "updating {} from build {current}",
        install.display()
    ));
    if let Some(pid) = options.wait_pid
        && let Err(error) = wait_for_exit(pid, EXIT_WAIT)
    {
        record_failure(&install, current, &error.to_string());
        return Err(error);
    }

    // The app has already quit. A failed download, verification or swap must
    // still reopen the working install instead of leaving Caper closed.
    let result = (|| -> Result<()> {
        let key = PUBLIC_KEY.ok_or("this build has no update key")?;
        let manifest = fetch_manifest(key)?;
        let artifact = manifest
            .update_for(platform, current)
            .ok_or("no newer build for this platform")?;
        // A fresh private folder, removed on drop: a predictable path in a
        // shared /tmp could be pre-created or swapped between hashing and
        // unpacking by another local user.
        let download_folder = private_temp_folder("caper-update-")?;
        let archive = download_folder
            .path()
            .join(format!("caper-update-{}", manifest.build));
        download(&artifact.url, artifact.size, &archive)?;
        let digest = install::sha256_file(&archive)?;
        if digest != artifact.sha256 {
            return Err(
                format!("download checksum {digest} does not match the signed manifest").into(),
            );
        }

        let stage = install::sibling(&install, "update")?;
        let replacement = install::unpack(layout, &archive, &stage)?;
        if layout == Layout::MacBundle {
            install::check_mac_signature(&install, &replacement)?;
        }
        let old = install::swap(&install, &replacement)
            .map_err(|error| format!("Could not replace the installed app: {error}"))?;
        log(&format!("installed build {}", manifest.build));
        install::remove_if_present(&stage).ok();
        install::remove_if_present(&old).ok();
        Ok(())
    })();
    if let Err(error) = &result {
        log(&format!("update failed: {error}; reopening installed app"));
        record_failure(&install, current, &error.to_string());
    } else if let Ok(path) = install::sibling(&install, "update-error") {
        fs::remove_file(path).ok();
    }
    let relaunched = relaunch(layout, &install);
    remove_relocated_copy();
    relaunched?;
    result
}

/// Unix lets a running program delete its own folder; Windows locks it, so
/// that copy stays for the system's temp cleanup.
fn remove_relocated_copy() {
    #[cfg(unix)]
    if let Ok(exe) = env::current_exe()
        && let Some(folder) = exe.parent()
        && is_relocation_folder(folder)
    {
        install::remove_if_present(folder).ok();
    }
}

/// Only a folder `relocate_command` made: never the install, even when
/// `--relocated` is passed by hand.
#[cfg(any(unix, test))]
fn is_relocation_folder(folder: &Path) -> bool {
    let in_temp = |temp: &Path| folder.parent() == Some(temp);
    let temp = env::temp_dir();
    (in_temp(&temp) || fs::canonicalize(&temp).is_ok_and(|temp| in_temp(&temp)))
        && folder
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(RELOCATION_PREFIX))
}

// Operational feedback lives beside the replaceable app, not in it. Bind it to
// the old build so a manual install cannot inherit an obsolete failure notice.
#[derive(Serialize, Deserialize)]
struct Failure {
    current_build: u64,
    message: String,
}

fn record_failure(install: &Path, current_build: u64, message: &str) {
    if let Ok(path) = install::sibling(install, "update-error")
        && let Ok(bytes) = serde_json::to_vec(&Failure {
            current_build,
            message: message.into(),
        })
        && let Err(error) = fs::write(path, bytes)
    {
        log(&format!("could not save update failure: {error}"));
    }
}

fn last_failure(install: &Path, current_build: u64) -> Option<String> {
    let file = fs::File::open(install::sibling(install, "update-error").ok()?).ok()?;
    let failure: Failure = serde_json::from_reader(file.take(16 * 1024)).ok()?;
    (failure.current_build == current_build).then_some(failure.message)
}

fn relocate_command(args: &[OsString]) -> Result<Command> {
    let source = env::current_exe()?;
    let name = source.file_name().ok_or("updater has no file name")?;
    // Never a predictable path: another user could pre-create it in a shared
    // /tmp and replace the copy before it runs.
    let folder = private_temp_folder(RELOCATION_PREFIX)?.keep();
    let copy = folder.join(name);
    fs::copy(&source, &copy)?;
    let mut command = Command::new(&copy);
    // Windows also locks a process's working directory. Moving only the exe
    // leaves an inherited install directory locked and prevents the swap.
    command.current_dir(&folder);
    // The first updater starts hidden, but Windows does not inherit that flag
    // when it relocates itself outside the directory being replaced.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    // The original app's launcher may tear down its process group and pipes
    // when it exits. The relocated updater must survive that shutdown.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.args(args).arg(RELOCATED_FLAG);
    Ok(command)
}

/// A new, randomly named folder in the temp folder that only this user can
/// open on Unix (tempfile otherwise leaves it 0755).
fn private_temp_folder(prefix: &str) -> io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    builder.tempdir()
}

fn relaunch(layout: Layout, install: &Path) -> Result<()> {
    let target = layout.launch_target(install);
    if layout == Layout::MacBundle {
        // LaunchServices owns the new app's lifetime. Force a new instance and
        // wait for the launch request (not the app) to report success.
        let output = Command::new("/usr/bin/open")
            .arg("-n")
            .arg(&target)
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "could not reopen {}: {}: {}",
                target.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
    } else {
        let mut command = Command::new(&target);
        command
            .current_dir(install)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.spawn()?;
    }
    log(&format!("reopened {}", target.display()));
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
    let key = manifest::decode_public_key(public_key)?;
    fetch_manifest_from(&client, &key, CACHED_MANIFEST_URL, MANIFEST_URL)
}

#[derive(Deserialize)]
struct CachedManifest {
    manifest: String,
    signature: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Highlights {
    schema: u8,
    platform: String,
    revision: String,
    sequence: u64,
    releases: Vec<HighlightedRelease>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HighlightedRelease {
    sequence: u64,
    revision: String,
    /// The native app version ("0.1.<build>"); absent on older entries.
    #[serde(default)]
    version: Option<String>,
    published_at: String,
    highlights: Vec<Highlight>,
}

impl HighlightedRelease {
    /// The build number in this entry's version, when it has one.
    fn build(&self) -> Option<u64> {
        let mut parts = self.version.as_deref()?.split('.');
        let build = parts.nth(2)?;
        (parts.next().is_none() && build.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| build.parse().ok())
            .flatten()
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct Highlight {
    title: String,
    body: String,
}

fn revision_valid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Highlights for the releases between the installed build and the update.
#[derive(Debug, PartialEq)]
struct ReleaseHighlights {
    /// Plain-text fallback for apps that predate `items`.
    notes: String,
    complete: bool,
    /// `None` when the highlights could not be read.
    items: Option<Vec<Highlight>>,
    released: Option<String>,
    /// The target snapshot's sequence.
    sequence: Option<u64>,
}

fn release_highlights_from(
    client: &reqwest::blocking::Client,
    url: &str,
    platform: &str,
    current_revision: &str,
    current_build: u64,
    target_revision: &str,
) -> Result<ReleaseHighlights> {
    if !matches!(platform, "macos" | "windows" | "linux")
        || !revision_valid(current_revision)
        || !revision_valid(target_revision)
    {
        return Err("invalid highlights revision or platform".into());
    }
    let fetch = |revision: &str| -> Result<Highlights> {
        let bytes = fetch_limited(
            client,
            &format!("{url}?platform={platform}&revision={revision}"),
            64 * 1024,
        )?;
        let feed: Highlights = serde_json::from_slice(&bytes)?;
        let text_valid = |value: &str, max: usize| {
            !value.trim().is_empty()
                && value.chars().count() <= max
                && !value
                    .chars()
                    .any(|char| char.is_control() || matches!(char, '<' | '>'))
        };
        let mut previous = 0;
        if feed.schema != 1
            || feed.platform != platform
            || feed.revision != revision
            || feed.sequence == 0
            || feed.sequence > 9_007_199_254_740_991
            || feed.releases.is_empty()
            || feed.releases.len() > 30
            || feed.releases.iter().any(|release| {
                let valid = release.sequence > previous
                    && release.sequence <= feed.sequence
                    && revision_valid(&release.revision)
                    && release.published_at.len() >= 20
                    && release.published_at.is_ascii()
                    && release.published_at.as_bytes().get(10) == Some(&b'T')
                    && release
                        .version
                        .as_deref()
                        .is_none_or(|version| text_valid(version, 64))
                    && release.highlights.len() <= 5
                    && release
                        .highlights
                        .iter()
                        .all(|item| text_valid(&item.title, 100) && text_valid(&item.body, 500));
                previous = release.sequence;
                !valid
            })
            || !feed.releases.last().is_some_and(|release| {
                release.sequence == feed.sequence && release.revision == revision
            })
        {
            return Err("invalid release highlights".into());
        }
        Ok(feed)
    };
    // The installed build's own snapshot is missing when its notes failed to
    // publish. The update's feed still lists earlier releases, so place the
    // installed build there, or by build number, rather than show nothing.
    let current = fetch(current_revision).ok();
    let target = fetch(target_revision)?;
    let installed = current.map(|current| current.sequence).or_else(|| {
        target
            .releases
            .iter()
            .find(|release| release.revision == current_revision)
            .map(|release| release.sequence)
    });
    let newer: Vec<_> = match installed {
        Some(installed) => target
            .releases
            .iter()
            .rev()
            .filter(|release| release.sequence > installed)
            .collect(),
        None => {
            let built: Vec<_> = target
                .releases
                .iter()
                .rev()
                .filter(|release| release.build().is_some_and(|build| build > current_build))
                .collect();
            if built.is_empty() {
                target.releases.last().into_iter().collect()
            } else {
                built
            }
        }
    };
    let notes: Vec<_> = newer
        .iter()
        .filter(|release| !release.highlights.is_empty())
        .map(|release| {
            format!(
                "{}\n{}",
                &release.published_at[..10],
                release
                    .highlights
                    .iter()
                    .map(|item| format!("{}\n{}", item.title, item.body))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            )
        })
        .collect();
    // Without the installed build's position, earlier entries may be missing.
    let complete = installed.is_some_and(|installed| target.releases[0].sequence <= installed + 1);
    // People install one update, not a list of versions: stack every skipped
    // release's highlights newest first, dropping a title repeated later on.
    let mut items: Vec<Highlight> = Vec::new();
    for item in newer.iter().flat_map(|release| &release.highlights) {
        let title = item.title.trim().to_lowercase();
        if !items
            .iter()
            .any(|seen| seen.title.trim().to_lowercase() == title)
        {
            items.push(item.clone());
        }
    }
    let released = newer
        .iter()
        .find(|release| !release.highlights.is_empty())
        .map(|release| release.published_at[..10].to_owned());
    Ok(ReleaseHighlights {
        notes: if notes.is_empty() {
            "No user-facing changes in this build.".into()
        } else {
            notes.join("\n\n")
        },
        complete,
        items: Some(items),
        released,
        sequence: Some(target.sequence),
    })
}

fn fetch_manifest_from(
    client: &reqwest::blocking::Client,
    key: &ed25519_dalek::VerifyingKey,
    cached_url: &str,
    github_url: &str,
) -> Result<Manifest> {
    // The site returns the signed bytes and signature together, so caches and
    // replicas cannot mix separate responses. Never trust metadata before
    // verifying it; an unavailable or invalid mirror falls back to GitHub.
    let cached = (|| -> Result<Manifest> {
        let envelope = fetch_limited(client, cached_url, 2 * manifest::MAX_MANIFEST_BYTES as u64)?;
        let envelope: CachedManifest = serde_json::from_slice(&envelope)?;
        let bytes = STANDARD.decode(&envelope.manifest)?;
        Ok(manifest::verify(&bytes, &envelope.signature, key)?)
    })();
    if let Ok(manifest) = cached {
        return Ok(manifest);
    }
    let bytes = fetch_limited(client, github_url, manifest::MAX_MANIFEST_BYTES as u64)?;
    let signature = fetch_limited(
        client,
        &format!("{github_url}.sig"),
        manifest::MAX_MANIFEST_BYTES as u64,
    )?;
    Ok(manifest::verify(
        &bytes,
        &String::from_utf8(signature)?,
        key,
    )?)
}

fn fetch_limited(client: &reqwest::blocking::Client, url: &str, limit: u64) -> Result<Vec<u8>> {
    let response = client.get(url).timeout(Duration::from_secs(15)).send()?;
    // GitHub uses 403 for some throttles, but a plain forbidden response is
    // still an error. Never hide authentication or signature failures.
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
        || (response.status() == reqwest::StatusCode::FORBIDDEN
            && (response.headers().contains_key("retry-after")
                || response
                    .headers()
                    .get("x-ratelimit-remaining")
                    .is_some_and(|value| value == "0")))
    {
        return Err(Box::new(RateLimited));
    }
    let response = response.error_for_status()?;
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("{url} is larger than expected").into());
    }
    Ok(bytes)
}

#[derive(Debug)]
struct RateLimited;

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("update check deferred by rate limiting")
    }
}

impl std::error::Error for RateLimited {}

fn download(url: &str, size: u64, target: &Path) -> Result<()> {
    if size > MAX_ARCHIVE_BYTES {
        return Err("update archive is too large".into());
    }
    let response = http()?.get(url).send()?.error_for_status()?;
    let mut file = fs::File::create_new(target)?;
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

/// Appends to `caper-updater.log`; failures are ignored.
fn log(message: &str) {
    let Some(path) = log_path() else {
        return;
    };
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(path) {
        let _ = writeln!(file, "{message}");
    }
}

/// macOS ($TMPDIR) and Windows (%TEMP%) temp folders belong to the user. Linux
/// /tmp is shared, so the log goes to the XDG state folder instead.
fn log_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    let folder = {
        let folder = linux_log_folder(
            env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            env::var_os("HOME").map(PathBuf::from),
        )?;
        fs::create_dir_all(&folder).ok()?;
        folder
    };
    #[cfg(not(target_os = "linux"))]
    let folder = env::temp_dir();
    Some(folder.join("caper-updater.log"))
}

/// `$XDG_STATE_HOME/caper`, else `~/.local/state/caper`.
#[cfg(any(target_os = "linux", test))]
fn linux_log_folder(xdg_state: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    xdg_state
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|path| path.is_absolute())
                .map(|home| home.join(".local/state"))
        })
        .map(|state| state.join("caper"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A parallel fork can inherit a fixture's write descriptor until exec,
    // causing ETXTBSY even after fs::copy closes it in this process. Keep all
    // child-process tests separate from each other's write-then-exec windows.
    // https://github.com/rust-lang/rust/issues/114554
    static CHILD_PROCESS_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serve_metadata(
        responses: Vec<(&'static str, u16, Vec<u8>)>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for (path, status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(
                    String::from_utf8(request)
                        .unwrap()
                        .starts_with(&format!("GET {path} HTTP/1.1\r\n"))
                );
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (url, server)
    }

    #[test]
    fn preinstall_highlights_use_exact_platform_revisions_and_skip_old_and_silent_entries() {
        use serde_json::json;
        let current_revision = "a".repeat(40);
        let target_revision = "d".repeat(40);
        let entry = |sequence, revision: &str, title: Option<&str>| {
            json!({
                "sequence": sequence, "revision": revision, "publishedAt": "2026-10-11T12:00:00Z",
                "highlights": title.map(|title| vec![json!({"title": title, "body": "A useful change."})]).unwrap_or_default()
            })
        };
        let current = json!({"schema":1,"platform":"macos","revision":current_revision,"sequence":1,
            "releases":[entry(1,&current_revision,Some("Already installed"))]});
        let target = json!({"schema":1,"platform":"macos","revision":target_revision,"sequence":4,
            "releases":[entry(1,&current_revision,Some("Already installed")),entry(2,&"b".repeat(40),Some("Skipped update")),
                entry(3,&"c".repeat(40),None),entry(4,&target_revision,Some("Newest update"))]});
        let client = reqwest::blocking::Client::new();
        for (invalid, body) in [
            (false, target.clone()),
            (true, {
                let mut value = target.clone();
                value["platform"] = json!("windows");
                value
            }),
            (true, {
                let mut value = target.clone();
                value["revision"] = json!("e".repeat(40));
                value
            }),
        ] {
            let (url, server) = serve_metadata(vec![
                (
                    "/notes?platform=macos&revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    200,
                    serde_json::to_vec(&current).unwrap(),
                ),
                (
                    "/notes?platform=macos&revision=dddddddddddddddddddddddddddddddddddddddd",
                    200,
                    serde_json::to_vec(&body).unwrap(),
                ),
            ]);
            let result = release_highlights_from(
                &client,
                &format!("{url}/notes"),
                "macos",
                &current_revision,
                1,
                &target_revision,
            );
            if invalid {
                assert!(result.is_err());
            } else {
                let result = result.unwrap();
                assert_eq!(
                    result.notes,
                    "2026-10-11\nNewest update\nA useful change.\n\n2026-10-11\nSkipped update\nA useful change."
                );
                assert!(result.complete);
                let titles: Vec<_> = result
                    .items
                    .unwrap()
                    .into_iter()
                    .map(|item| item.title)
                    .collect();
                assert_eq!(titles, ["Newest update", "Skipped update"]);
                assert_eq!(result.released.as_deref(), Some("2026-10-11"));
            }
            server.join().unwrap();
        }
        let silent = json!({"schema":1,"platform":"macos","revision":target_revision,"sequence":4,
            "releases":[entry(4,&target_revision,None)]});
        let (url, server) = serve_metadata(vec![
            (
                "/notes?platform=macos&revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                200,
                serde_json::to_vec(&current).unwrap(),
            ),
            (
                "/notes?platform=macos&revision=dddddddddddddddddddddddddddddddddddddddd",
                200,
                serde_json::to_vec(&silent).unwrap(),
            ),
        ]);
        assert_eq!(
            release_highlights_from(
                &client,
                &format!("{url}/notes"),
                "macos",
                &current_revision,
                1,
                &target_revision
            )
            .unwrap(),
            ReleaseHighlights {
                notes: "No user-facing changes in this build.".into(),
                complete: false,
                items: Some(Vec::new()),
                released: None,
                sequence: Some(4),
            }
        );
        server.join().unwrap();
    }

    #[test]
    fn skipped_releases_stack_into_one_list_without_repeated_titles() {
        use serde_json::json;
        let current_revision = "a".repeat(40);
        let target_revision = "c".repeat(40);
        let entry = |sequence, revision: &str, date: &str, items: &[(&str, &str)]| {
            json!({
                "sequence": sequence, "revision": revision, "publishedAt": format!("{date}T12:00:00Z"),
                "highlights": items.iter().map(|(title, body)| json!({"title": title, "body": body})).collect::<Vec<_>>()
            })
        };
        let current = json!({"schema":1,"platform":"windows","revision":current_revision,"sequence":1,
            "releases":[entry(1,&current_revision,"2026-10-10",&[])]});
        let target = json!({"schema":1,"platform":"windows","revision":target_revision,"sequence":3,
            "releases":[entry(1,&current_revision,"2026-10-10",&[]),
                entry(2,&"b".repeat(40),"2026-10-12",&[("Voice in the channel list","Older words."),("Clearer pins","Pins say who pinned them.")]),
                entry(3,&target_revision,"2026-10-14",&[("Voice in the Channel List ","Newest words.")])]});
        let (url, server) = serve_metadata(vec![
            (
                "/notes?platform=windows&revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                200,
                serde_json::to_vec(&current).unwrap(),
            ),
            (
                "/notes?platform=windows&revision=cccccccccccccccccccccccccccccccccccccccc",
                200,
                serde_json::to_vec(&target).unwrap(),
            ),
        ]);
        let result = release_highlights_from(
            &reqwest::blocking::Client::new(),
            &format!("{url}/notes"),
            "windows",
            &current_revision,
            1,
            &target_revision,
        )
        .unwrap();
        server.join().unwrap();
        let items: Vec<_> = result
            .items
            .unwrap()
            .into_iter()
            .map(|item| (item.title, item.body))
            .collect();
        assert_eq!(
            items,
            [
                (
                    "Voice in the Channel List ".to_owned(),
                    "Newest words.".to_owned()
                ),
                (
                    "Clearer pins".to_owned(),
                    "Pins say who pinned them.".to_owned()
                ),
            ]
        );
        assert_eq!(result.released.as_deref(), Some("2026-10-14"));
        assert_eq!(result.sequence, Some(3));
    }

    #[test]
    fn missing_installed_snapshot_falls_back_to_the_update_feed() {
        use serde_json::json;
        let current_revision = "a".repeat(40);
        let target_revision = "d".repeat(40);
        let entry = |sequence, revision: &str, version: Option<&str>, title: &str| {
            let mut value = json!({
                "sequence": sequence, "revision": revision, "publishedAt": "2026-10-18T12:00:00Z",
                "highlights": [{"title": title, "body": "A useful change."}]
            });
            if let Some(version) = version {
                value["version"] = json!(version);
            }
            value
        };
        let feed = |releases: Vec<serde_json::Value>| json!({"schema":1,"platform":"windows","revision":target_revision,"sequence":4,"releases":releases});
        let titles = |releases: Vec<serde_json::Value>, installed_build| {
            let (url, server) = serve_metadata(vec![
                (
                    "/notes?platform=windows&revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    502,
                    b"release highlights are not available yet".to_vec(),
                ),
                (
                    "/notes?platform=windows&revision=dddddddddddddddddddddddddddddddddddddddd",
                    200,
                    serde_json::to_vec(&feed(releases)).unwrap(),
                ),
            ]);
            let result = release_highlights_from(
                &reqwest::blocking::Client::new(),
                &format!("{url}/notes"),
                "windows",
                &current_revision,
                installed_build,
                &target_revision,
            )
            .unwrap();
            server.join().unwrap();
            let titles: Vec<_> = result
                .items
                .unwrap()
                .into_iter()
                .map(|item| item.title)
                .collect();
            (titles, result.complete, result.sequence)
        };
        // The installed revision is listed in the update's feed: exact position.
        assert_eq!(
            titles(
                vec![
                    entry(1, &"0".repeat(40), Some("0.1.110"), "Older"),
                    entry(2, &current_revision, Some("0.1.119"), "Installed"),
                    entry(3, &"b".repeat(40), Some("0.1.121"), "Skipped"),
                    entry(4, &target_revision, Some("0.1.124"), "Newest"),
                ],
                119
            ),
            (
                vec!["Newest".to_owned(), "Skipped".to_owned()],
                true,
                Some(4)
            )
        );
        // Not listed (its notes never published): releases with newer builds.
        assert_eq!(
            titles(
                vec![
                    entry(2, &"0".repeat(40), Some("0.1.119"), "Older"),
                    entry(3, &"b".repeat(40), Some("0.1.123"), "Skipped"),
                    entry(4, &target_revision, Some("0.1.124"), "Newest"),
                ],
                121
            ),
            (
                vec!["Newest".to_owned(), "Skipped".to_owned()],
                false,
                Some(4)
            )
        );
        // No build numbers to compare: only the update's own highlights.
        assert_eq!(
            titles(
                vec![
                    entry(3, &"b".repeat(40), None, "Skipped"),
                    entry(4, &target_revision, None, "Newest"),
                ],
                121
            ),
            (vec!["Newest".to_owned()], false, Some(4))
        );
    }

    #[test]
    fn release_versions_yield_build_numbers() {
        let release = |version: Option<&str>| HighlightedRelease {
            sequence: 1,
            revision: "a".repeat(40),
            version: version.map(str::to_owned),
            published_at: "2026-10-18T12:00:00Z".into(),
            highlights: Vec::new(),
        };
        assert_eq!(release(Some("0.1.121")).build(), Some(121));
        for version in [
            None,
            Some("0.1"),
            Some("0.1.12a"),
            Some("0.1.2.3"),
            Some("0.1."),
        ] {
            assert_eq!(release(version).build(), None, "{version:?}");
        }
    }

    #[test]
    fn only_confirmed_http_throttles_defer_checks() {
        let client = reqwest::blocking::Client::new();
        for (status, headers, deferred) in [
            (429, "", true),
            (403, "Retry-After: 120\r\n", true),
            (403, "X-RateLimit-Remaining: 0\r\n", true),
            (403, "X-RateLimit-Remaining: 10\r\n", false),
            (403, "", false),
            (500, "Retry-After: 120\r\n", false),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                write!(stream, "HTTP/1.1 {status} Test\r\n{headers}Content-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            });
            let error = fetch_limited(&client, &url, 1024).unwrap_err();
            assert_eq!(error.is::<RateLimited>(), deferred, "{status} {headers}");
            server.join().unwrap();
        }
    }

    #[test]
    fn cached_metadata_is_verified_and_invalid_site_responses_fall_back_to_github() {
        let key = SigningKey::from_bytes(&[11; 32]);
        // Keep whitespace/newline: signatures cover bytes, not reserialized JSON.
        let bytes = b"{\n  \"schema\":1,\"build\":17,\"version\":\"0.1.17\",\"commit\":\"abc\",\"platforms\":{}\n}\n";
        let signature = manifest::sign(bytes, &key);
        let valid = serde_json::to_vec(&serde_json::json!({
            "manifest": STANDARD.encode(bytes), "signature": signature,
        }))
        .unwrap();
        let mixed = serde_json::to_vec(&serde_json::json!({
            "manifest": STANDARD.encode(bytes),
            "signature": manifest::sign(b"a different release", &key),
        }))
        .unwrap();
        let oversized = vec![b'a'; 2 * manifest::MAX_MANIFEST_BYTES + 1];
        let client = reqwest::blocking::Client::new();
        for (status, cached, fallback) in [
            (200, valid, false),
            (502, b"unavailable".to_vec(), true),
            (429, b"rate limited".to_vec(), true),
            (200, b"{}".to_vec(), true),
            (200, mixed, true),
            (200, oversized, true),
        ] {
            let mut responses = vec![("/cached", status, cached)];
            if fallback {
                responses.extend([
                    ("/latest.json", 200, bytes.to_vec()),
                    ("/latest.json.sig", 200, signature.as_bytes().to_vec()),
                ]);
            }
            let (url, server) = serve_metadata(responses);
            let manifest = fetch_manifest_from(
                &client,
                &key.verifying_key(),
                &format!("{url}/cached"),
                &format!("{url}/latest.json"),
            )
            .unwrap();
            assert_eq!(manifest.build, 17);
            assert_eq!(manifest.version, "0.1.17");
            server.join().unwrap();
        }
        // Fallback is not permission to accept an unsigned or mismatched release.
        let (url, server) = serve_metadata(vec![
            ("/cached", 502, vec![]),
            ("/latest.json", 200, bytes.to_vec()),
            (
                "/latest.json.sig",
                200,
                manifest::sign(b"tampered", &key).into_bytes(),
            ),
        ]);
        assert!(
            fetch_manifest_from(
                &client,
                &key.verifying_key(),
                &format!("{url}/cached"),
                &format!("{url}/latest.json")
            )
            .is_err()
        );
        server.join().unwrap();
    }

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
    fn relocated_updater_runs_from_its_temporary_folder() {
        let _child_process_test = CHILD_PROCESS_TEST.lock().unwrap();
        const REPORT: &str = "CAPER_TEST_RELOCATION_REPORT";
        const INSTALL: &str = "CAPER_TEST_RELOCATION_INSTALL";
        let args = [
            "--exact",
            "tests::relocated_updater_runs_from_its_temporary_folder",
            "--",
        ]
        .map(OsString::from);
        if let Some(report) = env::var_os(REPORT) {
            if !env::args_os().any(|arg| arg == RELOCATED_FLAG) {
                // Exercise apply's real handoff decision, not just the copying
                // helper. On Windows current_exe may omit canonicalize's \\?\.
                apply(
                    &Options {
                        current_build: Some(1),
                        install: Some(PathBuf::from(env::var_os(INSTALL).unwrap())),
                        ..Options::default()
                    },
                    &args,
                )
                .unwrap();
                return;
            }
            #[cfg(unix)]
            {
                use std::os::unix::{
                    fs::{FileTypeExt, MetadataExt},
                    io::AsFd,
                };
                // SAFETY: getpgrp only reads the current process group.
                assert_eq!(unsafe { libc::getpgrp() } as u32, std::process::id());
                let null = fs::metadata("/dev/null").unwrap();
                for fd in [
                    io::stdin().as_fd().try_clone_to_owned().unwrap(),
                    io::stdout().as_fd().try_clone_to_owned().unwrap(),
                    io::stderr().as_fd().try_clone_to_owned().unwrap(),
                ] {
                    let metadata = fs::File::from(fd).metadata().unwrap();
                    assert!(metadata.file_type().is_char_device());
                    assert_eq!(metadata.rdev(), null.rdev());
                }
            }
            let report = PathBuf::from(report);
            fs::write(
                report.with_extension("tmp"),
                serde_json::to_vec(&env::current_dir().unwrap().canonicalize().unwrap()).unwrap(),
            )
            .unwrap();
            fs::rename(report.with_extension("tmp"), report).unwrap();
            return;
        }

        let report_folder = tempfile::tempdir().unwrap();
        let report = report_folder.path().join("cwd.json");
        let install = report_folder.path().join("Caper with spaces");
        let layout = Layout::for_platform(manifest::current_platform().unwrap()).unwrap();
        let (app, updater) = match layout {
            Layout::MacBundle => ("Contents/Info.plist", "Contents/MacOS/caper-updater"),
            Layout::WindowsFolder => ("Caper.exe", "caper-updater.exe"),
            Layout::LinuxTarball => ("caper-desktop", "caper-updater"),
        };
        let updater = install.join(updater);
        fs::create_dir_all(updater.parent().unwrap()).unwrap();
        fs::write(install.join(app), "untouched app").unwrap();
        fs::copy(env::current_exe().unwrap(), &updater).unwrap();
        // Run only this test in the copied executable. After `--`, libtest
        // accepts the updater's --relocated flag as an additional test filter.
        // Use the canonical path too: this is exactly how the Windows app starts
        // its updater, and does not guarantee current_exe uses the same prefix.
        let mut child = Command::new(updater.canonicalize().unwrap())
            .args(&args)
            .current_dir(&install)
            .env(REPORT, &report)
            .env(INSTALL, &install)
            .spawn()
            .unwrap();
        let status = child.wait().unwrap();
        assert!(status.success(), "relocated updater failed: {status}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !report.exists() {
            assert!(
                Instant::now() < deadline,
                "relocated updater did not report"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let working_folder: PathBuf = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        // Check the actual child process, not only Command's configuration.
        assert!(is_relocation_folder(&working_folder));
        assert!(!working_folder.starts_with(install.canonicalize().unwrap()));
        let relocated_folder = working_folder;
        assert_eq!(
            fs::read_to_string(install.join(app)).unwrap(),
            "untouched app"
        );
        // A handoff interrupted before install leaves actionable feedback. A
        // manually installed newer build must not inherit the old failure.
        assert!(
            last_failure(&install, 1)
                .unwrap()
                .contains("did not finish")
        );
        assert!(last_failure(&install, 2).is_none());
        // The reporting child can still be exiting (Windows locks its exe).
        while install::remove_if_present(&relocated_folder).is_err() {
            assert!(Instant::now() < deadline, "relocated updater did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn each_relocation_gets_a_new_private_folder() {
        let folders: Vec<PathBuf> = (0..2)
            .map(|_| {
                let command = relocate_command(&[]).unwrap();
                let folder = command.get_current_dir().unwrap().to_path_buf();
                assert_eq!(
                    command.get_program(),
                    folder.join(env::current_exe().unwrap().file_name().unwrap())
                );
                folder
            })
            .collect();
        assert_ne!(folders[0], folders[1], "names must not be predictable");
        for folder in &folders {
            assert!(is_relocation_folder(folder));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(folder).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o700);
            }
            install::remove_if_present(folder).unwrap();
        }
        // Cleanup never reaches an install or the temp folder itself.
        assert!(!is_relocation_folder(&env::temp_dir()));
        assert!(!is_relocation_folder(&env::temp_dir().join("Caper")));
        assert!(!is_relocation_folder(Path::new("/opt/caper-updater-x")));
    }

    #[test]
    fn linux_log_lives_in_the_users_state_folder() {
        assert_eq!(
            linux_log_folder(Some("/state".into()), Some("/home/a".into())),
            Some(PathBuf::from("/state/caper"))
        );
        assert_eq!(
            linux_log_folder(Some("relative".into()), Some("/home/a".into())),
            Some(PathBuf::from("/home/a/.local/state/caper"))
        );
        assert_eq!(linux_log_folder(None, Some("relative".into())), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn replaced_app_reopens_from_its_install_without_the_old_process_context() {
        let _child_process_test = CHILD_PROCESS_TEST.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("Caper with spaces");
        fs::create_dir(&install).unwrap();
        fs::write(install.join("caper-desktop"), "old app").unwrap();
        let archive = root.path().join("update.tar.gz");
        // Preserve the inherited descriptors before report redirections alter
        // the shell's stdout; inspect those, not the report file's descriptor.
        let app = br#"#!/bin/sh
set -eu
exec 3<&0 4>&1 5>&2
printf 'updated\n' > launch-report
pwd >> launch-report
printf '%s\n' "$$" >> launch-report
ps -o pgid= -p "$$" >> launch-report
for fd in 3 4 5; do readlink "/proc/$$/fd/$fd" >> launch-report; done
mv launch-report launch-report.ready
"#;
        let encoder = flate2::write::GzEncoder::new(
            fs::File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tarball = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(app.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tarball
            .append_data(&mut header, "Caper-linux-x64/caper-desktop", &app[..])
            .unwrap();
        tarball.into_inner().unwrap().finish().unwrap();
        let stage = install::sibling(&install, "update").unwrap();
        let replacement = install::unpack(Layout::LinuxTarball, &archive, &stage).unwrap();
        let old = install::swap(&install, &replacement).unwrap();
        install::remove_if_present(&stage).unwrap();
        install::remove_if_present(&old).unwrap();
        relaunch(Layout::LinuxTarball, &install).unwrap();

        let report = install.join("launch-report.ready");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !report.exists() {
            assert!(Instant::now() < deadline, "updated app did not reopen");
            thread::sleep(Duration::from_millis(10));
        }
        let contents = fs::read_to_string(report).unwrap();
        let lines: Vec<_> = contents.lines().map(str::trim).collect();
        assert_eq!(lines[0], "updated");
        assert_eq!(Path::new(lines[1]), install.canonicalize().unwrap());
        assert_eq!(lines[2], lines[3], "app needs its own process group");
        assert_eq!(&lines[4..], &["/dev/null", "/dev/null", "/dev/null"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejected_mac_launch_is_reported_as_an_error() {
        let _child_process_test = CHILD_PROCESS_TEST.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("Missing Caper.app");
        assert!(relaunch(Layout::MacBundle, &missing).is_err());
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
        let _child_process_test = CHILD_PROCESS_TEST.lock().unwrap();
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
