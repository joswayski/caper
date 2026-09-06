# Repository guidance

Adapted from the conventions in `joswayski/captures`.

## Product and repository map

- Caper is an early-stage space for conversations. Distinguish working features from roadmap ideas; do not describe mockups as shipped functionality.
- `apps/web`: TanStack Start browser app and same-origin development API adapter.
- `apps/api`: Rust media control service. Cloudflare Realtime SFU/TURN carries media; AWS carries control traffic. The anonymous public lobby has no accounts or durable database.
- `apps/desktop`: independent Tauri shell and React UI. Browser WebRTC success does not prove native webview support.
- `shared/design.css`: shared visual tokens. `docs/brand` contains the owner's reference images and style guidance.
- `docs/media.md`: configuration, deployment, privacy, validation matrix, and call runbook.

## Working conventions

- Keep changes focused; reuse existing patterns before adding dependencies or abstractions.
- Other agents may work concurrently, particularly on desktop. Use an isolated worktree for new concurrent work; never stash, overwrite, or publish another agent's changes.
- Keep macOS, Windows, and Linux parity explicit. Document unsupported or untested capture/device behavior instead of assuming browser and Tauri APIs are interchangeable.
- Never expose provider secrets or log SDP, credentials, or raw media. No unrestricted Cloudflare API proxy.
- Keep one authoritative media registry. Do not scale the API beyond one instance without shared coordination and recovery; web replicas are independent of this constraint.

## Visual design

- Use shared tokens: blackout #0C0D0F, surface #151719, border #34383B, text #F3F4F5, terracotta #B64D32, caper green #637A43. Satoshi typography; restrained borders and 8px control corners.
- Terracotta is for primary actions, selection, and focus. Green primarily belongs to the character, with restrained presence/status use. Keep general chrome neutral.
- Build hierarchy with typography, spacing, and clear layouts rather than large color fields. Do not add fake participants, messages, or working-looking controls for unimplemented features.
- Inspect rendered desktop/mobile and affected non-default browser states before claiming visual completion. Use real browser interactions or explicitly labeled test mocks.

## Documentation and validation

- Leave README accurate and concise. Put operational detail in `docs/media.md`; maintain an honest platform/test matrix.
- Run `npm run check` and `npm test --workspace @caper/web`; for Rust, `cargo fmt --all -- --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- Build changed Docker images when a daemon is available; otherwise validate build stages directly and report the limitation.
- Never equate mocks with live SFU validation. Record multi-network/TURN, sustained share, and device checks separately.
- Use focused, ready-for-review GitHub PRs, not direct default-branch pushes. Include exact post-merge operator commands for deployment/configuration changes.
