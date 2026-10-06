# Caper

A place for your people. Text and voice conversations at [caper.chat](https://caper.chat).
Early-stage and actively in development.

Accounts receive a saved, random Caper avatar from 100 designs and eight colorways.
Custom photo/GIF uploads and an avatar gallery are not implemented yet.

Signed-in accounts can start persistent, private one-to-one messages by username.
The Direct messages list is shared across spaces. Mobile push is deferred;
when needed, the server will integrate directly with APNs for iOS and FCM for Android.

Channel pins are shared with everyone who can read the channel. Joined members
can pin or unpin in one action; the channel header opens the complete pins list.
Native implementation and validation status are tracked in [the runbook](docs/media.md#message-pins).

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
- [apps/native](apps/native/README.md) — native development clients; not yet feature-parity releases
- [shared](shared) — shared design tokens

## Documentation

- [Configuration, deployment, and testing](docs/media.md)
- [Native builds and platform status](apps/native/README.md)
- [Brand and style guide](docs/brand)
- [Caper character catalog](assets/avatars/README.md)
