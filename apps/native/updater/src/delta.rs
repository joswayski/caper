//! Patch updates: rebuild the new release from the installed one.
//!
//! A patch turns one published build into the newest one for one platform. It
//! lists every directory, file and symlink of the new release's archive, as the
//! full archive unpacks, with each file's size and SHA-256. Each file is
//! - copied from the installed app when its bytes did not change,
//! - a bsdiff patch (bzip2-compressed) of the installed file at a given path, or
//! - its whole content, bzip2-compressed, when the installed app has no base.
//!
//! The signed manifest pins the patch's own SHA-256 and size, so everything in it
//! is as trusted as the full archive. The updater rebuilds the release in the
//! private stage folder, hashes every installed file it reads, then checks that
//! the stage holds exactly the listed entries with exactly the listed bytes
//! before it swaps anything. Any mismatch falls back to the full archive.
//!
//! Layout: `CAPERPATCH1\n`, the header length (u32, little endian), the JSON
//! [`Header`], then the data that entries point into by offset and length.
//!
//! Release tooling makes patches with `caper-updater delta` from the published
//! archives, and rebuilds the target from an unpacked base before publishing.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::install::{self, Layout};

const MAGIC: &[u8; 12] = b"CAPERPATCH1\n";
const SCHEMA: u32 = 1;
const MAX_HEADER_BYTES: u32 = 4 * 1024 * 1024;
/// Largest single file a patch may rebuild in memory.
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub schema: u32,
    pub platform: String,
    pub from_build: u64,
    pub to_build: u64,
    /// The archive's single top-level folder (`Caper.app`, `Caper-linux-x64`),
    /// or empty when the archive holds the app's files at its root (Windows).
    pub root: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Entry {
    Dir {
        path: String,
        /// Unix permission bits; `None` where the archive records none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<u32>,
    },
    File {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<u32>,
        size: u64,
        sha256: String,
        source: Source,
    },
    Symlink {
        path: String,
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    /// The installed file at `base` already has this entry's bytes.
    Copy { base: String },
    /// A bsdiff patch from the installed file at `base`.
    Patch {
        base: String,
        base_size: u64,
        base_sha256: String,
        offset: u64,
        length: u64,
    },
    /// The whole file, bzip2-compressed.
    Full { offset: u64, length: u64 },
}

impl Entry {
    fn path(&self) -> &str {
        match self {
            Self::Dir { path, .. } | Self::File { path, .. } | Self::Symlink { path, .. } => path,
        }
    }
}

/// A file, folder or link as the archive unpacks, without where its bytes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Dir {
        mode: Option<u32>,
    },
    File {
        mode: Option<u32>,
        size: u64,
        sha256: String,
    },
    Symlink {
        target: String,
    },
}

fn error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// What the signed manifest says this patch must be.
pub struct Expected<'a> {
    pub platform: &'a str,
    pub from_build: u64,
    pub to_build: u64,
}

/// Rebuilds the release described by `patch` from `install` in a fresh `stage`
/// folder and returns the new app root inside it, which replaces `install`.
pub fn apply(
    layout: Layout,
    patch: &Path,
    expected: &Expected,
    install: &Path,
    stage: &Path,
) -> io::Result<PathBuf> {
    let mut bundle = fs::File::open(patch)?;
    let (header, data_start) = read_header(&mut bundle)?;
    if header.platform != expected.platform
        || header.from_build != expected.from_build
        || header.to_build != expected.to_build
    {
        return Err(error("patch is for a different build or platform"));
    }
    validate(&header, layout)?;
    install::create_stage(stage)?;
    let root = if header.root.is_empty() {
        stage.to_path_buf()
    } else {
        let root = stage.join(&header.root);
        fs::create_dir(&root)?;
        set_mode(&root, Some(0o755))?;
        root
    };
    // Folders, then files, then links, so nothing is written through a link.
    for entry in &header.entries {
        if let Entry::Dir { path, .. } = entry {
            fs::create_dir_all(join(&root, path))?;
        }
    }
    for entry in &header.entries {
        if let Entry::File {
            path,
            mode,
            size,
            sha256,
            source,
        } = entry
        {
            let target = join(&root, path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let output = fs::File::create_new(&target)?;
            let mut output = Hashing::new(io::BufWriter::new(output));
            match source {
                Source::Copy { base } => {
                    let mut input = open_base(install, base)?;
                    io::copy(&mut input, &mut output)?;
                }
                Source::Patch {
                    base,
                    base_size,
                    base_sha256,
                    offset,
                    length,
                } => {
                    let base_bytes = read_base(install, base, *base_size, base_sha256)?;
                    let delta = read_data(&mut bundle, data_start, *offset, *length)?;
                    qbsdiff::Bspatch::new(&delta)?.apply(&base_bytes, &mut output)?;
                }
                Source::Full { offset, length } => {
                    let compressed = read_data(&mut bundle, data_start, *offset, *length)?;
                    let mut decoder = bzip2::read::BzDecoder::new(&compressed[..]);
                    io::copy(&mut (&mut decoder).take(size + 1), &mut output)?;
                }
            }
            let (written, digest) = output.finish()?;
            if written != *size || digest != *sha256 {
                return Err(error(format!("{path} does not match the release")));
            }
            set_mode(&target, *mode)?;
        }
    }
    for entry in &header.entries {
        if let Entry::Symlink { path, target } = entry {
            symlink(target, &join(&root, path))?;
        }
    }
    // Deepest first, so a read-only folder never blocks its children.
    for entry in header.entries.iter().rev() {
        if let Entry::Dir { path, mode } = entry {
            set_mode(&join(&root, path), *mode)?;
        }
    }
    verify_tree(&root, &nodes(&header.entries))?;
    if !install::is_self_contained(layout, &root) {
        return Err(error("patched app is not a complete Caper install"));
    }
    Ok(root)
}

fn read_header(bundle: &mut fs::File) -> io::Result<(Header, u64)> {
    let mut magic = [0; 12];
    bundle.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(error("not a Caper patch"));
    }
    let mut length = [0; 4];
    bundle.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length);
    if length > MAX_HEADER_BYTES {
        return Err(error("patch header is too large"));
    }
    let mut json = vec![0; length as usize];
    bundle.read_exact(&mut json)?;
    let header: Header = serde_json::from_slice(&json).map_err(|e| error(e.to_string()))?;
    if header.schema != SCHEMA {
        return Err(error("unsupported patch schema"));
    }
    Ok((header, (MAGIC.len() + 4) as u64 + u64::from(length)))
}

fn read_data(bundle: &mut fs::File, start: u64, offset: u64, length: u64) -> io::Result<Vec<u8>> {
    let end = start
        .checked_add(offset)
        .and_then(|position| position.checked_add(length))
        .ok_or_else(|| error("patch data is out of range"))?;
    if length > MAX_FILE_BYTES || end > bundle.metadata()?.len() {
        return Err(error("patch data is out of range"));
    }
    bundle.seek(SeekFrom::Start(start + offset))?;
    let mut bytes = vec![0; length as usize];
    bundle.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// An installed regular file; never a link the old install may hold there.
fn open_base(install: &Path, path: &str) -> io::Result<fs::File> {
    let path = join(install, path);
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(error(format!("{} is not a file", path.display())));
    }
    fs::File::open(path)
}

fn read_base(install: &Path, path: &str, size: u64, sha256: &str) -> io::Result<Vec<u8>> {
    if size > MAX_FILE_BYTES {
        return Err(error("patch base is too large"));
    }
    let mut bytes = Vec::new();
    open_base(install, path)?
        .take(size + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != size || hex(&Sha256::digest(&bytes)) != sha256 {
        return Err(error(format!(
            "installed {path} is not the build this patch expects"
        )));
    }
    Ok(bytes)
}

/// Every path must stay inside the app and appear once (also ignoring case,
/// as Mac and Windows file systems do), and links must point inside the app.
fn validate(header: &Header, layout: Layout) -> io::Result<()> {
    let root = &header.root;
    let root_ok = match layout {
        Layout::WindowsFolder => root.is_empty(),
        Layout::MacBundle => safe_path(root) && !root.contains('/') && root.ends_with(".app"),
        Layout::LinuxTarball => safe_path(root) && !root.contains('/'),
    };
    if !root_ok {
        return Err(error("patch has an unexpected app folder"));
    }
    let mut seen = BTreeSet::new();
    let mut links = Vec::new();
    for entry in &header.entries {
        let path = entry.path();
        if !safe_path(path) || !seen.insert(path.to_lowercase()) {
            return Err(error(format!(
                "patch has an unsafe or repeated path {path:?}"
            )));
        }
        match entry {
            Entry::Symlink { target, .. } => {
                if cfg!(windows) || !link_stays_inside(path, target) {
                    return Err(error(format!("patch has an unsupported link {path:?}")));
                }
                links.push(path.to_lowercase());
            }
            Entry::File {
                sha256,
                source,
                size,
                ..
            } => {
                let base_ok = match source {
                    Source::Copy { base } => safe_path(base),
                    Source::Patch {
                        base, base_sha256, ..
                    } => safe_path(base) && is_sha256(base_sha256),
                    Source::Full { .. } => true,
                };
                if !is_sha256(sha256) || !base_ok || *size > MAX_FILE_BYTES {
                    return Err(error(format!("patch entry {path:?} is invalid")));
                }
            }
            Entry::Dir { .. } => {}
        }
    }
    // Nothing may be written beneath a link: the link is created last and the
    // full archive could not hold both either.
    for path in &seen {
        if links
            .iter()
            .any(|link| path.len() > link.len() && path.starts_with(&format!("{link}/")))
        {
            return Err(error(format!("patch writes beneath a link at {path:?}")));
        }
    }
    Ok(())
}

/// Relative, `/`-separated, without empty, `.` or `..` parts, and without
/// characters Windows reads as separators, drives or streams.
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.contains(['\\', ':', '\0'])
        })
}

fn link_stays_inside(path: &str, target: &str) -> bool {
    if target.is_empty() || target.starts_with('/') || target.contains(['\\', ':', '\0']) {
        return false;
    }
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return false;
                }
            }
            part => parts.push(part),
        }
    }
    true
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn join(root: &Path, path: &str) -> PathBuf {
    path.split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn nodes(entries: &[Entry]) -> BTreeMap<String, Node> {
    entries
        .iter()
        .map(|entry| {
            let node = match entry {
                Entry::Dir { mode, .. } => Node::Dir { mode: *mode },
                Entry::File {
                    mode, size, sha256, ..
                } => Node::File {
                    mode: *mode,
                    size: *size,
                    sha256: sha256.clone(),
                },
                Entry::Symlink { target, .. } => Node::Symlink {
                    target: target.clone(),
                },
            };
            (entry.path().to_owned(), node)
        })
        .collect()
}

/// `root` must hold exactly `expected`, plus folders implied by its paths: no
/// extra or missing entries, and every file re-read from disk and hashed.
fn verify_tree(root: &Path, expected: &BTreeMap<String, Node>) -> io::Result<()> {
    let mut wanted = expected.clone();
    for path in expected.keys() {
        let mut parent = path.as_str();
        while let Some((folder, _)) = parent.rsplit_once('/') {
            wanted
                .entry(folder.to_owned())
                .or_insert(Node::Dir { mode: None });
            parent = folder;
        }
    }
    let mut found = BTreeMap::new();
    walk(root, "", &mut found)?;
    if found.len() != wanted.len() {
        return Err(error("rebuilt app has missing or extra files"));
    }
    for (path, node) in &wanted {
        let actual = found
            .remove(path)
            .ok_or_else(|| error(format!("rebuilt app is missing {path}")))?;
        let matches = match (node, &actual) {
            (Node::Dir { mode }, Node::Dir { mode: actual }) => mode_matches(*mode, *actual),
            (
                Node::File { mode, size, sha256 },
                Node::File {
                    mode: actual_mode,
                    size: actual_size,
                    ..
                },
            ) => {
                mode_matches(*mode, *actual_mode)
                    && size == actual_size
                    && install::sha256_file(&join(root, path))? == *sha256
            }
            (Node::Symlink { target }, Node::Symlink { target: actual }) => target == actual,
            _ => false,
        };
        if !matches {
            return Err(error(format!("rebuilt {path} does not match the release")));
        }
    }
    Ok(())
}

fn mode_matches(expected: Option<u32>, actual: Option<u32>) -> bool {
    !cfg!(unix) || expected.is_none() || expected == actual
}

fn walk(folder: &Path, prefix: &str, found: &mut BTreeMap<String, Node>) -> io::Result<()> {
    for item in fs::read_dir(folder)? {
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| error("rebuilt app has a non-UTF-8 name"))?;
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let metadata = fs::symlink_metadata(item.path())?;
        let node = if metadata.file_type().is_symlink() {
            let target = fs::read_link(item.path())?;
            Node::Symlink {
                target: target
                    .to_str()
                    .ok_or_else(|| error("link target is not UTF-8"))?
                    .to_owned(),
            }
        } else if metadata.is_dir() {
            walk(&item.path(), &path, found)?;
            Node::Dir {
                mode: unix_mode(&metadata),
            }
        } else {
            Node::File {
                mode: unix_mode(&metadata),
                size: metadata.len(),
                sha256: String::new(),
            }
        };
        found.insert(path, node);
    }
    Ok(())
}

#[cfg(unix)]
fn unix_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn unix_mode(_: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match mode {
        Some(mode) => fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777)),
        None => Ok(()),
    }
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: Option<u32>) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn symlink(target: &str, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_: &str, _: &Path) -> io::Result<()> {
    Err(error("links are not supported on this platform"))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Counts and hashes everything written through it.
struct Hashing<W> {
    inner: W,
    hasher: Sha256,
    written: u64,
}

impl<W: Write> Hashing<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            written: 0,
        }
    }

    fn finish(mut self) -> io::Result<(u64, String)> {
        self.inner.flush()?;
        Ok((self.written, hex(&self.hasher.finalize())))
    }
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

// Release tooling: making a patch from two published archives.

/// What `caper-updater delta` reports about the patch it wrote.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub size: u64,
    pub sha256: String,
    pub copied: usize,
    pub patched: usize,
    pub full: usize,
}

/// Writes the patch from `base` (build `from_build`) to `target` and proves it
/// by rebuilding `target` from the unpacked base, exactly as a client does.
pub fn generate(
    platform: &str,
    base: &Path,
    from_build: u64,
    target: &Path,
    to_build: u64,
    output: &Path,
) -> io::Result<Summary> {
    let layout = Layout::for_platform(platform).ok_or_else(|| error("unknown platform"))?;
    if from_build >= to_build {
        return Err(error("a patch must go to a newer build"));
    }
    let work = tempfile::tempdir()?;
    let (base_root, base_tree) = extract(layout, base, &work.path().join("base"))?;
    let (target_root, target_tree) = extract(layout, target, &work.path().join("target"))?;
    let base_files: HashMap<&str, &str> = base_tree
        .iter()
        .filter_map(|(path, node)| match node {
            Node::File { sha256, .. } => Some((sha256.as_str(), path.as_str())),
            _ => None,
        })
        .collect();

    let mut data = Vec::new();
    let mut entries = Vec::new();
    let (mut copied, mut patched, mut full) = (0, 0, 0);
    for (path, node) in &target_tree {
        entries.push(match node {
            Node::Dir { mode } => Entry::Dir {
                path: path.clone(),
                mode: *mode,
            },
            Node::Symlink { target } => Entry::Symlink {
                path: path.clone(),
                target: target.clone(),
            },
            Node::File { mode, size, sha256 } => {
                let same_path = matches!(base_tree.get(path), Some(Node::File { sha256: base, .. }) if base == sha256);
                let source = if same_path {
                    copied += 1;
                    Source::Copy { base: path.clone() }
                } else if let Some(base) = base_files.get(sha256.as_str()) {
                    copied += 1;
                    Source::Copy {
                        base: (*base).to_owned(),
                    }
                } else {
                    let bytes = fs::read(join(&target_root, path))?;
                    let offset = data.len() as u64;
                    if let Some(Node::File {
                        size: base_size,
                        sha256: base_sha256,
                        ..
                    }) = base_tree.get(path)
                    {
                        patched += 1;
                        let base_bytes = fs::read(join(&base_root, path))?;
                        qbsdiff::Bsdiff::new(&base_bytes, &bytes)
                            .small_match(0)
                            .compression_level(9)
                            .compare(&mut data)?;
                        Source::Patch {
                            base: path.clone(),
                            base_size: *base_size,
                            base_sha256: base_sha256.clone(),
                            offset,
                            length: data.len() as u64 - offset,
                        }
                    } else {
                        full += 1;
                        let mut encoder =
                            bzip2::write::BzEncoder::new(&mut data, bzip2::Compression::best());
                        encoder.write_all(&bytes)?;
                        encoder.finish()?;
                        Source::Full {
                            offset,
                            length: data.len() as u64 - offset,
                        }
                    }
                };
                Entry::File {
                    path: path.clone(),
                    mode: *mode,
                    size: *size,
                    sha256: sha256.clone(),
                    source,
                }
            }
        });
    }
    let header = Header {
        schema: SCHEMA,
        platform: platform.to_owned(),
        from_build,
        to_build,
        root: target_root
            .strip_prefix(work.path().join("target"))
            .ok()
            .and_then(|root| root.to_str())
            .unwrap_or_default()
            .to_owned(),
        entries,
    };
    let json = serde_json::to_vec(&header).map_err(io::Error::other)?;
    let length = u32::try_from(json.len())
        .ok()
        .filter(|length| *length <= MAX_HEADER_BYTES)
        .ok_or_else(|| error("patch header is too large"))?;
    let mut file = io::BufWriter::new(fs::File::create_new(output)?);
    file.write_all(MAGIC)?;
    file.write_all(&length.to_le_bytes())?;
    file.write_all(&json)?;
    file.write_all(&data)?;
    file.into_inner().map_err(|e| e.into_error())?.sync_all()?;

    // The same code path a client runs, against the base as it unpacks.
    let stage = work.path().join("stage");
    let rebuilt = apply(
        layout,
        output,
        &Expected {
            platform,
            from_build,
            to_build,
        },
        &base_root,
        &stage,
    )?;
    debug_assert!(rebuilt.ends_with(&header.root));
    Ok(Summary {
        size: fs::metadata(output)?.len(),
        sha256: install::sha256_file(output)?,
        copied,
        patched,
        full,
    })
}

/// Unpacks a release archive the way the full update does (ditto keeps zip
/// modes and links on Mac) and lists what it holds, relative to the app root.
fn extract(
    layout: Layout,
    archive: &Path,
    destination: &Path,
) -> io::Result<(PathBuf, BTreeMap<String, Node>)> {
    let mut raw = Vec::new();
    let unix_modes = layout != Layout::WindowsFolder;
    match layout {
        Layout::MacBundle | Layout::WindowsFolder => {
            let mut zip =
                zip::ZipArchive::new(fs::File::open(archive)?).map_err(io::Error::other)?;
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index).map_err(io::Error::other)?;
                let name = entry
                    .name()
                    .map_err(io::Error::other)?
                    .trim_end_matches('/')
                    .to_owned();
                let mode = entry
                    .unix_mode()
                    .map(|mode| mode & 0o777)
                    .filter(|_| unix_modes);
                if entry.is_dir() {
                    raw.push((name, RawKind::Dir, mode));
                } else {
                    let mut bytes = Vec::new();
                    entry.read_to_end(&mut bytes)?;
                    if entry.is_symlink() {
                        let target = String::from_utf8(bytes)
                            .map_err(|_| error("link target is not UTF-8"))?;
                        raw.push((name, RawKind::Symlink(target), None));
                    } else {
                        raw.push((name, RawKind::File(bytes), mode));
                    }
                }
            }
        }
        Layout::LinuxTarball => {
            let mut tarball =
                tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(archive)?));
            for entry in tarball.entries()? {
                let mut entry = entry?;
                let name = entry
                    .path()?
                    .to_str()
                    .ok_or_else(|| error("archive path is not UTF-8"))?
                    .trim_end_matches('/')
                    .trim_start_matches("./")
                    .to_owned();
                let mode = Some(entry.header().mode()? & 0o777);
                match entry.header().entry_type() {
                    tar::EntryType::Directory => raw.push((name, RawKind::Dir, mode)),
                    tar::EntryType::Regular | tar::EntryType::Continuous => {
                        let mut bytes = Vec::new();
                        entry.read_to_end(&mut bytes)?;
                        raw.push((name, RawKind::File(bytes), mode));
                    }
                    tar::EntryType::Symlink => {
                        let target = entry
                            .link_name()?
                            .and_then(|target| target.to_str().map(str::to_owned))
                            .ok_or_else(|| error("link target is not UTF-8"))?;
                        raw.push((name, RawKind::Symlink(target), None));
                    }
                    other => return Err(error(format!("unsupported archive entry {other:?}"))),
                }
            }
        }
    }
    let root = match layout {
        Layout::WindowsFolder => String::new(),
        Layout::MacBundle | Layout::LinuxTarball => {
            let roots: BTreeSet<&str> = raw
                .iter()
                .map(|(name, _, _)| name.split('/').next().unwrap_or_default())
                .collect();
            let roots: Vec<&str> = roots.into_iter().collect();
            let [root] = roots.as_slice() else {
                return Err(error("archive must hold exactly one app folder"));
            };
            if layout == Layout::MacBundle && !root.ends_with(".app") {
                return Err(error("Mac archive must hold an .app"));
            }
            (*root).to_owned()
        }
    };
    let root_dir = if root.is_empty() {
        destination.to_path_buf()
    } else {
        destination.join(&root)
    };
    fs::create_dir_all(&root_dir)?;
    let mut tree = BTreeMap::new();
    let mut files = Vec::new();
    for (name, kind, mode) in raw {
        let path = if root.is_empty() {
            name
        } else if name == root {
            continue;
        } else {
            name.strip_prefix(&format!("{root}/"))
                .ok_or_else(|| error("archive entry is outside the app folder"))?
                .to_owned()
        };
        if !safe_path(&path) || tree.contains_key(&path) {
            return Err(error(format!(
                "archive has an unsafe or repeated path {path:?}"
            )));
        }
        let node = match kind {
            RawKind::Dir => {
                fs::create_dir_all(join(&root_dir, &path))?;
                Node::Dir { mode }
            }
            RawKind::File(bytes) => {
                let sha256 = hex(&Sha256::digest(&bytes));
                let size = bytes.len() as u64;
                files.push((path.clone(), bytes, mode));
                Node::File { mode, size, sha256 }
            }
            RawKind::Symlink(target) => {
                if !link_stays_inside(&path, &target) {
                    return Err(error(format!("archive link {path:?} leaves the app")));
                }
                Node::Symlink { target }
            }
        };
        tree.insert(path, node);
    }
    for (path, bytes, mode) in files {
        let file = join(&root_dir, &path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file, bytes)?;
        set_mode(&file, mode)?;
    }
    for (path, node) in &tree {
        if let Node::Symlink { target } = node {
            symlink(target, &join(&root_dir, path))?;
        }
    }
    for (path, node) in tree.iter().rev() {
        if let Node::Dir { mode } = node {
            set_mode(&join(&root_dir, path), *mode)?;
        }
    }
    // Folders the archive only implies have no recorded mode, as on unpack.
    for path in tree.keys().cloned().collect::<Vec<_>>() {
        let mut parent = path.as_str();
        while let Some((folder, _)) = parent.rsplit_once('/') {
            tree.entry(folder.to_owned())
                .or_insert(Node::Dir { mode: None });
            parent = folder;
        }
    }
    Ok((root_dir, tree))
}

enum RawKind {
    Dir,
    File(Vec<u8>),
    Symlink(String),
}

/// A Windows release zip holding `files` at its root.
#[cfg(test)]
pub(crate) fn test_zip(path: &Path, files: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

/// A Linux release tarball (`Caper-linux-x64/…`) holding `files` and `links`.
#[cfg(all(test, unix))]
pub(crate) fn test_tarball(path: &Path, files: &[(&str, &[u8], u32)], links: &[(&str, &str)]) {
    let encoder =
        flate2::write::GzEncoder::new(fs::File::create(path).unwrap(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    for (name, bytes, mode) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("Caper-linux-x64/{name}"), *bytes)
            .unwrap();
    }
    for (name, target) in links {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        builder
            .append_link(&mut header, format!("Caper-linux-x64/{name}"), target)
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Large enough that a binary patch is much smaller than the file.
    pub(crate) fn program(seed: u8) -> Vec<u8> {
        (0..200_000u32)
            .map(|index| (index.wrapping_mul(2_654_435_761) >> 13) as u8 ^ seed)
            .collect()
    }

    #[cfg(unix)]
    fn linux_releases(root: &Path) -> (PathBuf, PathBuf) {
        let old = program(0);
        let mut new = old.clone();
        new[1_000..1_040].copy_from_slice(&[7; 40]);
        new.extend_from_slice(b"one more function");
        let base = root.join("base.tar.gz");
        test_tarball(
            &base,
            &[
                ("caper-desktop", &old, 0o755),
                ("caper-updater", b"updater", 0o755),
                ("README.md", b"read me", 0o644),
                ("libfoo.so.1", b"library", 0o755),
                ("removed.txt", b"gone in the update", 0o644),
            ],
            &[("libfoo.so", "libfoo.so.1")],
        );
        let target = root.join("target.tar.gz");
        test_tarball(
            &target,
            &[
                ("caper-desktop", &new, 0o755),
                ("caper-updater", b"updater", 0o755),
                // Same bytes at a new path: copied, not downloaded.
                ("docs/README.md", b"read me", 0o600),
                ("libfoo.so.1", b"library", 0o755),
                ("new-notice.txt", b"a brand new file", 0o644),
            ],
            &[("libfoo.so", "libfoo.so.1")],
        );
        (base, target)
    }

    #[cfg(unix)]
    fn installed(root: &Path, archive: &Path, name: &str) -> PathBuf {
        let unpacked = install::unpack(Layout::LinuxTarball, archive, &root.join(name)).unwrap();
        // Hidden-name stages are private (0700); an install is not.
        set_mode(&unpacked, Some(0o755)).unwrap();
        unpacked
    }

    #[cfg(unix)]
    fn expected(from_build: u64, to_build: u64) -> Expected<'static> {
        Expected {
            platform: "linux-x64",
            from_build,
            to_build,
        }
    }

    #[cfg(unix)]
    #[test]
    fn patches_rebuild_the_exact_release_from_the_installed_build() {
        let root = tempfile::tempdir().unwrap();
        let (base, target) = linux_releases(root.path());
        let patch = root.path().join("update.caperpatch");
        let summary = generate("linux-x64", &base, 7, &target, 9, &patch).unwrap();
        assert_eq!((summary.copied, summary.patched, summary.full), (3, 1, 1));
        assert!(summary.size < 20_000, "{} bytes", summary.size);
        assert_eq!(summary.sha256, install::sha256_file(&patch).unwrap());

        let install = installed(root.path(), &base, "install");
        let stage = root.path().join("stage");
        let rebuilt = apply(
            Layout::LinuxTarball,
            &patch,
            &expected(7, 9),
            &install,
            &stage,
        )
        .unwrap();
        assert_eq!(rebuilt, stage.join("Caper-linux-x64"));
        let full = installed(root.path(), &target, "full");
        let mut from_patch = BTreeMap::new();
        let mut from_full = BTreeMap::new();
        walk(&rebuilt, "", &mut from_patch).unwrap();
        walk(&full, "", &mut from_full).unwrap();
        assert_eq!(from_patch, from_full, "same entries, modes and links");
        for (path, node) in &from_full {
            if matches!(node, Node::File { .. }) {
                assert_eq!(
                    fs::read(join(&rebuilt, path)).unwrap(),
                    fs::read(join(&full, path)).unwrap(),
                    "{path}"
                );
            }
        }
        assert!(!rebuilt.join("removed.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_changed_install_or_another_build_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let (base, target) = linux_releases(root.path());
        let patch = root.path().join("update.caperpatch");
        generate("linux-x64", &base, 7, &target, 9, &patch).unwrap();
        let install = installed(root.path(), &base, "install");
        let stage = root.path().join("stage");
        for wrong in [expected(6, 9), expected(7, 10)] {
            assert!(apply(Layout::LinuxTarball, &patch, &wrong, &install, &stage).is_err());
        }
        assert!(
            apply(
                Layout::WindowsFolder,
                &patch,
                &expected(7, 9),
                &install,
                &stage
            )
            .is_err()
        );
        // A copied file and a patch base changed after install, one at a time.
        for changed in ["caper-updater", "caper-desktop"] {
            let original = fs::read(install.join(changed)).unwrap();
            let mut edited = original.clone();
            edited[0] ^= 1;
            fs::write(install.join(changed), edited).unwrap();
            let result = apply(
                Layout::LinuxTarball,
                &patch,
                &expected(7, 9),
                &install,
                &stage,
            );
            assert!(result.is_err(), "{changed}");
            fs::write(install.join(changed), original).unwrap();
        }
        // A file replaced by a link to the right bytes elsewhere is not a base.
        fs::rename(install.join("caper-updater"), root.path().join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(root.path().join("elsewhere"), install.join("caper-updater"))
            .unwrap();
        assert!(
            apply(
                Layout::LinuxTarball,
                &patch,
                &expected(7, 9),
                &install,
                &stage
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_paths_and_links_that_leave_or_alias_the_app() {
        let file = |path: &str| Entry::File {
            path: path.into(),
            mode: None,
            size: 1,
            sha256: "0".repeat(64),
            source: Source::Full {
                offset: 0,
                length: 1,
            },
        };
        let link = |path: &str, target: &str| Entry::Symlink {
            path: path.into(),
            target: target.into(),
        };
        let header = |entries| Header {
            schema: SCHEMA,
            platform: "linux-x64".into(),
            from_build: 1,
            to_build: 2,
            root: "Caper-linux-x64".into(),
            entries,
        };
        // Windows installs never hold links, so every link is refused there.
        assert_eq!(
            validate(
                &header(vec![file("a"), link("lib", "sub/../a")]),
                Layout::LinuxTarball
            )
            .is_ok(),
            cfg!(unix)
        );
        for entries in [
            vec![file("../escaped")],
            vec![file("a/../../escaped")],
            vec![file("/etc/passwd")],
            vec![file("C:evil")],
            vec![file("a\\..\\b")],
            vec![file("Caper"), file("caper")],
            vec![link("lib", "../../outside")],
            vec![link("lib", "/usr/lib")],
            vec![link("lib", "a"), file("lib/inside")],
        ] {
            assert!(
                validate(&header(entries.clone()), Layout::LinuxTarball).is_err(),
                "{entries:?}"
            );
        }
        let mut wrong_root = header(vec![file("a")]);
        wrong_root.root = "../Caper".into();
        assert!(validate(&wrong_root, Layout::LinuxTarball).is_err());
        wrong_root.root = "Caper-linux-x64".into();
        assert!(
            validate(&wrong_root, Layout::MacBundle).is_err(),
            "Mac roots end in .app"
        );
        wrong_root.root = String::new();
        assert!(validate(&wrong_root, Layout::WindowsFolder).is_ok());
    }

    #[test]
    fn windows_zips_patch_at_the_folder_root() {
        let root = tempfile::tempdir().unwrap();
        let zip = |name: &str, exe: &[u8]| {
            let path = root.path().join(name);
            test_zip(
                &path,
                &[
                    ("Caper.exe", exe),
                    ("caper-updater.exe", b"updater"),
                    ("licenses/LICENSE", b"license"),
                ],
            );
            path
        };
        let old = program(3);
        let mut new = old.clone();
        new[50_000] ^= 0xff;
        let (base, target) = (zip("base.zip", &old), zip("target.zip", &new));
        let patch = root.path().join("update.caperpatch");
        let summary = generate("windows-x64", &base, 1, &target, 2, &patch).unwrap();
        assert_eq!((summary.copied, summary.patched, summary.full), (2, 1, 0));
        let install =
            install::unpack(Layout::WindowsFolder, &base, &root.path().join("install")).unwrap();
        let stage = root.path().join("stage");
        let expected = Expected {
            platform: "windows-x64",
            from_build: 1,
            to_build: 2,
        };
        let rebuilt = apply(Layout::WindowsFolder, &patch, &expected, &install, &stage).unwrap();
        assert_eq!(rebuilt, stage);
        assert_eq!(fs::read(stage.join("Caper.exe")).unwrap(), new);
        assert_eq!(
            fs::read(stage.join("licenses/LICENSE")).unwrap(),
            b"license"
        );
    }
}
