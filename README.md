# Caper

Good company. Great conversations.

Caper is an open-source space for messages, calls, threads, and the useful
context around them. It is in its earliest stage today.

- Website: [caper.chat](https://caper.chat)
- Source: [github.com/joswayski/caper](https://github.com/joswayski/caper)

## Repository layout

- `apps/web` — TanStack Start website and health endpoint
- `apps/desktop` — React UI inside a minimal Tauri 2 shell
- `shared` — framework-neutral design tokens shared by both clients

A dedicated Rust API will be added as a separate service when the messaging,
presence, authentication, and WebRTC signaling contracts are defined. Realtime
voice and screen-sharing media will use WebRTC rather than pass through the API.

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
`/api/health`.
