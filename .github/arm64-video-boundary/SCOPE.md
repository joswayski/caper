# Local activation candidate, never-merge same-file timestamp diagnostic

This is not a health-gate correction or Caper acceptance. `Pixels.swift`, the old
replay supervisor, both original media and all pre-existing workflows are unchanged.
`.github/workflows/apple-video-boundary.yml` selects only `apple-video-boundary-run`.
This local activation candidate is not authorization to publish or execute it;
both actions require separate exact-pin approval. The workflow checks out the event
SHA and the supervisor requires a clean matching HEAD, attempt 1, GitHub-hosted
ARM64 image `20261102.0473.1`, macOS `26.7.2/25G332`, Xcode `26.6/17F113` at
`/Applications/Xcode_26.6.app/Contents/Developer`. An authorization review must pin
the eventual activation head externally; matching GITHUB_SHA alone is not approval.

Only the existing `.github/arm64-health/video-replay/failed.mp4` is input: 51,285
bytes, SHA256 `449df2b898b9b5b907a9989aeae75bf002035c69f18456912dd47a1faf8cd42c`.
It is the original ZIP media from run 42496005325 and the unchanged replay input
from run 42501352093. No download, new capture, remux, edited media or other file.

Compile `Boundary.swift` once (180s); invoke the binary in three independent fresh
processes, once each (60s each), at integer CMTime values 369, 370, 1720 with
timescale 600. No 339 request, no image-request loop, retries, alternate decoder,
tolerance change or fallback. The outer step/job caps are 8/10 minutes; identity
commands add up to 110s of individual ceilings, so these caps are not guaranteed
reserved execution time. Cancellation can leave later cases unexecuted and evidence
incomplete. A case error does not prevent the other distinct cases; an identity,
compile or media-integrity failure prevents further cases. Never rerun a case.

The original retained ffprobe stream has time_base `1/600`, start_pts 370 and
integer frame pts `[370,440,1720,2251,2280,2340,2370,2380,2510,3090,3110,3130,3150,3190,3250,3330]`.
The original MP4 movie timescale is 600; edit list first entry is duration 370,
media_time -1; second is duration 3020, media_time 1811. Original unedited first
and third frame PTS are 1811 and 3161; subtract 1811 and add 370 to recover 370
and 1720. Thus 369 is one tick before the empty edit ends; 370 and 1720 are exact
retained presentation ticks, not rounded decimal estimates. This metadata is not
a promise about what AVFoundation returns, and no result is prescribed.

As in the original helper, construct `AVURLAsset(url:)`, load duration, construct
`AVAssetImageGenerator(asset:)`, set `appliesPreferredTrackTransform = true`, and
set both requested time tolerances to `.zero`; leave other generator options at
their defaults. Apple's [image(at:) contract](https://developer.apple.com/documentation/avfoundation/avassetimagegenerator/image(at:))
uses the asset timeline and returns an image plus actual time. Apple's
[tolerance documentation](https://developer.apple.com/documentation/avfoundation/avassetimagegenerator/requestedtimetolerancebefore)
describes zero tolerances as frame-accurate requests that may add decoding delay.
Record actual CMTime without requiring equality to requested CMTime.

The new helper makes no OCR/content judgment. Preserve requested/actual CMTime
(value, timescale, epoch, flags, seconds), duration, PID, monotonic sequence,
phase/error and returned image dimensions; save the returned image as PNG. A PNG
failure after image return keeps `imageGenerationSucceeded:true` and independently
records output failure. Every error exits 1. The aggregate remains failed for any
nonzero, timeout, signal or incomplete evidence; there are no expected-negative
exceptions to turn the job green. `contentAcceptance` is `not-evaluated` and
`productAcceptance` stays false, including if all three images are generated.

Bounded diagnostic output: at most ten structured events per case, one PNG per
case, one final JSON, raw stdout/stderr and process result. Error traversal retains
up to 16 underlying NSError entries (domain/code/description/failureReason only),
128/512-character field bounds, cycle/depth/non-NSError/truncation markers. Such
markers mean incomplete error evidence, not a complete chain. No arbitrary
userInfo, file sweep or other-app data. Raw framework stderr is retained without
filtering; time caps bound process lifetime, not an absolute byte quota.

The supervisor reuses only `command`, `ok`, `record`, `sha` from the unchanged
health module; importing it does not invoke its main or any Caper/health/session
function. Its process-group watchdog/exit/signal/timeout recording is unchanged.
Copy only manifest-listed source/dependencies/media, retain their hashes and the
compiled binary/hash; record the exact environment/run/head/attempt. Preserve
incremental error streams even if final JSON cannot be collected. Upload on failure
when the runner permits it. No GUI session setup, AX, screen capture, tests, product
builds, hardware, satellite or Captures work.

Logging, duration loading, hashing and process startup add overhead. Independent
processes reset generator state but still share the hosted VM and OS caches.
Neither these changes nor fixed timestamps guarantee timing-equivalence with the
historical run. Same-file before-boundary/boundary/later outcomes can distinguish
empty-range behavior from broader decode failure; they do not retrospectively
prove the old call or clear historical health/forwarding failures or 42/43.
