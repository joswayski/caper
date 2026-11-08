# Inactive, never-merge ARM64 forwarding observation

## Inactive retained-video phase replay proposal

`video-replay-workflow.yml` is a template OUTSIDE `.github/workflows`; no existing
trigger, forwarding workflow or supervisor command changes. Activation and any
execution require separate exact-pin review. `video_replay.py` invokes only the
pixel helper, once for each of two retained, hash-pinned local recordings. It
does not call the health/Caper/session entrypoints, create a UI, change display
or preferences, produce new recordings, install dependencies or launch tests.
It imports only existing process/result helpers; importing `run` executes no phase.

The proposed dedicated branch is `apple-video-phase-replay-run`, attempt1 only,
GitHub-hosted ARM64 image `20261102.0473.1`, macOS `26.7.2`/`25G332`, Xcode
`26.6`/`17F113` at `/Applications/Xcode_26.6.app/Contents/Developer`. Verify actual
identity, clean checkout and input hashes before compiling Pixels once (180s).
Run failed then passing recording once each (unchanged 60s subprocess caps), with
independent outcomes and no retry even when the first fails. The 7min step/10min
job caps can terminate earlier; partial stdout/stderr/results upload on failure.
The helper stops at the first thrown error or first content pass as before;
never skip a bad sample. Neither replay result changes any historical outcome.
Source/media/binary hashes and before/after media integrity are retained. Native
compilation, AVFoundation behavior and timing effects require hosted validation.

`video-replay/failed.mp4` is the exact standalone attachment
`41B4ED30-211F-4DC7-B71F-48F18DCD58D8.mp4` from run `42496005325`/attempt1,
head `a78b2969465c4be09c26172396bf927392dc31ac`, artifact `13623279072`;
SHA256 `449df2b898b9b5b907a9989aeae75bf002035c69f18456912dd47a1faf8cd42c`, 51285 bytes.
`video-replay/passing.mp4` is the exact standalone attachment
`B58CC12B-C663-472D-9FD0-F979B12E070C.mp4` from run `42491607243`/attempt1,
head `c01b43613f24a162acd422ee4f4df05a28da61cf`, artifact `13621941721`;
SHA256 `4d490dea2913e03f82e8de57ca93d952b1a8919f00d461b1ee9aba8194ede597`, 102694 bytes.
These names identify prior pixel outcomes, not expectations imposed on a replay.

Video-only `PIXELS_VIDEO` stderr JSONL retains monotonic sequence/uptime, duration
load start/result (raw CMTime and finite seconds), each request index and nominal
seconds/rounded CMTime, returned actual CMTime, inspection start/result, PNG and
output stages. Existing final stdout results stay intact; thrown errors additionally
retain the last stage and prior events. NSError traversal follows only
`NSUnderlyingErrorKey`, at most four unique objects, with explicit cycle/depth/
non-NSError endings. Only domain (128 characters), code, message/failure reason
(512 characters each) are retained; no userInfo dictionary, URL or arbitrary-key
dump. The existing primary top-level error is preserved. Events are capped at64
(current paths need at most44); incremental stderr survives a later throw/timeout.
Diagnostic serialization failure has an explicit stderr marker, not a passed
observation. This synchronous logging adds overhead; it is not timing-neutral.
The nine interior times, 600 timescale, zero tolerances, preferred transform,
Vision content criterion, first-error behavior and all nonvideo modes are unchanged.

Preparation branch: `apple-arm64-health-sidebar-prepared`. Only a separately
approved push to `apple-arm64-health-sidebar-run` triggers this workflow. No
dispatch, schedule, existing-ref change, job retry, production merge or cutover.
One hosted VM/job; all results remain `productAcceptance: false`.

## Proposed forwarding-only scope (requires separate execution review)

The workflow now calls `forwarding`, not the retained `cumulative` entrypoint.
It uses the same exact product, fresh health, session setup/restore, preparation,
package recovery, two build-for-testing invocations, fixture and test flags below.
It selects ONLY `CaperAppKitTests/CaperAppKitTests/testForwardPickerCancelLoadRetrySearchAndPartialDeliveryPreserveDrafts`,
once. Zero non-UI or other Caper UI selections execute. The singleton's entire
original body, gestures, assertions and five existing checkpoints remain; only
its FIRST note replacement is observed. The `cumulative` output directory/step
ID is retained for the unchanged finalization and cleanup contracts. The original
37+6 entrypoint/manifest described below remains available but is NOT invoked.

After the unchanged AX-capture overlay, `forward-note.patch` adds observation
calls; `ForwardNoteObservation.swift` is copied into the diagnostic core and
UI-test source directories only. No shipping source/ref is edited. Original and
observed source hashes are checked, and the helper, patch and resulting sources
are retained. No alternate input, sleep, polling, retries, throwing assertions,
forced focus/editability or responder/menu change. Instrumentation adds CPU,
clipboard readback, synchronous metadata I/O and event-monitor overhead; it is
not timing-neutral and can perturb a race. The local monitor returns the exact
received event without consuming or redirecting it.

App observation requires the development bundle, parity fixture, exact loopback
API and a unique test-created temporary directory passed only by this singleton.
Only the `forward-note` view registers. A readiness state/monitor receipt is
written independently of paste, then first-replacement events are gated by an
atomic marker. The first helper writes the marker after its original pasteboard
write and closes it on helper exit (including normal assertion-failure return).
No subsequent replacement arms it. If abnormal test exit prevents closure, a
12-second monotonic observation bound and 256-record-per-process cap stop further
records; reaching either bound is incomplete evidence, never an implicit pass.
There is no timer or observation polling. Temporary metadata is not deleted from
the disposable VM before collection.

Each record has a process/stream identity, contiguous sequence and machine-uptime
nanoseconds; cross-process timestamps permit correlation but do not make readings
atomic. Weak-key UUID identities distinguish live view/responder instances from
recreation, without logging their descriptions or contents. Record app/key-window,
actual first responder, note native/SwiftUI flags, UTF-16 text/binding lengths,
exact string-inequality resync decision, focus callback results, paste entry/exit
and textDidChange before/after. The initial ready snapshot is update-entry state;
in-interval event snapshots independently report state even if paste never runs.

The first armed mouseDown monitor records event window opaque identity/number,
event number/timestamp and window-base point; note and enclosing clip visible
rectangles are converted to that same window-base space. It makes at most one
predictive `contentView.hitTest` query, using the receiver's **superview** coordinates
as [AppKit requires](https://developer.apple.com/documentation/appkit/nsview/hittest(_:)).
Note-local and query-point round trips back to window-base are retained separately.
Missing/mismatched windows or missing content/scroll views retain an unavailable
reason and never trigger another query. Recipient ancestry is limited to 16 nodes,
with opaque IDs, classes and equality roles for note/clip/scroll/root/host. Host
identification explicitly uses the nearest `NSHostingView` class-name prefix in
at most 16 note ancestors; absence/truncation is not inferred host identity.
Ancestry truncation is recorded, not presented as a complete hierarchy.

The diagnostic overlay also wraps `ComposerTextView.mouseDown`: its original event
is forwarded to `super` exactly once, unconditionally. Helper entry/exit observations
require a registered note and the same active first-replacement interval and retain
responder state and event identity. No other composer logs handler observations.
Prediction and actual handler evidence are assessed independently: a predicted note
with zero handler calls is not delivery, and handler delivery need not acquire focus.
Missing exit, event/view mismatch or incomplete coordinates remain evidence failures,
independent of XCTest outcome. The query, conversions, bounded ancestry traversal,
guard checks and synchronous handler logging add overhead and may perturb timing;
querying hitTest invokes framework code and does not prove the eventual recipient.
There is no swizzling, substitute event, forced focus/layout, gesture/overlay change,
delay or retry. This instrumentation is not a product fix.

The test records pasteboard write result, before/after changeCounts and separate
readback-presence, exact-canary-match, type-policy and count-stability booleans,
retaining the original compound readback result. A failed write skips content
readback; false presence then means not read, not proof the board was empty.
No unexpected type names or contents are emitted. Readback digest/types/length
are emitted only if readback exactly matches its just-written synthetic string,
the only declared type is string and changeCount remains stable. Otherwise record
the mismatch without the unexpected contents, digest or types. At the registered
note's armed paste entry, compare current/before-and-after changeCounts against
that original marker. Content inspection is permitted only when the original
write/readback succeeded with the exact pinned synthetic canary metadata, the
count is unchanged and only the string type is advertised. Recheck count after
reading; emit digest/types/length only for byte-matching canary content with
unchanged count. Changed provenance skips content inspection; unavailable or
unexpected readback never emits its contents, digest or types. Preserve the
original test observation independently, including failed writes/readbacks.
Explicit count and readback statuses distinguish unchanged/changed counts,
matching canary, unavailable readback and unsafe-provenance skips. Collection
requires typed metadata and consistent statuses, not successful clipboard
operations or a paste callback. No raw clipboard/message text, key characters,
credentials or other applications' data are recorded. Count/type/content reads
are not atomic: a concurrent change can occur between the guard and read, but
mismatched content is never hashed or emitted. This adds synchronous observation
overhead, not input modification, waiting or recovery.

Records go to stderr AND bounded per-stream JSONL files, so successful tests need
not rely on failure-only diagnostic exports. The collector copies only this
test's reported UUID temporary directory, either the supervisor temporary root
or the current user's exact development XCTest runner container's `Data/tmp`.
It never searches containers. Component-wise no-follow directory descriptors and
bounded regular, single-link event-file reads reject symlinks and traversal;
reports retain the failing stage, reported path and accepted root provenance.
At most two 1MiB JSONL files are copied. An explicit offline console-recovery
function accepts at most two caller-selected 16MiB logs, retains source hashes
and line numbers separately, and never replaces the live collection report.
Legacy fields are not inferred, so legacy streams remain incomplete under the
new schema. This fallback is not invoked by the workflow. Collection retains hashes and a
monotonically ordered view, and checks stream completeness independently of
XCTest success. Missing readiness, missing in-interval state, unmatched callbacks,
limits or collection failures remain incomplete; zero paste callbacks with
independent state is a meaningful observation, not a root-cause diagnosis.
Collection runs even after test/export failure and never substitutes for the
primary assertion, stdout/stderr, xcresult, AX attachments or full recording.
The existing health/session/cleanup contracts and all caps below are unchanged.

## Temporary session display before full health controls

On this disposable hosted VM only, `display-setup` first checks exact provenance,
then compiles the read-only inventory and session transaction helpers. It observes
one active, awake, unmirrored main display and freshly enumerates default advertised
modes with `CGDisplayCopyAllDisplayModes(id, nil)`. Exactly one GUI-usable mode must
have width/height AND pixelWidth/pixelHeight 1600×1200. No hard-coded mode ID,
fallback resolution, virtual display, renderer, defaults or guest utility change.
The original mode must also be uniquely present in the advertised list.

Original full geometry/mode, selected mode and run identity are saved atomically
before any mutation. The helper rechecks topology/current mode, then performs at
most one `CGBeginDisplayConfiguration` / `CGConfigureDisplayWithDisplayMode` /
`CGCompleteDisplayConfiguration(..., .forSession)` transaction. API statuses and
current/target/advertised modes are retained, including on failure. An already
matching mode needs no transaction. A pending failed transaction is cancelled;
completion invalidates its token even on error, so it is not cancelled again.

Apple documents [atomic completion](https://developer.apple.com/documentation/coregraphics/cgcompletedisplayconfiguration(_:_:))
and [session lifetime](https://developer.apple.com/documentation/coregraphics/cgconfigureoption/forsession):
the setting survives the setter's exit and reverts to the saved permanent
configuration at logout. No `.permanently`, caller-scoped setter or permanent
configuration restore is used. This does not establish that selection succeeds
on a particular hosted VM; errors remain failures, without another switch.

After the setter exits, fresh inventory processes observe settling at most five
times within a 10s observation budget (2s/process, 0.2s between observations).
Only successful, complete single-display observations may be repeated; API,
process, signal, timeout and malformed/incomplete inventory failures stop.
Require the actual selected mode, 1600×1200 screen frame, backing scale1 AND
`NSScreen.main.visibleFrame` >=1440×900 **points**. Nominal pixels do not admit
controls. The snapshot is not atomic; all raw observations are retained.

Only successful setup admits a fresh preflight/crash baseline and full unchanged
graphics/AX/video/crash health controls on this VM. Nothing reuses prior health.
The earlier visible-point check before product preparation remains; another fresh
mode/scale/usable-point check occurs after builds, before the fixture/tests start.
Product pins, test bodies, selection and evidence-only overlay stay unchanged.

An `always()` cleanup attempts the saved original session mode at most once,
using freshly advertised modes, the same run identity and hashed helper binaries.
It refuses changed display identity or an unexpected third-party mode. It then
observes the original mode/frame/scale; current usable geometry is retained but
need not meet the test-size requirement. Missing original state means no mutation
was possible and cleanup records not-needed. Restoration failure is separately
retained and fails the final aggregate without replacing health/test failures.
Cleanup is best-effort: VM loss, cancellation or the overall job deadline can
prevent `always()` from finishing. Session/logout scope, not a cleanup guarantee,
is the last boundary. Never use personal hardware for this workflow.

Setup cap5min: provenance100s, two helper compiles60s each, initial inventory5s,
transaction10s, settling10s (or failure inventory5s). Cleanup cap1min; no recompiles,
transaction retry or fallback. Overall job110min and cumulative70min stay fixed.
The separate `display-inventory` entrypoint still fails on a missing Anka receipt;
it is not called by session setup and no receipt is a CoreGraphics dependency.
No interpretation of an earlier inventory result is changed.

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

Setup, preflight, cumulative and restoration each bind `RUNNER_ENVIRONMENT` to
`${{ runner.environment }}` in step-level `env`, where GitHub permits that context.
There is no job-level binding or hard-coded observed identity. These bindings
preserve the hosted checks in provenance, the session helpers and the fresh
pre-test identity check. Validate expression-context availability with pinned
actionlint v1.7.12 as well as workflow structure; neither proves native execution.

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
normally nonzero. No further sizing occurs during controls or cumulative tests;
defaults, test widths and assertions do not change. Recheck fresh mode/scale/usable
points immediately before starting the fixture/tests, after both builds.
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
