# Inactive, never-merge AX/sidebar diagnostic

Preparation branch: `apple-macos26-ax-sidebar-prepared`. Only a future push to
`apple-macos26-ax-sidebar-run` could trigger this workflow; creating or moving
that ref requires a separate execution decision. No dispatch or scheduled trigger.

Product checkout is exactly `5910cfa98ee038fe53204b333c7065879f29a9ba`, tree
`8793de8321c788ce41587b97104971f805433000`. It is not merged with this harness.
The harness adds only this directory and its diagnostic workflow to that product
commit. No original workflow or application file changes.

## Environment and independence

One GitHub-hosted `macos-26-intel` VM; fail closed unless architecture `x86_64`,
image `20260824.0517.1`, macOS `26.6.1`/`25G76`, and Xcode `26.6`/`17F113` at
`/Applications/Xcode_26.6.app/Contents/Developer` match. Attempt must be 1.
These are the retained run42455029308 environment pins, not macOS15 acceptance.

After provenance succeeds, run two independent bounded checks. Failure of the AX
check does not gate the sidebar check. Either failure, missing result, export
failure, or attempted diagnostic capture failure fails the final job. Both
reports explicitly set `productAcceptance: false`. No third test or Caper UI
suite is selected. Standalone success cannot authorize any additional work.

## Standalone check

Build a separate `AXMenuDiagnostic` project from two Swift files and a minimal
XcodeGen spec. XcodeGen is pinned to
`21ac9944b0ab546a07422dbed86f33dd2ebd76f8`, the existing project tool revision.
The application imports only AppKit; the UI test imports Foundation/XCTest.
There are no package or Caper/SwiftUI/WebRTC dependencies. Retain executable and
debug-dylib `otool -L` output and fail on direct product/SwiftUI/WebRTC linkage.

Select only `AXMenuTests/AXMenuTests/testInitialTextFieldAndFullHierarchy`.
It launches once, waits for the text field with the existing 15-second query
allowance, then captures the complete unfiltered `app.debugDescription` as a
keepAlways attachment. It does not click, type, select menus, change first
responder or adjust focus/defaults/permissions. App startup uses the normal
`makeKeyAndOrderFront`/activation sequence of the pinned host, not an automation
focus workaround. Ordinary App/File/Edit menus retain responder actions;
product-specific Settings/What's-new/Bold/Italic callbacks are inert local
callbacks. No menu is hidden, removed, or specially skipped by accessibility.
This is a minimal discriminator, not a replica of Caper's model callbacks.

Begin/end query markers include the launched app's PID and elapsed time. A
temporary PID file passed via launchEnvironment avoids guessing a process by
name; the test removes it on exit. If the query is still in flight when its
marker is observed, concurrently attempt one 5-second sample of that PID and
one of launchd. Each sampling command has a 15-second process bound. Launchd
sampling uses `sudo -n sample` read-only; no service restart, prompt or permission
grant. A sample permission error/nonzero/timeout/missing file fails the diagnostic.
A query that finishes before observation has an explicit `not_attempted` sample
status; it is not reported as sampled. Sampling may perturb timing. There is no
artificial delay to extend the app lifetime and no retry to obtain evidence.

## Sidebar check and sole source overlay

Build only the existing Debug `CaperAppKit` scheme for testing, not
`CaperAppKitTests` or a shipping/Release/iOS application. The existing preparation
script still prepares dependencies, fonts, and all five shared Rust slices;
its generated shipping project is not built. The scheme also compiles its normal
core/view test bundles, but the only test selector is:

`CaperAppKitViewTests/NavigationSidebarTests/testVoiceCardPreservesFocusAndLiveRowWhenChannelsCollapse`

No local API fixture server is started for this in-process view test. Existing
`prepare.sh` is unchanged. Its invocation exports a process-local Bash curl
wrapper appending `--retry 0` and `CARGO_NET_RETRY=0`, so its explicit dependency
download retries are disabled without editing source or application defaults.
There is no workflow, native test, sampling-command or build retry.
The RNNoise source/model are restored from the same prior SHA256-pinned inputs;
no diagnostic workflow from that old ref is adopted.

`sidebar-geometry.patch` adds 38 lines and removes none: one local serializer and
two calls immediately before the original wide/narrow count assertions. It
records the complete NSView tree (class/identifier/label/frame/bounds/visibility),
scroll content size, clip/document bounds, actual/preferred scroller style,
vertical scroller frame/visibility, border, insets and persisted split share.
The tree contains the private card body and face/overflow tile frames without
changing their access level or invoking an extra layout pass. No scroller style,
width, focus, timeout, fixture, expected count, or original statement changes.

Original test SHA256: `dfb837a6075b6a38bb0855a9883eec16b0699cb50bcceaf0b809b14ffbfb245d`.
Observed test SHA256: `87f647234a0a959b8b9b08e5ce26e7c8ffe9011b604c1651c7fb688be087904b`.
Patch SHA256: `2ca7be97ba1bdf747bcdaf26098840c5500102f31d80b485bfa1f8d2fad5e172`.

## Bounds and retained evidence

Native test flags retain `-parallel-testing-enabled NO`, test timeouts enabled,
default allowance300, maximum600, and diagnostics `on-failure`. Each xcodebuild
test subprocess has the previous900-second external cap, with real exit/signal
and timeout recorded; no increased allowance. Workflow step limits further cap
standalone25min/sidebar70min and the whole job110min. A step cap is a failure,
not a skipped check. Each xcresult export has180 seconds. Keep original/generated
schemes, xctestrun files, binary hashes, build logs, raw test stdout/stderr,
xcresults, JSON summaries, exact starts/outcomes, AX/geometry attachments and
XCTest recordings through keepAlways/screenRecording scheme settings. A passing
recording still requires inspection; absent/failed attachments never waive a
failed test. Count gates require exactly the selected singleton once, one pass,
zero failures/skips/expected failures, and a zero process exit.

Before and after each independent check, a separate30-second process copies
available relevant crash reports from the user/system DiagnosticReports folders.
Names are limited to iconservices, launchd, AXMenu, Caper, Dock and Spotlight;
retain original paths, hashes and bytes. Bounds:64 reports,20MB/file,80MB per
capture. Exceeding bounds or a read/copy error fails rather than truncating
silently. An empty manifest means no available report, not no crash. Preserve
both outcomes and upload evidence even on failure; no renderer/service/default
workaround is performed.
