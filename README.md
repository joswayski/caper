# Caper

Website: [caper.chat](https://caper.chat)

Caper is a place for your people. Chat with anyone, about anything. It is in
its earliest stage today (and an active work in progress)!

## Repository layout

- `apps/web` — TanStack Start website and health endpoint
- `apps/api` — Rust control service for the anonymous browser media lobby
- `apps/desktop` — React UI inside a minimal Tauri 2 shell
- `shared` — framework-neutral design tokens shared by both clients

## Browser MVP

`/live` (https://caper.chat/live after deployment) provides one public **General voice channel** using Cloudflare Realtime
SFU/TURN. Join or leave whenever; there are no outgoing calls or invitations.
No account or installation is required; joining requests microphone permission.
Faker generates an adjective/animal nickname on join, shared through the roster
and retained on automatic reconnect. Names are not verified or guaranteed unique.
Mute, deafen, microphone and
speaker selection are available. No text chat, database, camera or screen sharing.
Nothing is recorded by Caper. This is transport-encrypted, not E2EE; other
participants can record what they hear.

Voice is **disabled by default** unless server credentials are configured.
The temporary Cloudflare SFU app and TURN key have been provisioned; AWS
production activation is separate. See [media setup and runbook](docs/media.md) for the
remaining activation and acceptance checks. The desktop app is independent;
browser support does not establish Tauri media support. Messaging, accounts,
and persistent channels are not part of this MVP.

The web replicas use one dedicated Rust media-control instance on AWS; media
travels directly to Cloudflare, not through AWS. There is no PlanetScale
dependency for this ephemeral, account-free lobby.

Brand references and the owner's style guide are in [docs/brand](docs/brand).

## Development

Node.js 24, npm 11, and Rust 1.94 are used across local development and CI.

```bash
npm ci
npm run dev:web
```

Run the desktop shell with:

```bash
npm run dev
```

Desktop-affecting merges to `main` are batched into signed Preview releases for
macOS Apple silicon, Windows x64, and Linux x64. An installed Preview checks for
updates automatically. See [Desktop releases](docs/desktop-releases.md) for the
one-time signing setup and release behavior.

Run all checks with:

```bash
npm run check
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The production web container exposes port `3000` and reports readiness at
`/api/health`. The separate `apps/api/Dockerfile` API container (`caper-api`) exposes port
`3001` with `/health`. Run `cargo run -p caper-api` alongside the web app and set
`MEDIA_API_URL=http://127.0.0.1:3001` on the web process for local development.
