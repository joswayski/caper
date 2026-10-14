# Repository guidance

Adapted from the conventions in `joswayski/captures`.

## Product and repository map

- Caper is an early-stage space for conversations. Distinguish working features from roadmap ideas; do not describe mockups as shipped functionality.
- `apps/web`: TanStack Start browser app and same-origin development API adapter.
- `apps/api`: Rust media control, accounts, spaces and unified text/voice channels. Cloudflare Realtime SFU/TURN carries audio; AWS carries control traffic. The public General demo is retired; the homepage shows a labelled local simulation linking to email sign-in. Account spaces require membership, and private channels have explicit member grants. Owners manage spaces/channels, add existing accounts by username, and create Discord-style invite links (`/invite/<code>`, expiry/max uses/landing channel; owner-removed or blocked accounts cannot rejoin by link). Text uses Postgres/outbox and the independently deployed `--gateway` WebSocket role with Valkey fanout. No custom roles, camera, or screen sharing.
- `shared/design.css`: shared visual tokens. `docs/brand` contains the owner's reference images and style guidance.
- `docs/media.md`: configuration, deployment, privacy, validation matrix, and call runbook.

## Working conventions

- Keep changes focused; reuse existing patterns before adding dependencies or abstractions.
- Implement every product behavior change across all applicable clients: web (desktop and mobile layouts), Android, Apple (iOS/macOS), and Rust desktop. Never leave a feature web-only or native-only unless the owner explicitly requests a platform-specific change. Missing toolchains, devices or CI are validation gaps, not reasons to omit an implementation: make the best possible cross-platform change and report what could not be verified. Platform-native presentation may differ, but capabilities must stay equivalent.
- Other agents may work concurrently. Use an isolated worktree for new concurrent work; never stash, overwrite, or publish another agent's changes.
- Treat native clients as independent implementations. Browser WebRTC success does not prove native capture or playback support.
- Never expose provider secrets or log SDP, credentials, or raw media. No unrestricted Cloudflare API proxy.
- Keep data that is or might be useful (users, spaces, channels, messages, memberships, channel joins, reactions, invitations, DMs, sign-in sessions): removals set `deleted_at` (or a status/`revoked_at`) and reads filter on it, instead of `DELETE` or `ON DELETE CASCADE`. Short-lived operational records (expired sign-in codes, rate-limit logs, push delivery state) may be pruned.
- Keep one desired API replica until every API pod uses the same `VALKEY_URL` and compatible state schema. Never mix in-memory and shared-mode pods. Follow the staged cutover in `docs/media.md`; shared-mode shutdown must not close healthy Cloudflare tracks. Web replicas are independent.

## Visual design

- Use shared tokens: blackout #0C0D0F, surface #151719, border #34383B, text #F3F4F5, eggshell action #F1EDE4, terracotta #B64D32, caper green #637A43. Satoshi typography; restrained borders and 8px control corners.
- Primary actions are eggshell fills with blackout labels; disabled ones step back to neutral chrome. Terracotta is for selection, focus, mentions and unread state. Green primarily belongs to the character, with restrained presence/status use. Keep general chrome neutral.
- Motion: things arrive rather than teleport. Use the shared `--motion-*`/`--ease-out` tokens: short, decelerating, opacity/translate-only entrances; closing stays instant; animate only messages that arrive while the reader watches the live end; honor reduced-motion settings on every client.
- Build hierarchy with typography, spacing, and clear layouts rather than large color fields. Fictional participants/messages belong only in the explicitly labelled homepage simulation; never use them in real account spaces or add working-looking controls for unimplemented features.
- Inspect rendered desktop/mobile and affected non-default browser states before claiming visual completion. Use real browser interactions or explicitly labeled test mocks.

## Documentation and validation

- Leave README accurate and concise. Put operational detail in `docs/media.md`; maintain an honest platform/test matrix.
- Run `npm run check` (Oxlint, Oxfmt, Vite build and native TypeScript 7 typecheck) and `npm test` (Vitest web and shared native-support tests); use `npm run fmt` to format first-party JS/TS/CSS. For Rust, run `cargo fmt --all -- --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- For Python tooling, use pinned uv and `uv.lock`: `uv run --locked ruff check .`, `uv run --locked ruff format --check .`, `uv run --locked python -m unittest discover -s tests -p 'test_*.py' -v`, and `uv run --locked python shared/fonts/test_native_fonts.py -v`. Use `uv run --locked ruff format .` to format Python; keep release/authoring dependencies in their separate groups, not the global interpreter.
- Build changed Docker images when a daemon is available; otherwise validate build stages directly and report the limitation.
- Never equate mocks with live SFU validation. Record multi-network/TURN, sustained voice, and physical device checks separately.

## Shipping

Speed beats ceremony: there are no real users yet, so ship, watch, and fix forward.

- Commit and push straight to `main`; no pull requests. Run the local checks for what you changed first (above), then `git pull --rebase origin main` and push. Never force-push `main` or rewrite other agents' commits.
- Never put literal CI-skip directives in commit messages, even to explain them: path-filtered image builds would silently not run.
- Pushes to `main` run `CI` (a few minutes) and, when their paths changed, the `AWS image` (web) and `API image` (API and gateway) builds. `Native development builds` runs nightly and on demand; don't start it for routine changes.
- After pushing, deploy what changed yourself, servers before clients, waiting for each step to succeed before the next. `SHA` is your pushed commit; find its image runs with `gh run list --repo joswayski/caper --commit "$SHA"` and wait with `gh run watch <id> --exit-status` (infrastructure runs: `gh run list --repo joswayski/infrastructure --workflow <file> --limit 1`). Report what you deployed.
  1. Infrastructure or secrets the change needs, in `joswayski/infrastructure`, following that repository's guidance.
  2. If `API image` ran for `SHA`: `gh workflow run deploy-caper-api.yml --repo joswayski/infrastructure --ref main -f git_sha="$SHA"`, wait for it, then the same with `deploy-caper-gateway.yml`. The API applies database migrations on startup.
  3. If `AWS image` ran for `SHA`: `gh workflow run deploy-caper-web.yml --repo joswayski/infrastructure --ref main -f git_sha="$SHA"`.
  4. If native client code changed: `gh workflow run release.yml --repo joswayski/caper --ref main -f git_sha="$SHA"`. It takes 20-40 minutes; you need not wait. The Intel Mac build finishes on its own and never blocks the release.
- If `CI`, an image build, a deploy or a release fails, fix it forward with another commit. Roll back a broken server with the deploy workflow's `rollback_from_sha` input only when a fix will take longer than a few minutes.
- Still ask the owner first for anything irreversible: destructive or data-dropping migrations, deleting infrastructure or data, rotating secrets.
