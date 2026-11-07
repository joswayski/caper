//! The signed update manifest (`latest.json`) published with every release.
//!
//! The release workflow writes `latest.json` next to the downloads and signs its
//! exact bytes with the updater's Ed25519 key into `latest.json.sig` (base64).
//! Clients trust nothing in the manifest until that signature verifies against
//! the public key compiled into the updater, so a tampered release, redirect or
//! mirror cannot deliver a different archive: each archive's SHA-256 and size
//! come from the signed manifest.

use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Largest manifest or signature the updater will read.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    /// Monotonic release number; a client updates only to a larger one.
    pub build: u64,
    /// Human-readable version shown in the update prompt, e.g. `0.1.42`.
    pub version: String,
    pub commit: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub changelog: Vec<ChangelogEntry>,
    /// Oldest installed build for which the retained history is complete.
    #[serde(default)]
    pub changelog_from_build: Option<u64>,
    pub platforms: BTreeMap<String, Artifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangelogEntry {
    pub build: u64,
    pub version: String,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// Patches from earlier builds to this one (see `delta`). Updaters that
    /// predate them ignore the field and download the full archive above.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patches: Vec<Patch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    pub from_build: u64,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

impl Artifact {
    /// The patch that turns `current_build` into this artifact's build.
    pub fn patch_from(&self, current_build: u64) -> Option<&Patch> {
        self.patches
            .iter()
            .find(|patch| patch.from_build == current_build)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ManifestError {
    TooLarge,
    BadPublicKey,
    BadSignature,
    SignatureMismatch,
    Malformed(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "update manifest is too large"),
            Self::BadPublicKey => write!(f, "update public key is invalid"),
            Self::BadSignature => write!(f, "update signature is not valid base64 Ed25519"),
            Self::SignatureMismatch => write!(f, "update manifest signature does not verify"),
            Self::Malformed(error) => write!(f, "update manifest is malformed: {error}"),
        }
    }
}

impl std::error::Error for ManifestError {}

pub fn decode_public_key(encoded: &str) -> Result<VerifyingKey, ManifestError> {
    let bytes: [u8; 32] = STANDARD
        .decode(encoded.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(ManifestError::BadPublicKey)?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| ManifestError::BadPublicKey)
}

pub fn encode_public_key(key: &VerifyingKey) -> String {
    STANDARD.encode(key.as_bytes())
}

/// Signs manifest bytes, returning the base64 signature stored in `latest.json.sig`.
pub fn sign(manifest: &[u8], key: &SigningKey) -> String {
    STANDARD.encode(key.sign(manifest).to_bytes())
}

/// Verifies the signature over the exact bytes, then parses and validates them.
pub fn verify(
    manifest: &[u8],
    signature: &str,
    key: &VerifyingKey,
) -> Result<Manifest, ManifestError> {
    if manifest.len() > MAX_MANIFEST_BYTES || signature.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge);
    }
    let signature: [u8; 64] = STANDARD
        .decode(signature.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(ManifestError::BadSignature)?;
    key.verify(manifest, &Signature::from_bytes(&signature))
        .map_err(|_| ManifestError::SignatureMismatch)?;
    let mut manifest: Manifest = serde_json::from_slice(manifest)
        .map_err(|error| ManifestError::Malformed(error.to_string()))?;
    manifest.validate()?;
    // A patch is only an optimization: drop one that is unusable instead of
    // refusing the release, which still updates through the full archive.
    let build = manifest.build;
    for artifact in manifest.platforms.values_mut() {
        let mut bases = std::collections::BTreeSet::new();
        artifact.patches.retain(|patch| {
            patch.url.starts_with("https://")
                && is_sha256(&patch.sha256)
                && patch.size > 0
                && patch.from_build < build
                && bases.insert(patch.from_build)
        });
    }
    Ok(manifest)
}

impl Manifest {
    fn validate(&self) -> Result<(), ManifestError> {
        let malformed = |message: &str| Err(ManifestError::Malformed(message.to_owned()));
        if self.schema != SCHEMA {
            return malformed("unsupported schema");
        }
        if self.build == 0 || self.version.trim().is_empty() {
            return malformed("missing build or version");
        }
        for (platform, artifact) in &self.platforms {
            if !is_platform(platform) {
                return malformed("unknown platform");
            }
            if !artifact.url.starts_with("https://") {
                return malformed("artifact URLs must use HTTPS");
            }
            if !is_sha256(&artifact.sha256) {
                return malformed("artifact SHA-256 must be 64 lowercase hex characters");
            }
            if artifact.size == 0 {
                return malformed("artifact size must be positive");
            }
        }
        Ok(())
    }

    /// The artifact for `platform` when it is strictly newer than `current_build`.
    pub fn update_for(&self, platform: &str, current_build: u64) -> Option<&Artifact> {
        (self.build > current_build)
            .then(|| self.platforms.get(platform))
            .flatten()
    }

    /// Show only changes included in the jump from the installed build to latest.
    pub fn changes_since(&self, current_build: u64) -> Vec<&ChangelogEntry> {
        let mut entries = self
            .changelog
            .iter()
            .filter(|entry| current_build < entry.build && entry.build <= self.build)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.build));
        entries
    }

    pub fn history_complete(&self, current_build: u64) -> bool {
        self.changelog_from_build
            .is_some_and(|oldest| current_build >= oldest)
            && self.changelog.iter().any(|entry| entry.build == self.build)
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub const PLATFORMS: [&str; 4] = ["macos-arm64", "macos-x64", "windows-x64", "linux-x64"];

fn is_platform(name: &str) -> bool {
    PLATFORMS.contains(&name)
}

/// The platform this updater binary was built for.
pub const fn current_platform() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("macos-x64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn manifest_json(build: u64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": 1,
            "build": build,
            "version": format!("0.1.{build}"),
            "commit": "abc",
            "notes": "Screen sharing",
            "platforms": {
                "linux-x64": {"url": "https://example.com/Caper-Linux-x64.tar.gz", "sha256": SHA, "size": 10},
                "windows-x64": {"url": "https://example.com/Caper-Windows-x64.zip", "sha256": SHA, "size": 10}
            }
        }))
        .unwrap()
    }

    #[test]
    fn verifies_signed_manifest_and_offers_only_newer_builds() {
        let bytes = manifest_json(42);
        let signature = sign(&bytes, &key());
        let public = decode_public_key(&encode_public_key(&key().verifying_key())).unwrap();
        let manifest = verify(&bytes, &signature, &public).unwrap();
        assert_eq!(manifest.version, "0.1.42");
        assert!(manifest.update_for("linux-x64", 41).is_some());
        assert!(manifest.update_for("linux-x64", 42).is_none());
        assert!(manifest.update_for("linux-x64", 43).is_none());
        assert!(manifest.update_for("macos-arm64", 1).is_none());
    }

    #[test]
    fn changelog_filters_both_boundaries_and_sorts_by_build_not_version_text() {
        let mut value: serde_json::Value = serde_json::from_slice(&manifest_json(42)).unwrap();
        value["changelog_from_build"] = 8.into();
        value["changelog"] = serde_json::json!([
            {"build": 9, "version": "0.1.9", "notes": "Older change"},
            {"build": 43, "version": "0.1.43", "notes": "Not shipped yet"},
            {"build": 10, "version": "0.1.10", "notes": "Installed change"},
            {"build": 42, "version": "0.1.42", "notes": "Newest change"},
            {"build": 11, "version": "0.1.11", "notes": "Skipped change"}
        ]);
        let bytes = serde_json::to_vec(&value).unwrap();
        let manifest = verify(&bytes, &sign(&bytes, &key()), &key().verifying_key()).unwrap();
        assert_eq!(
            manifest
                .changes_since(10)
                .iter()
                .map(|entry| entry.build)
                .collect::<Vec<_>>(),
            vec![42, 11]
        );
        assert!(manifest.history_complete(8));
        assert!(!manifest.history_complete(7));
        assert!(manifest.changes_since(42).is_empty());
        let legacy: Manifest = serde_json::from_slice(&manifest_json(42)).unwrap();
        assert!(legacy.changes_since(10).is_empty());
        assert!(!legacy.history_complete(10));
        assert_eq!(legacy.notes, "Screen sharing");
    }

    #[test]
    fn rejects_tampered_bytes_wrong_keys_and_bad_signatures() {
        let bytes = manifest_json(42);
        let signature = sign(&bytes, &key());
        let public = key().verifying_key();
        let mut tampered = bytes.clone();
        let position = tampered.iter().position(|&b| b == b'4').unwrap();
        tampered[position] = b'9';
        assert_eq!(
            verify(&tampered, &signature, &public),
            Err(ManifestError::SignatureMismatch)
        );
        let other = SigningKey::from_bytes(&[9; 32]).verifying_key();
        assert_eq!(
            verify(&bytes, &signature, &other),
            Err(ManifestError::SignatureMismatch)
        );
        assert_eq!(
            verify(&bytes, "not base64!", &public),
            Err(ManifestError::BadSignature)
        );
        assert_eq!(
            verify(&vec![b' '; MAX_MANIFEST_BYTES + 1], &signature, &public),
            Err(ManifestError::TooLarge)
        );
        assert_eq!(decode_public_key("AAAA"), Err(ManifestError::BadPublicKey));
    }

    #[test]
    fn patches_are_offered_per_installed_build_and_unusable_ones_are_dropped() {
        let patch = |from: u64, url: &str, size: u64| serde_json::json!({"from_build": from, "url": url, "sha256": SHA, "size": size});
        let mut value: serde_json::Value = serde_json::from_slice(&manifest_json(42)).unwrap();
        value["platforms"]["linux-x64"]["patches"] = serde_json::json!([
            patch(41, "https://example.com/41.caperpatch", 5),
            patch(39, "https://example.com/39.caperpatch", 7),
            patch(39, "https://example.com/again.caperpatch", 7),
            patch(38, "http://example.com/38.caperpatch", 5),
            patch(37, "https://example.com/37.caperpatch", 0),
            patch(42, "https://example.com/42.caperpatch", 5),
            {"from_build": 36, "url": "https://example.com/36", "sha256": "ABC", "size": 5},
        ]);
        let bytes = serde_json::to_vec(&value).unwrap();
        let manifest = verify(&bytes, &sign(&bytes, &key()), &key().verifying_key()).unwrap();
        let linux = manifest.update_for("linux-x64", 39).unwrap();
        assert_eq!(
            linux
                .patches
                .iter()
                .map(|patch| patch.from_build)
                .collect::<Vec<_>>(),
            vec![41, 39],
            "only the first patch per build, over HTTPS, with a size and hash"
        );
        assert_eq!(linux.patch_from(39).unwrap().size, 7);
        for build in [40, 42, 36] {
            assert!(linux.patch_from(build).is_none());
        }
        assert!(manifest.platforms["windows-x64"].patches.is_empty());

        // Updaters released before patches read the same signed bytes and
        // still get the full archive.
        #[derive(Deserialize)]
        struct OldArtifact {
            url: String,
        }
        #[derive(Deserialize)]
        struct OldManifest {
            platforms: BTreeMap<String, OldArtifact>,
        }
        let old: OldManifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            old.platforms["linux-x64"].url,
            "https://example.com/Caper-Linux-x64.tar.gz"
        );
    }

    #[test]
    fn rejects_signed_manifests_with_unsafe_artifacts() {
        for artifact in [
            serde_json::json!({"url": "http://example.com/a.zip", "sha256": SHA, "size": 1}),
            serde_json::json!({"url": "https://example.com/a.zip", "sha256": "ABC", "size": 1}),
            serde_json::json!({"url": "https://example.com/a.zip", "sha256": SHA, "size": 0}),
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "schema": 1, "build": 2, "version": "0.1.2", "commit": "c",
                "platforms": {"windows-x64": artifact}
            }))
            .unwrap();
            let signature = sign(&bytes, &key());
            assert!(matches!(
                verify(&bytes, &signature, &key().verifying_key()),
                Err(ManifestError::Malformed(_))
            ));
        }
    }
}
