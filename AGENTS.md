# Repository guidance

Adapted from the conventions in `joswayski/captures`.

## Product and repository map

- Caper is an early-stage space for conversations. Distinguish working features from roadmap ideas; do not describe mockups as shipped functionality.
- `apps/web`: TanStack Start browser app and same-origin development API adapter.
- `apps/api`: Rust media control, accounts, spaces and unified text/voice channels. Cloudflare Realtime SFU/TURN carries audio; AWS carries control traffic. The public General demo is retired; the homepage shows a labelled local simulation linking to email sign-in. Account spaces require membership, and private channels have explicit member grants. Owners manage spaces/channels, add existing accounts by username, and create Discord-style invite links (`/invite/<code>`, expiry/max uses/landing channel; owner-removed or blocked accounts cannot rejoin by link). Text uses Postgres/outbox and the independently deployed `--gateway` WebSocket role with Valkey fanout. No custom roles, video calls, or screen sharing (the attach drawer's camera only captures photos and videos to attach).
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

`main` is production. A push to `main` reaches users within minutes, with no
pull request, review or deploy button in between. Speed beats ceremony: there
are no real users yet, so ship small, watch it land, and fix forward.

Before you push:

- Run the local checks for what you changed (above) and build what you
  touched. CI on `main` is a signal, not a gate. Nothing waits for it, so it
  cannot stop a mistake from reaching users.
- Push only what you would be happy for users to have right now, and keep
  every commit deployable.
- Write database migrations to be additive and backward compatible. The API
  applies them on startup, while the previous gateway and web still run
  against it. Leave dropping or rewriting data to a later, owner-approved
  change.
- Push a branch and open a draft pull request instead of pushing to `main`
  when:
  - the work is unfinished and you have to stop (session ending, disk full,
    missing toolchain). The draft PR is the handoff: say what is done and what
    is left.
  - it needs a product decision the owner hasn't made.
  - it is irreversible: destructive or data-rewriting migrations, deleting
    data or infrastructure, rotating secrets. Ask the owner first.

Push and deploy:

- Push with `git pull --rebase origin main` (rerun checks if that pulled in
  changes to code you touched), then `git push origin HEAD:main`, and retry
  the same way if the push is rejected. Never force-push `main` or rewrite
  other agents' commits.
- Never put literal CI-skip directives in commit messages, even to explain
  them: the path-filtered image builds would silently not run.
- Pushes to `main` run `CI` (a few minutes) and, when their paths changed,
  `API image` (API and gateway) and `AWS image` (web), about 2-3 minutes each.
  Each image run then dispatches `Ship` in `joswayski/infrastructure`. Ship
  rolls production forward to the newest published images on `main`:
  - in order: API, then gateway, then web;
  - never backwards;
  - holding the web while a newer API build is still running or failed.

  You don't coordinate deploys with other agents or the owner. Ship serializes
  them. If the image run's "Ship to production" step says Ship is not
  configured, dispatch it yourself once your image runs succeed. Dispatching
  again is harmless:
  `gh workflow run ship.yml --repo joswayski/infrastructure --ref main -f repository=caper`.
  Don't run the per-app `deploy-caper-*.yml` workflows for routine changes.
- Native clients release by themselves: a push that changes the native apps
  starts `Native release` (`release.yml`, 20-40 minutes). It hands builds to
  testers and publishes the desktop update only after the API image and Ship
  for caper have rolled out the servers it needs. Don't dispatch it yourself
  for routine changes. The Intel Mac build finishes on its own and never blocks
  the release. `Native development builds` runs nightly and on demand; don't
  start it for routine changes.
- Infrastructure or secrets the change needs go to `joswayski/infrastructure`
  first, following that repository's guidance.
- Watch it land. `SHA` is your pushed commit:
  - its runs (CI, images, `Native release`): `gh run list --repo joswayski/caper --commit "$SHA"`, then
    `gh run watch <id> --repo joswayski/caper --exit-status`;
  - Ship runs: `gh run list --repo joswayski/infrastructure --workflow ship.yml --limit 3`.
    Each run's summary says what shipped and what waits.

  Report what reached production (your `SHA` or a newer one). Without `gh`,
  use the GitHub MCP Actions tools; Claude sessions need
  `joswayski/infrastructure` added to dispatch Ship.

When something breaks:

- If `CI`, an image build, Ship or a release fails, fix it forward with
  another commit. If the fix will take more than a few minutes, `git revert`
  the bad commit and push; Ship deploys the revert like any other commit.
- A deploy workflow's `rollback_from_sha` gives immediate relief, but only
  until the next Ship rolls forward again. Always follow it with a revert or
  fix.
