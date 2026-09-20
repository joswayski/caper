# Caper

Website: [caper.chat](https://caper.chat)

Caper is a place for your people. Chat with anyone, about anything. It is in
its earliest stage today (and an active work in progress)!

## Repository layout

- `apps/web` — TanStack Start website and health endpoint
- `apps/api` — Rust voice-control and account service
- `shared` — framework-neutral design tokens

Brand references and the owner's style guide are in [docs/brand](docs/brand).

## Development

Node.js 24, npm 11, and Rust 1.94 are used across local development and CI.
Local development runs the complete app through Docker Compose. It uses the regular
`staging` AWS profile and passes only its temporary credentials into the API
container. The Rust API reads `staging/apps/caper` from Secrets Manager and falls
back to the ignored repository `.env` and Compose defaults when that record is
unavailable. Secrets Manager wins when a setting exists in both places, including
the database URLs. No environment exports are needed.

```bash
npm ci
aws sso login --profile staging
npm run dev
```

Open `http://localhost:3000/login`. The helper never prints or writes the
short-lived credentials. Keep it running in the foreground so restarting the
stack refreshes expiring SSO credentials, and stop it with Ctrl-C. Populate `.env`
from `.env.example` only when using the fallback; at minimum it needs `AUTH_SECRET`.

Inspect which keys come from Secrets Manager, differ from `.env`, or are missing
without printing any values:

```bash
npm run secrets:check
# Production source plus External Secrets Operator readiness:
npm run secrets:check -- --environment production --profile production --cluster
```

Use `npm run dev:web` only when running the web process independently.

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
The `/live` demo offers one public General voice channel: guests get a random name,
while signed-in people use their display name. No account is required to join. Set `MEDIA_ENABLED=true` and the four
server-only Cloudflare variables in `.env.example` in the API environment to
enable calls. No database is required; leave database URLs unset for local voice
testing. The account API supports email sign-in codes when its database, SES, and
`AUTH_SECRET` settings are configured. Codes contain six uppercase letters or digits,
allow three attempts, and expire after 10 minutes. The website supports email-code sign-in and
required username/display-name onboarding. Guest names remain unverified and are
not reserved.
See the [media runbook](docs/media.md) for provider configuration, deployment,
privacy guidance, and the validation matrix.
