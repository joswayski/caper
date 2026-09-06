# Caper

Good company. Great conversations.

Caper is an open-source space for messages, calls, threads, and the useful
context around them. It is in its earliest stage today.

- Website: [caper.chat](https://caper.chat)
- Source: [github.com/joswayski/caper](https://github.com/joswayski/caper)

## Repository layout

- `apps/web` — TanStack Start website and health endpoint
- `apps/api` — Rust control service for the anonymous browser media lobby
- `apps/desktop` — React UI inside a minimal Tauri 2 shell
- `shared` — framework-neutral design tokens shared by both clients

## Browser MVP

`/call` provides one public lobby for microphone voice, cameras, and screen
sharing using Cloudflare Realtime SFU/TURN. No account or installation is
required; joining requests microphone permission. Names are optional and are
not verified identities. Cameras and shared screens require an explicit Watch
action. Nothing is recorded by Caper. This is transport-encrypted, not E2EE;
other participants can capture what they receive.

Calls are **disabled by default** pending provider configuration and live
multi-network testing. See [media setup and runbook](docs/media.md) for the
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

Run all checks with:

```bash
npm run check
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The production web container exposes port `3000` and reports readiness at
`/api/health`. The separate `apps/api/Dockerfile` media container exposes port
`3001` with `/health`. Run `cargo run -p caper-api` alongside the web app and set
`MEDIA_API_URL=http://127.0.0.1:3001` on the web process for local development.
