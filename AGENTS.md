# Repository guidance

Adapted from the conventions in `joswayski/captures`.

## Product and repository map

- Caper is an early-stage space for conversations. Distinguish working features from roadmap ideas; do not describe mockups as shipped functionality.
- `apps/web`: TanStack Start browser app and same-origin development API adapter.
- `apps/api`: Rust media control, accounts, spaces and unified text/voice channels. Cloudflare Realtime SFU/TURN carries audio; AWS carries control traffic. The public General demo is retired; the homepage shows a labelled local simulation linking to email sign-in. Account spaces require membership, and private channels have explicit member grants. Owners manage spaces/channels and add existing accounts by username. Text uses Postgres/outbox and the independently deployed `--gateway` WebSocket role with Valkey fanout. No invite links, custom roles, camera, or screen sharing.
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

- Use shared tokens: blackout #0C0D0F, surface #151719, border #34383B, text #F3F4F5, terracotta #B64D32, caper green #637A43. Satoshi typography; restrained borders and 8px control corners.
- Terracotta is for primary actions, selection, and focus. Green primarily belongs to the character, with restrained presence/status use. Keep general chrome neutral.
- Build hierarchy with typography, spacing, and clear layouts rather than large color fields. Fictional participants/messages belong only in the explicitly labelled homepage simulation; never use them in real account spaces or add working-looking controls for unimplemented features.
- Inspect rendered desktop/mobile and affected non-default browser states before claiming visual completion. Use real browser interactions or explicitly labeled test mocks.

## Documentation and validation

- Leave README accurate and concise. Put operational detail in `docs/media.md`; maintain an honest platform/test matrix.
- Run `npm run check` (Oxlint, Oxfmt, Vite build and native TypeScript 7 typecheck) and `npm test` (Vitest web and shared native-support tests); use `npm run fmt` to format first-party JS/TS/CSS. For Rust, run `cargo fmt --all -- --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- For Python tooling, use pinned uv and `uv.lock`: `uv run --locked ruff check .`, `uv run --locked ruff format --check .`, `uv run --locked python -m unittest discover -s tests -p 'test_*.py' -v`, and `uv run --locked python shared/fonts/test_native_fonts.py -v`. Use `uv run --locked ruff format .` to format Python; keep release/authoring dependencies in their separate groups, not the global interpreter.
- Build changed Docker images when a daemon is available; otherwise validate build stages directly and report the limitation.
- Never equate mocks with live SFU validation. Record multi-network/TURN, sustained voice, and physical device checks separately.
- Always open a focused, ready-for-review GitHub PR for every change once it is pushed, without waiting to be asked; never push directly to the default branch. Include exact post-merge operator commands for deployment/configuration changes.
- Don't run CI on pull requests. Never start or re-run `CI`, `Native development builds` or any other workflow on a PR branch, and don't suggest it in PR descriptions; it takes too long. Validate locally, merge to `main` (pushes to `main` run CI), and fix anything that breaks there with a follow-up PR.
- Squash merges default to copying the PR description into the commit body. Never include literal CI-skip directives in PR titles/descriptions, even to explain that they are unnecessary: GitHub still honors them inside prose or code fences. These workflows already omit PR triggers, so no skip directive is needed. When authorized to merge, supply an explicit clean `gh pr merge --squash --subject ... --body ... --match-head-commit ...`, then inspect the resulting commit message and verify runs for that exact SHA, not an earlier commit. Main CI success alone does not prove the separately path-filtered native and image builds ran.
- Every PR must include a **Deployment order** section with numbered, step-by-step rollout instructions. Cover infrastructure, secrets/configuration, database migrations, services, and client releases when applicable, including prerequisites and exact operator commands for infrastructure/configuration changes. State explicitly when a step is unnecessary, when components can deploy independently, or when no deployment is required. Include relevant verification and rollback notes; do not imply merging deploys components automatically.
