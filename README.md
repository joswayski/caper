# Caper

A place for your people. Text and voice conversations at [caper.chat](https://caper.chat).
Early-stage and actively in development.

Caper runs on the web, Android, iPhone/Mac and Windows/Linux desktop. Phones install
public betas from the homepage (TestFlight and a Google Play closed test).

## What works today

- Spaces with text and voice channels, private channels and invite links ([invites](docs/media.md#invite-links))
- Direct messages and notifications ([notifications](docs/notifications.md))
- Threads, pins, forwarding, editing and deleting messages
- File and media attachments, with an editor to crop, draw and add text ([uploads](docs/media.md#uploads-and-attachments))
- Profile pictures and space icons ([pictures](docs/media.md#custom-profile-pictures-and-space-icons-october-14-2026))
- `@feedback` in any conversation to reach the Caper team ([feedback](docs/media.md#feedback-in-place))
- Desktop apps that update themselves ([native apps](apps/native/README.md))

Per-feature details, rollout state and validation gaps live in [docs/media.md](docs/media.md).

## Development

You'll need Node.js 24, npm 11+, Docker Compose, Rust 1.94, and AWS CLI access to
the `staging` profile.

```bash
npm ci
aws sso login --profile staging
npm run dev
```

Open `http://localhost:3000/login`. Compose runs the web app, API, gateway and Valkey;
config comes from AWS Secrets Manager (`staging/apps/caper`), with
[.env.example](.env.example) listing the settings.

Checks:

```bash
npm run check   # lint, format check, build, typecheck
npm test        # unit tests
npm run fmt     # format JS/TS/CSS

cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Python tooling uses [uv](https://docs.astral.sh/uv/) and Ruff:

```bash
uv run --locked ruff check .
uv run --locked ruff format --check .
uv run --locked python -m unittest discover -s tests -p 'test_*.py' -v
```

## Repository

- [apps/web](apps/web) — TanStack Start web app
- [apps/api](apps/api) — Rust API and WebSocket gateway
- [apps/native](apps/native/README.md) — Android, Apple and desktop clients
- [shared](shared) — shared design tokens

## Documentation

- [Configuration, deployment, and testing](docs/media.md)
- [Notifications](docs/notifications.md)
- [Native builds and installs](apps/native/README.md)
- [Brand and style guide](docs/brand)
- [Caper character catalog](assets/avatars/README.md)
