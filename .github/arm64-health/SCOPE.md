# Inactive, never-merge ARM64 health gate

Preparation branch: `apple-arm64-health-sidebar-prepared`. Only a separately
approved push to `apple-arm64-health-sidebar-run` triggers this workflow. No
dispatch, schedule, existing-ref change, retry, production merge or cutover.
One hosted VM/job; all results remain `productAcceptance: false`.

## Exact inputs and environment

Product checkout: `9fca8b93f3629390e837794b9468c5cdc18d7617`, tree
`1fdd75c82099d48f375e275ed105c93cfcd6c316`. No overlay, assertion or product edit.
Test file SHA256: `38658ea7cdd3ea9a8625d54f589ef8f663ed4df2a111883409e7c7ced948a832`.

Fail closed unless GitHub-hosted, attempt 1, exact execution branch, ARM64,
`ImageVersion=20261102.0473.1`, macOS 26.7.2/build 25G332 and Xcode 26.6/build 17F113
at `/Applications/Xcode_26.6.app/Contents/Developer`. Label `macos-26` is not an
immutable image selector: an unexpected assigned version fails preflight.
Official image documentation is pinned at actions/runner-images commit
`b5e4fd2931679d96bf0c289a03f54d414633af0f`:
[macos-26-arm64 Readme](https://github.com/actions/runner-images/blob/b5e4fd2931679d96bf0c289a03f54d414633af0f/images/macos/macos-26-arm64-Readme.md).
Availability and observed provenance are not runtime health.

`control-provenance.json` maps all unchanged control files to their exact earlier
commits/paths and hashes. The complete 3836-byte published graphics fixture is
unchanged, including its one-second lifetime and all unused branches. Only
`hosting`, `hosting-canvas`, `hosting-blur` execute, once each. The ordinary-menu
AppKit application, XCTest source and XcodeGen project also remain byte-identical.
No menu/AX/renderer/default/permission/focus workaround. The original AppKit
startup activation remains, not an added automation focus action. Pixel analysis
and Metal provenance are separate helper processes, never linked into controls.

## Health controls and usable evidence

After provenance/baseline/pixel-helper compilation, execute the graphics group
and ordinary-menu AX check. AX still executes if graphics fails, to preserve both
health outcomes. Neither causes any Caper preparation/build until the joined
health gate passes. Missing reports, any crash/nonzero/timeout, export failure,
attempted capture failure or incomplete crash inventory fails the gate.

Graphics: compile the original fixture locally, run the three modes once and
retain real returnCode/signal/timeout, per-process duration, stdout/stderr and
exact begin/survival markers. Read-only Metal/device/capture-permission preflight
never requests permission. Missing capture permission is unhealthy, not a skip.

For each live graphics process, attempt ONE capture after 0.2 seconds. A separate
read-only helper finds exactly one on-screen, layer-zero window owned by that
PID (2s bound); `screencapture -x -o -l<windowID>` captures it once (5s bound),
without moving/focusing the window. No window, early exit or missing pixels fails
closed; the fixture's lifetime is never extended. Offline pixel checks (60s bound)
decode the image and require the Text control's exact phrase via Vision OCR, or
the expected green shape: green pixels >5% and <95%, with horizontal span >70%
for the Canvas ellipse or 15–60% for the blur circle. These are conservative
content/usability gates, not a visual fidelity oracle; human inspection remains
required. A surviving process with blank/unusable pixels is not healthy.

AX: only `AXMenuTests/AXMenuTests/testInitialTextFieldAndFullHierarchy`, once.
Retain full unfiltered AX hierarchy with the field and menu bar/File/Edit,
keepAlways attachments, screen recording, real counts and diagnostics. Require
exactly one matching start/end, 1 pass/0 fail/0 skip/0 expected-fail and exit 0.
The unchanged control emits elapsed time across its initial query AND complete
hierarchy capture: health requires one begin/end pair and finite elapsed time
in [0,15] seconds. A slow recovered query fails even if XCTest eventually passes.
This is a separate reported health bound, not an increased XCTest timeout.

Only if the query remains in flight more than 15 seconds after its begin marker
is observed, attempt one concurrent app/launchd five-second sample pair (15s
bound each). No sampling perturbs the allowed healthy interval. Completion before
sampling is explicitly not_attempted; capture errors are separate collection
failures, never called reproduced stalls. Old artifacts/reason fields are not
rewritten. The recording must decode and contain a frame with the standalone
text-field phrase: inspect up to nine fixed interior timestamps offline, save
the first matching frame and all inspected OCR results. No recapture/relaunch.

## Crash inventory without the historical-copy trap

Before any controls, inventory relevant user/system DiagnosticReports paths with
size, mtime/ctime/inode and SHA256, without copying/deleting historical reports.
After graphics, AX and (if admitted) sidebar, inventory again against this SAME
baseline. Every unchanged historical exclusion is explicit. Copy only new or
changed relevant reports with original paths/hashes; disappearance or a report
changing during its read makes the inventory incomplete and fails collection.
No absent or empty report list claims no crash occurred.

Each inventory/capture subprocess is bounded 30s. Scope: iconservices, launchd,
AXMenu, Caper, Dock, Spotlight, graphics-control/IntelGraphics, pixels,
metal-provenance, xctest and screencapture `.ips/.crash/.spin` reports. Inventory
bounds: 20,000 directory entries, 2,048 matching files, 20MB/file, 256MB read total.
Current-report retention bounds: 64 files/80MB total. Bounds/read/copy errors
retain an atomic partial manifest with complete:false and explicit reason, and
fail health even if functional controls pass. Historical files never consume
the 64-file current-report cap. Each root's inventoried/absent/incomplete status
is recorded. Any new/changed relevant report conservatively blocks health even
if collection succeeds; it is not automatically labelled an AX reproduction.
Existing old run artifacts remain untouched.

## Sole Caper test after health passes

Only then checkout the exact product pin, restore the same pinned RNNoise inputs,
prepare its ordinary dependencies/all five Rust slices, generate the existing
development project and build Debug `CaperAppKit` for testing. Existing core/view
bundles compile as scheme dependencies; no Caper UI scheme or shipping app builds.
Execute only:

`CaperAppKitViewTests/NavigationSidebarTests/testVoiceCardPreservesFocusAndLiveRowWhenChannelsCollapse`

That single method covers both explicit scroller-style fixtures with fixed
expectations and all reviewed focus/containment/collapse/leave assertions. No
37+6 suite or local API server. Keep xcresult, exact counts/summary/logs, original
test source, generated schemes/xctestruns and binary hashes. A standalone success
does not itself accept any Caper behavior; the singleton must pass independently.

Native timeouts remain default 300/max 600 and 900s external test subprocess cap;
no retry or timeout inflation. Process-local curl/Cargo settings disable existing
dependency download retries without source changes. Step caps: preflight 5min,
graphics 10min, AX 25min, sidebar 70min; job 110min. Step cancellation/missing output
is failure, never assumed success. Final summary and upload run even on failure.
