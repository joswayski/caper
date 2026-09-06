# Desktop releases

Caper uses Tauri's signed updater and a rolling GitHub Preview channel. Release
runs are batched: the active release finishes, GitHub retains only the newest
pending run, and that run snapshots the latest `origin/main` when it starts.
This prevents a merge queue from packaging every intermediate commit while
still shipping all accumulated desktop changes.

The workflow runs for desktop-affecting changes and can also be started
manually. It:

1. compares current `main` with the last dated desktop release;
2. creates a CalVer draft for that complete batch;
3. packages macOS Apple silicon, Windows x64, and Linux x64 in parallel;
4. validates all installers, updater archives, signatures, and `latest.json`;
5. publishes the immutable dated Preview;
6. synchronizes its downloads to the permanent `preview` release.

Released builds check for updates 15 seconds after startup and every six hours.
When a newer signed build exists, Caper downloads it, installs it, and restarts.
On Linux, in-place updating is supported by the AppImage; `.deb` users install
new packages manually.

## One-time GitHub setup

Create a protected GitHub environment named `release`, restricted to `main`.
Add one environment variable:

| Variable | Purpose |
| --- | --- |
| `TAURI_UPDATER_PUBLIC_KEY` | Public half of Caper's dedicated updater key |

Add these environment secrets:

| Secret | Purpose |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Signs update archives on every platform |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Updater private-key password |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | Password used when exporting the `.p12` |
| `KEYCHAIN_PASSWORD` | Random password for CI's temporary keychain |
| `APPLE_API_ISSUER` | App Store Connect API issuer ID |
| `APPLE_API_KEY` | App Store Connect API key ID |
| `APPLE_API_PRIVATE_KEY` | Complete contents of the API key `.p8` file |

Generate a dedicated updater key locally from the repository root:

```bash
npm exec --workspace @caper/desktop tauri signer generate -- \
  --write-keys ~/.tauri/caper.key
```

Store the generated private key and password in a password manager and an
encrypted offline backup before adding them to GitHub. Losing this key means
already-installed copies cannot authenticate a replacement key and therefore
cannot receive another automatic update. Do not reuse Captures' updater key.

The Apple credentials may use the same Apple Developer team and Developer ID
Application identity as another app, but must be added separately because
GitHub does not expose or copy secret values between repositories. The macOS
job fails closed if any credential is absent or if signing, notarization,
stapling, or Gatekeeper validation fails.

## Platform trust

Updater signatures and operating-system publisher signatures solve different
problems. The Tauri key authenticates updates. Apple signing and notarization
identify the macOS publisher and satisfy Gatekeeper.

Windows Authenticode is not configured yet, matching Captures' current release
implementation. Windows updates are authenticated by Tauri, but the installer
will display an unknown publisher. Before treating Windows distribution as
production-ready, add a public-trust signing service such as Microsoft Artifact
Signing and timestamp both `caper.exe` and the NSIS installer.

The first Preview must be downloaded and installed manually. Every later
Preview can update that installation automatically.
