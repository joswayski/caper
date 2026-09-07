# Caper

Website: [caper.chat](https://caper.chat)

Caper is a place for your people. Chat with anyone, about anything. It is in
its earliest stage today (and an active work in progress)!

## Repository layout

- `apps/web` — TanStack Start website and health endpoint
- `apps/api` — Rust account foundation and retained voice-control engine (access unavailable)
- `apps/desktop` — React UI inside a minimal Tauri 2 shell
- `shared` — framework-neutral design tokens shared by both clients

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
`3001` with `/health`. Run `cargo run -p caper-api` to develop the API separately;
the website no longer forwards account or media requests to it.
The website remains live, but accounts, profiles, and voice-room access are
currently unavailable. `/login`, `/live`, and `/profile` honestly show or route to
that unavailable state. The media engine remains in the codebase for future use,
but production authentication fails closed and does not enable anonymous access.
No replacement account provider has been selected or implemented.
See the [media runbook](docs/media.md) for provider configuration, deployment,
privacy guidance, and the validation matrix.
