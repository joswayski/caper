# Inactive, never-merge ARM64 health gate

Preparation branch: `apple-arm64-health-sidebar-prepared`. Only a separately
approved push to `apple-arm64-health-sidebar-run` triggers this workflow. No
dispatch, schedule, existing-ref change, job retry, production merge or cutover.
One hosted VM/job; all results remain `productAcceptance: false`.

## Exact inputs and environment

Product checkout: `9fca8b93f3629390e837794b9468c5cdc18d7617`, tree
`1fdd75c82099d48f375e275ed105c93cfcd6c316`. No product or test-method edit.
Sidebar test SHA256: `38658ea7cdd3ea9a8625d54f589ef8f663ed4df2a111883409e7c7ced948a832`.
The separately hashed `ax-capture.patch` is evidence-only: the same eight lines
used by combined harness `a7aff5300585845273ea9e33d54eeddb03401c2c`, adding full
debugDescription attachments to the two existing capture helpers. Original UI
source SHA256 `42785b0654aa6934519ce9bf08e72c9d2e88de84c29cdffd0d8b13470e19e8c8`
and observed SHA256 `0f0d4665aca9c6848c89900fe1c63f6a3746cff0a37669d1feabb21705e13a5c`
are checked before/after application. Retain both sources, patch and raw diff.
No assertions, fixture behavior, method bodies, widths or preferences change.

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

For each live graphics process, start ONE read-only readiness helper immediately,
with the unchanged external 2s process cap (including startup). Within that cap,
it observes only that PID's on-screen windows, every 50ms for at most 1.5s, stopping
on process exit, permission/inventory error, malformed fields, multiple eligible
windows or exactly one eligible layer-zero window. Only no-window observations
continue polling; no ambiguous candidate is selected. Signal 0 checks process
existence/access without sending a signal. Atomic `window-observations.json`
snapshots retain elapsed times, PID-specific candidate fields/classification,
eligible counts, terminal status and completeness; a killed helper can leave a
partial snapshot. Expected discovery/decoding errors emit structured JSON and
ordinary nonzero exits, not uncaught Swift errors. A collection/encoding failure
can still leave incomplete inventory; it fails closed, never becomes a healthy
control or an attributed renderer failure.

The supervisor rechecks the ORIGINAL fixture's process immediately before
`screencapture -x -o -l<windowID>`: at most ONE screenshot (5s bound), without
moving/focusing any window. Readiness observations are not repeated controls or
captures. No capture retry, fixture restart or extension of its one-second
lifetime. Process/window lifetime checks and capture cannot be atomic: a fixture
may exit or its window change after the last observation, startup can exhaust
the bound, and a ready window need not yet contain usable pixels. Those races
still fail closed; early discovery is not assumed to explain the earlier missing
captures, whose helper did not retain enough candidate inventory.

Offline pixel checks (60s bound) decode the image and require the Text control's
exact phrase via Vision OCR, or the expected green shape: green pixels >5% and
<95%, with horizontal span >70% for the Canvas ellipse or 15–60% for the blur
circle. These are conservative content/usability gates, not a visual fidelity
oracle; human inspection remains required. A surviving process with blank/unusable
pixels is not healthy.

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

Offline video decoding sets both `requestedTimeToleranceBefore` and
`requestedTimeToleranceAfter` to `.zero`. Apple's
[timing documentation](https://developer.apple.com/documentation/avfoundation/avassetimagegenerator/requestedtimetolerancebefore)
specifies infinite defaults and zero on both properties for frame-accurate
generation. [Additional decoding can cost time](https://developer.apple.com/documentation/avfoundation/creating-images-from-a-video-asset),
but the existing nine-sample maximum and 60s external helper cap remain unchanged.
Keep the original desired `requestedSeconds` and returned `actualSeconds` for
every inspected frame; never replace actual timestamps with requested ones.
OCR/content requirements remain unchanged. Decode errors, absent content and
timeouts still fail closed; no broader tolerance, fallback sampling or retry.

## Crash inventory without the historical-copy trap

Before any controls, inventory relevant user/system DiagnosticReports paths with
size, mtime/ctime/inode and SHA256, without copying/deleting historical reports.
After graphics, AX and (if admitted) cumulative validation, inventory again against this SAME
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

## Exact cumulative selection after health passes

Only then checkout the exact product pin, restore the same pinned RNNoise inputs,
prepare its ordinary dependencies/all five Rust slices, generate the existing
development project and build Debug `CaperAppKit` and `CaperAppKitTests` for testing,
once each, as in the earlier combined harness. No shipping scheme build.

Before preparation or the capture overlay, preserve the earlier combined
requirement for `NSScreen.main.visibleFrame` at least 1440×900 points. Record the
actual frame and keyboard-navigation state; a smaller or absent desktop exits
normally nonzero. Do not change resolution, defaults, test widths or assertions.
The ARM64 image pin alone does not guarantee this geometry; standalone health
and the narrow sidebar do not establish wide UI feasibility. No execution is
implied by this inactive preparation.

The workflow's RNNoise source curl has its own narrow transfer-recovery exception:
`--max-time 60 --retry 2 --retry-delay 5 --retry-max-time 120 --retry-all-errors`.
At most three transfers, not repeated dependency steps. Retrying all curl errors
also covers the observed HTTP 504 with curl exit 56, without assuming its network
cause; checksum verification is outside curl and is never retried. The source
URL/hash, model fetch/hash and extraction order remain unchanged. Any final curl
failure or hash failure stops before extraction. Neither the git fetch nor the
preparation/build/test is retried by this exception.

[curl's retry budget](https://curl.se/docs/manpage.html#--retry-max-time) is not a
total wall-clock timeout: an admitted transfer can outlast it. Allow conservatively
185s for this command (120s retry window + 5s delay + 60s final transfer), apart
from scheduling overhead; older curl versions check the budget before sleeping.
An over-budget `Retry-After` stops retries instead of adding that sleep. The unchanged
300s dependency-step cap includes the single git fetch, both hash checks and both
extractions, and can cancel sooner if they consume the remaining time. No guarantee
of download completion, deadline extension, cache/endpoint substitution or softened
hash gate follows from enabling transfer recovery.

Before either build, run `xcodebuild -resolvePackageDependencies` with the
same project, CaperAppKit scheme, configuration, destination and DerivedData. At most three
resolution attempts, each capped at 120s, separated by 5s only after the exact
WebRTC 153 binary-download diagnostic `Could not connect to the server.` with
exit 74. The overall resolution budget is 370s, including delays; the existing
70min cumulative/110min job caps still apply and can stop the work earlier.
Require the recognized diagnostic/wrapper/progress output without other errors:
unknown wording, another URL, integrity/checksum/signature/version errors,
signals, launch failures, timeouts and exhausted bounds stop without retry.
This deliberately does not cover every possible transient transport error.

Each attempt retains raw process results/stdout/stderr, the retry decision and
read-only copies/hashes of any `Package.resolved`, SwiftPM workspace state and
checkout manifests present. Missing files are recorded. A successful resolution
must provide the two declared pins (WebRTC 153.0.0, libwebp-Xcode 1.6.0); malformed
or different pins stop even after a transport failure. No package edits, alternate
endpoint, hand-populated cache or checksum bypass. Standard SwiftPM binary
[checksum validation](https://developer.apple.com/documentation/xcode/distributing-binary-frameworks-as-swift-packages)
remains enabled. Both builds and all test selections use `-disableAutomaticPackageResolution`,
as [Apple recommends for CI](https://developer.apple.com/documentation/xcode/building-swift-packages-or-apps-that-use-them-in-continuous-integration-workflows),
to preserve resolved versions. That flag is not an offline guarantee: a later
build can still encounter a download failure, which is never retried here.

`selections.json` pins all 43 exact identities against combined harness a7 and
its product `5910cfa98ee038fe53204b333c7065879f29a9ba`. Execute its original
`CaperCoreTests/MessageFormattingTests` + `CaperAppKitViewTests` selection once
(14+23), then the same six UI singleton selectors in their original order. The
corrected sidebar is already one of the 23; never invoke it separately. Exact
start/outcome sets AND counts must match; duplicates, omissions, extra methods,
failures, skips or expected failures reject the selection.

Reuse Node 24 and the product's unmodified `scripts/native-parity-fixture.mjs`.
It uses Node built-ins only, without npm installation. One local fixture process,
same bounded health-readiness loop (100 observations, 1s request timeout, 0.2s
delay), unchanged per-method reset/control behavior and process-group cleanup.
No production API, SFU, external fixture or personal hardware.

Both generated schemes retain keepAlways system/user attachments and successful
screen recordings. Retain both original/modified schemes, generated xctestruns,
binary hashes, package metadata, build result bundles and every test xcresult,
including failures. All three exports are attempted for each test result even
after XCTest failure. Independent test/evidence failures are aggregated without
repeating a method; missing/malformed exports fail that selection and do not
erase other results. Preparation/build failure stops before test invocation.

For each UI singleton, require nonempty recordings plus exactly one screenshot
and full AX tree for each of its existing named checkpoints (24 pairs total).
These include delivered reply context, original-message navigation, rotated
spoiler, pending/confirmed image and decoded viewer. No added capture calls or
test-method modifications. The final cancelled-quote send and viewer-close/focus
restoration rely on the unchanged assertions and whole success recording rather
than a new AX checkpoint. Human/agent inspection must verify the complete clips,
post-action states, all relevant pixels and full hierarchies; file presence is
not usable-media or product acceptance. `productAcceptance` stays false.

Native timeouts remain default 300/max 600 and 900s external test subprocess cap;
no test retry or timeout inflation. Download recovery also preserves the product script's existing
curl transfer retries (`--retry 6 --retry-delay 5 --retry-all-errors`), including
its archive hash verification. No curl override, new endpoint/version/cache or
generic preparation retry wrapper. `CARGO_NET_RETRY=0` and the 2700s preparation
cap remain unchanged. Preparation and each scheme build/test selection run at most
once; transfer recovery is not permission to repeat them or relax any gate.
Step caps: preflight 5min,
graphics 10min, AX 25min, cumulative 70min; job 110min. The sum of individual
worst-case preparation/build/test/export limits exceeds the outer cap; those are
ceilings, not reserved time. Cancellation may prevent later selections or exports,
which remains incomplete failure, never a skip, implicit pass or retry grant.
Final summary and upload run even on failure when the runner permits them.
