# Caper

A place for your people. Text and voice conversations at [caper.chat](https://caper.chat).
Early-stage and actively in development.

Accounts receive a saved, random Caper avatar from 100 designs and eight colorways.
Custom photo/GIF avatars and an avatar gallery are not implemented yet.

Signed-in members can attach up to 10 files per message on web, Android,
iPhone/Mac and Windows/Linux desktop: images, video, audio and other files.
Clients upload originals; a server-side media worker compresses every file the
same way (lossless WebP for screenshots, AVIF for photos, H.264 for video and
GIFs, FLAC for WAV, gzip for documents) within a per-person storage allowance
(10 GB by default, after compression). Desktop opens video and audio in the
system player. See [Uploads and attachments](docs/media.md#uploads-and-attachments).

Signed-in accounts can start persistent, private one-to-one messages by username.
The Direct messages list is shared across spaces. Mobile push is deferred;
when needed, the server will integrate directly with APNs for iOS and FCM for Android.

Channel pins are shared with everyone who can read the channel. Joined members
can pin or unpin in one action; the channel header opens the complete pins list.
Native implementation and validation status are tracked in [the runbook](docs/media.md#message-pins).

Channel messages support threads: replies stay with their parent, with reply counts
and participant avatars in the channel. Desktop uses a right panel and mobile a
full-screen view. **Also send to channel** shows the same reply in both places,
sharing its reactions. Native release and validation gaps are tracked in
[the runbook](docs/media.md#message-threads).

**Forward message** shares a live, read-only conversation across spaces or into
an existing DM, including edits, reactions and future replies. Destination readers do
not need source membership; their replies stay in a separate destination thread.
Native validation and rollout are tracked in
[the runbook](docs/media.md#live-message-forwarding). This change is not deployed
by merging alone.

Authors can edit messages on web, Android, Apple and Rust desktop, including
thread roots/replies and DMs. The edited indicator opens retained history with
the previous/current versions side by side and older changes selectable below;
broadcast replies update in both places. See platform validation gaps and
[editing and rollout details](docs/media.md#message-editing).

Mac release downloads use a signed, notarized disk image: open it, drag Caper onto
Applications, eject the image, then launch Caper from Applications. `~/Applications`
also works without administrator access. Release copies outside these folders or
in a non-writable folder show installation instructions and quit instead of leaving
you using a copy that cannot reliably update. Windows Setup installs per-user and
creates Start/Desktop shortcuts. Linux `.deb` installs use the package manager;
only writable, self-contained Linux archive installs support in-app installation
of updates. See [native installation details](apps/native/README.md#signed-builds-for-testers).

Packaged Mac, Windows, and Linux apps check for updates 20 seconds after launch
and every minute, using signed metadata cached on Caper's server for 60 seconds.
Check manually with **Caper → Check for Updates…** on Mac or
**User Settings → Settings… → Updates → Check for updates** on Windows/Linux.
Installing requires confirmation and a restart; protected installs offer a download
instead. Existing apps keep their previous schedule until updated once.
Android APK updates remain manual; iPhone builds use TestFlight.

## Development

You'll need Node.js 24, npm 11+, Docker Compose, and AWS CLI access to the `staging` profile.
Rust development uses Rust 1.94.

```bash
npm ci
aws sso login --profile staging
npm run dev
```

Open `http://localhost:3000/login`. Docker Compose runs the web app, API, gateway,
and local Valkey. Configuration comes from AWS Secrets Manager (`staging/apps/caper`),
with `.env` and Compose defaults as fallbacks. See [.env.example](.env.example)
for available settings. Stop the stack with Ctrl-C.

## Repository

- [apps/web](apps/web) — TanStack Start web app
- [apps/api](apps/api) — Rust API and WebSocket gateway
- [apps/media-worker](apps/media-worker) — Rust Lambda that compresses uploaded attachments
- [apps/native](apps/native/README.md) — native development clients; not yet feature-parity releases
- [shared](shared) — shared design tokens

## Documentation

- [Configuration, deployment, and testing](docs/media.md)
- [Native builds and platform status](apps/native/README.md)
- [Brand and style guide](docs/brand)
- [Caper character catalog](assets/avatars/README.md)
