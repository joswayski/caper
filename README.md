# Caper

Website: [caper.chat](https://caper.chat)

Caper is a place for your people. Chat with anyone, about anything. It is in
its earliest stage today (and an active work in progress)!

## Repository layout

- `apps/web` — TanStack Start website and health endpoint
- `apps/api` — Rust account and authenticated voice control service
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
`3001` with `/health`. Run `cargo run -p caper-api` alongside the web app and set
`MEDIA_API_URL=http://127.0.0.1:3001` on the web process for local development.
The AuthKit trial requires WorkOS configuration and Postgres; see `.env.example`.
Web pages and voice require login and a completed Caper profile. The prepared
deployment uses `caper.chat` for the website and `api.caper.chat` for the same Rust
API shared by all clients. API requests use `Authorization: Bearer <access-token>`;
desktop/mobile clients call it directly, not a separate native gateway. Browser
cookies stay on the website, whose server calls Rust internally. Public hostname
activation and desktop/mobile login UI are not deployed by this PR.
See the [media runbook](docs/media.md) for provider configuration, deployment,
privacy guidance, and the validation matrix.
