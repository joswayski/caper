# Caper

Website: [caper.chat](https://caper.chat)

Caper is a place for your people. Chat with anyone, about anything. It is in
its earliest stage today (and an active work in progress)!

## Repository layout

- `apps/web` — TanStack Start website and health endpoint
- `apps/api` — Rust voice-control and account service
- `apps/desktop` — React UI inside a minimal Tauri 2 shell
- `shared` — framework-neutral design tokens shared by both clients

Brand references and the owner's style guide are in [docs/brand](docs/brand).

## Development

Node.js 24, npm 11, and Rust 1.94 are used across local development and CI.

```bash
npm ci
npm run dev:web
```

For local account login, run PostgreSQL and the Rust API too. This example uses
the same local role for migrations and runtime; hosted environments retain
separate roles. Account startup applies migrations and grants automatically.

```bash
docker run --rm --name caper-postgres \
  -e POSTGRES_PASSWORD=caper -e POSTGRES_DB=caperchat \
  -p 54320:5432 postgres:17

export MIGRATION_DATABASE_URL='postgres://postgres:caper@127.0.0.1:54320/caperchat?sslmode=disable'
export DATABASE_URL="$MIGRATION_DATABASE_URL"
export AUTH_SECRET="$(openssl rand -base64 48)"
export AWS_PROFILE=staging-caper AWS_REGION=us-east-1
export SES_FROM_ADDRESS='Caper <noreply@staging.caper.chat>'
export SES_CONFIGURATION_SET=staging-caper-transactional
cargo run -p caper-api
```

In another terminal, run `npm run dev:web` and open `/login`. Authenticate the
`staging-caper` AWS profile first as documented in the infrastructure repository's
`docs/staging-sender-access.md`. Staging currently provides SES/IAM for local and
orb development; it is not a separately deployed Caper app or database.

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
`/health`. The separate `apps/api/Dockerfile` API container (`caper-api`) exposes port
`3001` with `/health`. In production, Traefik routes public `caper.chat/api/*`
requests directly to that Rust service. Run `cargo run -p caper-api` alongside
`npm run dev:web`; Vite forwards development `/api` requests to port `3001`.
The `/live` demo offers one public General voice channel: get a random guest name
and join without an account or profile. Set `MEDIA_ENABLED=true` and the four
server-only Cloudflare variables in `.env.example` in the API environment to
enable calls. No database is required; leave database URLs unset for local voice
testing. The account API supports email sign-in codes when its database, SES, and
`AUTH_SECRET` settings are configured. The website supports email-code sign-in and
required username/display-name onboarding; desktop login is not wired up yet.
Guest names remain unverified and are not reserved.
See the [media runbook](docs/media.md) for provider configuration, deployment,
privacy guidance, and the validation matrix.
