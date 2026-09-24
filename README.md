# Caper

Website: [caper.chat](https://caper.chat)

Caper is a place for your people. Chat with anyone, about anything. It is in
its earliest stage today (and an active work in progress)!

## Repository layout

- `apps/web` - TanStack Start website and health endpoint
- `apps/api` - Rust voice-control, accounts, and text commands; separate WebSocket gateway role
- `apps/native` - browser-free desktop/mobile development clients and packaging
- `shared` - framework-neutral design tokens

Brand references and the owner's style guide are in [docs/brand](docs/brand).

## Native apps

[Native development builds](apps/native/README.md) target macOS, Windows, Linux,
iPhone, and Android without Tauri or Electron. GitHub Actions produces platform
development artifacts with checksums and source revision metadata. These are not
feature-parity releases: native voice, locked-screen calling, physical-device
acceptance, and trusted distribution remain release gates. The unsigned iOS
artifact is for Simulator, not installation on an iPhone.

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

Compose also starts disposable local Valkey for shared call state; no AWS cache
is needed for staging. Its port is not published to the host. API restarts keep
the state while Valkey stays running; restarting Valkey clears active calls.
Leave `VALKEY_URL` unset in `.env` and `staging/apps/caper` to use this default.
See [local shared-state testing](docs/media.md#local-compose-shared-state-testing).

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
`3001` with `/health` for liveness and `/readyz` for dependency-aware readiness.
Set server-only `VALKEY_URL` to share live calls across API replicas; leave it empty
only for single-process development. The [shared-state rollout](docs/media.md#shared-call-state-and-rolling-deployments)
requires a one-time empty-channel cutover before enabling multiple replicas.
In production, Traefik routes public `caper.chat/api/*`
requests directly to that Rust service. Run `cargo run -p caper-api` alongside
`npm run dev:web`; Vite forwards development `/api` requests to port `3001`.
The exception is `/api/chat/events`, which goes to the independent application
WebSocket gateway on `3002` (`cargo run -p caper-api -- --gateway`). One connection
carries chat delivery, typing, member status, and voice controls/rosters. Durable
message writes and history remain HTTP. Member status becomes idle after ten
minutes without input (`PRESENCE_IDLE_TIMEOUT_SECONDS=600`); temporary session
records expire in Valkey rather than accumulating presence history. See the
[gateway runbook](docs/media.md#application-gateway-and-account-presence) for
recovery guarantees, configuration, and deployment order. With `CHAT_ENABLED=true`,
Postgres, and Valkey, the homepage offers messaging in the public demo's `general`
channel. Guests can read/send without joining voice or signing in. Messages
are persisted; the [text runbook](docs/media.md#public-text-demo) covers limits,
replay, and staged production activation. Compose includes the gateway.
Signed-in users can create spaces with unified text/voice channels, add existing
accounts by username, and restrict private channels to selected space members.
Owners can rename/delete spaces and channels. Default limits are 20 owned spaces,
100 total space memberships, and 100 channels per space. Invite links, custom
roles, and screen sharing are not implemented. See
[space access and rollout](docs/media.md#spaces-and-channel-access).
Voice in the same public `general` channel gives guests a random name,
while signed-in people use their display name. No account is required to join. Set `MEDIA_ENABLED=true` and the four
server-only Cloudflare variables in `.env.example` in both API and gateway environments
to enable calls. The current browser requires the gateway, Postgres, and Valkey
even for guest voice; legacy HTTP-only clients can still use the single-process API.
The account API supports email sign-in codes when its database, SES, and
`AUTH_SECRET` settings are configured. Codes contain six uppercase letters or digits,
allow three attempts, and expire after 10 minutes. The website supports email-code sign-in and
required username/display-name onboarding. Guest names remain unverified and are
not reserved.
See the [media runbook](docs/media.md) for provider configuration, deployment,
privacy guidance, and the validation matrix.
