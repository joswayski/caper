# Caper

Website: [caper.chat](https://caper.chat)

Caper is a space for messages, calls, threads, and the useful context around
them. It is in its earliest stage today (and an active work in progress)!

## Repository layout

- `apps/web` — TanStack Start website and health endpoint
- `apps/desktop` — React UI inside a minimal Tauri 2 shell
- `shared` — framework-neutral design tokens shared by both clients

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
`/api/health`.
