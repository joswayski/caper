# Caper

A place for your people. Text and voice conversations at [caper.chat](https://caper.chat).
Early-stage and actively in development.

Accounts receive a saved, random Caper avatar from 100 designs and eight colorways.
Custom photo/GIF avatars and an avatar gallery are not implemented yet.

Signed-in members can attach up to 10 files per message on the web: images
(compressed in the browser), video, audio and other files, within a per-person
storage allowance (1 GB by default). Android, Apple and Rust desktop show the
message text but not its files yet. See [Uploads and attachments](docs/media.md#uploads-and-attachments).

Signed-in accounts can start persistent, private one-to-one messages by username.
The Direct messages list is shared across spaces. Optional iOS/Android push needs
provider configuration and signed-device validation; it is not enabled by default.

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
