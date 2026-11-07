import Foundation
import XCTest
@testable import CaperCore
#if os(macOS)
import AppKit
#endif

/// Voice messages with transcripts (the shared voice-message contract).
final class VoiceMessageTests: XCTestCase {
    // MARK: Waveform

    func testWaveformUsesSliceRMSOnAPerceptualScaleSmoothedAndNormalised() {
        // 0.1 RMS is -20 dB: 0.6 on the [-50, 0] dB scale; full scale is 1.
        var rms = [Float](repeating: 0.1, count: 64)
        rms[63] = 1
        let waveform = VoiceWaveform.build(rms: rms)
        XCTAssertEqual(waveform.count, 64)
        XCTAssertEqual(waveform[63], 255, "the loudest (smoothed) slice is 255")
        XCTAssertEqual(waveform[62], 234, "3-tap average of 0.6, 0.6 and 1 against the loudest 0.8")
        XCTAssertEqual(waveform[0], 191, "0.6 / 0.8 × 255")
        XCTAssertEqual(Set(waveform[0..<61]), [191], "steady speech is one level, not jagged")
        // Each slice is the RMS of its readings, not their peak.
        let pairs = (0..<64).flatMap { _ in [Float(0.3), Float(0.4)] }
        XCTAssertEqual(Set(VoiceWaveform.build(rms: pairs)), [255])
        XCTAssertEqual(VoiceWaveform.level(rms: (0.125 as Float).squareRoot()), VoiceWaveform.level(rms: 0.35355339), accuracy: 0.0001)
    }

    func testWaveformScaleSmoothingAndSilence() {
        XCTAssertEqual(VoiceWaveform.level(rms: 1), 1)
        XCTAssertEqual(VoiceWaveform.level(rms: 0.1), 0.6, accuracy: 0.0001)
        XCTAssertEqual(VoiceWaveform.level(rms: 0.001), 0, "-60 dB is below the floor")
        XCTAssertEqual(VoiceWaveform.level(rms: 0), 0)
        XCTAssertEqual(VoiceWaveform.level(rms: .nan), 0)
        XCTAssertEqual(VoiceWaveform.normalised([0, 0, 1, 0, 0]), [0, 255, 255, 255, 0], "a spike spreads over its neighbours")
        XCTAssertEqual(VoiceWaveform.normalised([0.5, 1]), [255, 255], "the ends average what they have")
        XCTAssertEqual(VoiceWaveform.normalised([]), [])
        XCTAssertEqual(VoiceWaveform.build(rms: []), Array(repeating: 0, count: 64))
        XCTAssertEqual(VoiceWaveform.build(rms: [0, 0, 0]), Array(repeating: 0, count: 64), "silence stays flat")
        XCTAssertEqual(VoiceWaveform.build(rms: [.nan, -1, .infinity, 0.5]).max(), 255, "invalid readings count as silence")
        XCTAssertEqual(VoiceWaveform.build(rms: [1, 2], slices: 0), [])
        XCTAssertEqual(VoiceWaveform.build(rms: [0.2, 0.2]).count, 64, "short recordings stretch over every slice")
    }

    func testRMSAccumulatorSplitsTheRecordingIntoEqualSlices() {
        var accumulator = VoiceWaveform.RMSAccumulator(totalFrames: 8, slices: 2)
        accumulator.add([1, -1, 1], start: 0)
        accumulator.add([-1, 0, 0, 0.5, -0.5], start: 3)
        XCTAssertEqual(accumulator.rms[0], 1)
        XCTAssertEqual(accumulator.rms[1], 0.3535534, accuracy: 0.0001)
        XCTAssertEqual(VoiceWaveform.RMSAccumulator(totalFrames: 10).rms, Array(repeating: 0, count: 64))
    }

    func testWaveformEncodesAsPaddedStandardBase64AndDecodesOnlyValidLengths() {
        let bytes = (0..<64).map { UInt8($0 * 4) }
        let encoded = VoiceWaveform.encode(bytes)
        XCTAssertEqual(encoded.count, 88, "64 bytes are 88 base64 characters with padding")
        XCTAssertTrue(encoded.hasSuffix("=="))
        XCTAssertEqual(VoiceWaveform.decode(encoded), bytes)
        XCTAssertEqual(VoiceWaveform.encode([255, 0, 128]), "/wCA")
        XCTAssertNil(VoiceWaveform.decode(nil))
        XCTAssertNil(VoiceWaveform.decode(""), "empty is absent")
        XCTAssertNil(VoiceWaveform.decode("not base64!"))
        XCTAssertNil(VoiceWaveform.decode(Data(count: 257).base64EncodedString()), "over 256 bytes is invalid")
        XCTAssertEqual(VoiceWaveform.decode(Data(count: 256).base64EncodedString())?.count, 256)
    }

    func testBarsResampleWithLinearInterpolationAndDrawAFlatLine() {
        XCTAssertEqual(VoiceWaveform.resample([0, 255], count: 3), [0, 0.5, 1])
        XCTAssertEqual(VoiceWaveform.resample([0, 255, 0], count: 5), [0, 0.5, 1, 0.5, 0])
        let down = VoiceWaveform.resample((0..<64).map { UInt8($0 * 4) }, count: 22)
        XCTAssertEqual(down.count, 22)
        XCTAssertEqual(down.first, 0, "the first bar is the first value")
        XCTAssertEqual(down.last ?? 0, 252.0 / 255, accuracy: 0.0001, "the last bar is the last value")
        XCTAssertEqual(zip(down, down.dropFirst()).allSatisfy { $0 < $1 }, true, "a ramp stays a smooth ramp")
        XCTAssertEqual(VoiceWaveform.resample(nil, count: 3), [0, 0, 0], "no waveform draws a flat line")
        XCTAssertEqual(VoiceWaveform.resample([], count: 2), [0, 0])
        XCTAssertEqual(VoiceWaveform.resample([255], count: 4), [1, 1, 1, 1])
        XCTAssertEqual(VoiceWaveform.resample([51, 255], count: 1), [1])
        XCTAssertEqual(VoiceWaveform.resample([1, 2], count: 0), [])
    }

    func testMetersAndLiveBars() {
        XCTAssertEqual(VoiceWaveform.amplitude(decibels: 0), 1)
        XCTAssertEqual(VoiceWaveform.amplitude(decibels: -20), 0.1, accuracy: 0.0001)
        XCTAssertEqual(VoiceWaveform.amplitude(decibels: -160), 0, accuracy: 0.0001)
        XCTAssertEqual(VoiceWaveform.amplitude(decibels: .nan), 0)
        XCTAssertEqual(VoiceWaveform.amplitude(decibels: 6), 1, "clipped readings stay at full scale")
        XCTAssertEqual(VoiceWaveform.meterLevel(decibels: 0), 1)
        XCTAssertEqual(VoiceWaveform.meterLevel(decibels: -25), 0.5)
        XCTAssertEqual(VoiceWaveform.meterLevel(decibels: -80), 0)
        XCTAssertEqual(VoiceWaveform.scrollFraction(sinceLastBar: 0.0375), 0.5, accuracy: 0.0001)
        XCTAssertEqual(VoiceWaveform.scrollFraction(sinceLastBar: 0.09), 1.2, accuracy: 0.0001, "keeps sliding while a bar is late")
        XCTAssertEqual(VoiceWaveform.scrollFraction(sinceLastBar: 1), 3)
        XCTAssertEqual(VoiceWaveform.scrollFraction(sinceLastBar: -1), 0)
        XCTAssertLessThan(VoiceMessage.meterInterval, VoiceMessage.liveBarInterval, "every live bar gets a reading")
        XCTAssertEqual(VoiceWaveform.barCount(available: 100, width: 3, gap: 2), 20)
        XCTAssertEqual(VoiceWaveform.barCount(available: 0, width: 3, gap: 2), 1)
    }

    func testLiveMeterBarsTakeEachWindowsPeakAndFallFast() {
        let start = Date(timeIntervalSinceReferenceDate: 0)
        func at(_ ms: Double) -> Date { start.addingTimeInterval(ms / 1000) }
        var meter = VoiceLiveMeter()
        XCTAssertFalse(meter.add(0.9, at: at(0)), "the first reading starts the clock")
        XCTAssertEqual(meter.pending, 0.9, "the forming bar shows the first reading at once")
        XCTAssertEqual(meter.lastBarAt, at(0))
        XCTAssertFalse(meter.add(0.3, at: at(60)), "a bar is due 75 ms in")
        XCTAssertTrue(meter.add(0.5, at: at(80)))
        XCTAssertEqual(meter.bars, [0.9], "the loudest reading in the window")
        XCTAssertEqual(meter.pending, 0.9 * 0.35, accuracy: 0.0001, "the next bar starts from the fall")
        XCTAssertEqual(meter.lastBarAt, at(75), "a constant cadence, not the reading's time")
        XCTAssertTrue(meter.add(0, at: at(150)))
        XCTAssertEqual(meter.bars[1], 0.9 * 0.35, accuracy: 0.0001, "silence falls to 35% of the previous bar")
        XCTAssertTrue(meter.add(.nan, at: at(225)))
        XCTAssertEqual(meter.bars[2], 0.9 * 0.35 * 0.35, accuracy: 0.0001, "an unreadable level is silence")
        XCTAssertFalse(meter.add(0.6, at: at(270)))
        XCTAssertEqual(meter.pending, 0.6, accuracy: 0.0001, "a louder reading shows before its bar is due")
        XCTAssertTrue(meter.add(0.2, at: at(305)))
        XCTAssertEqual(meter.bars[3], 0.6, accuracy: 0.0001, "a louder reading wins over the fall")
        XCTAssertEqual(meter.lastBarAt?.timeIntervalSince(start) ?? 0, 0.3, accuracy: 1e-9)

        // A stall adds one bar, then restarts the cadence from now instead of catching up.
        XCTAssertTrue(meter.add(0.1, at: at(900)))
        XCTAssertEqual(meter.bars.count, 5)
        XCTAssertEqual(meter.bars[4], 0.21, accuracy: 0.0001)
        XCTAssertEqual(meter.lastBarAt, at(900))
        XCTAssertFalse(meter.add(1, at: at(930)))

        var long = VoiceLiveMeter()
        for index in 0..<300 { long.add(0.5, at: at(Double(index) * 80)) }
        XCTAssertEqual(long.bars.count, VoiceLiveMeter.historyLimit)
    }

    func testLiveMeterShapeAndEnvelope() {
        XCTAssertEqual(VoiceLiveMeter.shape(0), 0)
        XCTAssertEqual(VoiceLiveMeter.shape(0.28), 0, "about -36 dBFS and quieter sits at the minimum")
        XCTAssertEqual(VoiceLiveMeter.shape(0.75), 1, accuracy: 0.0001, "about -12 dBFS fills the meter")
        XCTAssertEqual(VoiceLiveMeter.shape(1), 1)
        XCTAssertEqual(VoiceLiveMeter.shape(0.515), 0.43528, accuracy: 0.0001)
        XCTAssertEqual(VoiceLiveMeter.shape(.nan), 0)
        XCTAssertEqual(VoiceLiveMeter.smoothstep(0, 10, 5), 0.5)
        XCTAssertEqual(VoiceLiveMeter.smoothstep(0, 0, 1), 1)

        XCTAssertEqual(VoiceLiveMeter.envelope(center: 100, width: 200), .init(scale: 1, alpha: 1))
        XCTAssertEqual(VoiceLiveMeter.envelope(center: 198.5, width: 200), .init(scale: 1, alpha: 1),
                       "full size and visible right at the edge, so speech shows at once")
        let pinched = VoiceLiveMeter.envelope(center: 20, width: 200)
        XCTAssertEqual(pinched.scale, 0.65, accuracy: 0.0001, "halfway into the 40-point pinch")
        XCTAssertEqual(pinched.alpha, 1)
        let left = VoiceLiveMeter.envelope(center: 0, width: 200)
        XCTAssertEqual(left.scale, 0.3, accuracy: 0.0001)
        XCTAssertEqual(left.alpha, 0)
        XCTAssertEqual(VoiceLiveMeter.envelope(center: 16, width: 40).scale, 0.65, accuracy: 0.0001,
                       "a narrow meter pinches over 80% of its width")
    }

    func testLiveMeterLayout() {
        XCTAssertEqual(VoiceLiveMeter.layout([1], pending: 1, width: 0, height: 22, scroll: 0), [])
        let pills = VoiceLiveMeter.layout([0, 1], pending: 1, width: 60, height: 22, scroll: 0)
        XCTAssertEqual(pills.count, 3, "the forming bar, then the newest and older bars")
        XCTAssertEqual(pills[0].x, 57, "the forming bar sits at the right edge")
        XCTAssertEqual(pills[0].width, 3)
        XCTAssertEqual(pills[0].height, 22 * 0.94, accuracy: 0.0001, "at full size")
        XCTAssertEqual(pills[0].y, (22 - pills[0].height) / 2, accuracy: 0.0001)
        XCTAssertEqual(pills[0].brightness, 1, "loud bars are eggshell")
        XCTAssertEqual(pills[0].alpha, 1)
        XCTAssertEqual(pills[1].x, 51)
        XCTAssertEqual(pills[1].brightness, 1)
        XCTAssertEqual(pills[2].x, 45)
        XCTAssertEqual(pills[2].brightness, 0.25)
        let scrolled = VoiceLiveMeter.layout([0, 1], pending: 1, width: 60, height: 22, scroll: 0.5)
        XCTAssertEqual(scrolled.map(\.x), [54, 48, 42], "everything slides left between bars")
        XCTAssertEqual(VoiceLiveMeter.layout(Array(repeating: 0.5, count: 200), pending: 0.5, width: 60, height: 22, scroll: 0).count, 10,
                       "only bars on screen are drawn")
        let dot = VoiceLiveMeter.layout(Array(repeating: 0, count: 20), pending: 0, width: 200, height: 22, scroll: 0)[10]
        XCTAssertEqual(dot, .init(x: 137, y: 9.5, width: 3, height: 3, brightness: 0.25, alpha: 1), "silence is a small dot")
        let quiet = VoiceLiveMeter.color(brightness: 0)
        XCTAssertEqual(quiet.red * 255, 150, accuracy: 0.001)
        XCTAssertEqual(quiet.blue * 255, 158, accuracy: 0.001)
        let loud = VoiceLiveMeter.color(brightness: 1)
        XCTAssertEqual(loud.red * 255, 241, accuracy: 0.001)
        XCTAssertEqual(loud.green * 255, 237, accuracy: 0.001)
    }

    // MARK: Durations and recording rules

    func testDurationLabels() {
        XCTAssertEqual(VoiceDuration.format(ms: 0), "0:00")
        XCTAssertEqual(VoiceDuration.format(ms: 4_200), "0:04")
        XCTAssertEqual(VoiceDuration.format(ms: 59_999), "0:59")
        XCTAssertEqual(VoiceDuration.format(ms: 61_000), "1:01")
        XCTAssertEqual(VoiceDuration.format(ms: 600_000), "10:00")
        XCTAssertEqual(VoiceDuration.format(ms: -5), "0:00")
        XCTAssertEqual(VoiceDuration.remaining(durationMs: 4_200, elapsedMs: 0), "0:05", "time left rounds up")
        XCTAssertEqual(VoiceDuration.remaining(durationMs: 4_200, elapsedMs: 4_200), "0:00")
        XCTAssertEqual(VoiceDuration.playerLabel(durationMs: 4_200, elapsedMs: nil), "0:04", "idle shows the total")
        XCTAssertEqual(VoiceDuration.playerLabel(durationMs: 4_200, elapsedMs: 1_500), "0:03", "playing shows the time left")
        XCTAssertEqual(VoiceDuration.playerLabel(durationMs: nil, elapsedMs: nil), "0:00")
        XCTAssertEqual(VoiceDuration.playerLabel(durationMs: nil, elapsedMs: 2_000), "0:02")
    }

    func testSendabilityAndProfileRetryRules() {
        XCTAssertTrue(VoiceMessage.sendable(durationMs: 1_000, byteSize: 1))
        XCTAssertTrue(VoiceMessage.sendable(durationMs: 600_000, byteSize: 9_000))
        XCTAssertFalse(VoiceMessage.sendable(durationMs: 999, byteSize: 9_000), "under a second never uploads")
        XCTAssertFalse(VoiceMessage.sendable(durationMs: 600_001, byteSize: 9_000), "over the server's limit")
        XCTAssertFalse(VoiceMessage.sendable(durationMs: 4_000, byteSize: 0), "an empty file never uploads")
        // Failures before the POST (destination, session, upload) always keep the recording.
        XCTAssertTrue(VoiceMessage.keepsForRetry(attemptedSend: false, status: 403))
        XCTAssertTrue(VoiceMessage.keepsForRetry(attemptedSend: false, status: nil))
        // After the POST: unknown outcomes keep it (Retry resends the same id); refusals drop it.
        XCTAssertTrue(VoiceMessage.keepsForRetry(attemptedSend: true, status: nil), "a network error is an unknown outcome")
        XCTAssertTrue(VoiceMessage.keepsForRetry(attemptedSend: true, status: 503))
        XCTAssertTrue(VoiceMessage.keepsForRetry(attemptedSend: true, status: 502))
        for status in [400, 401, 403, 404, 409, 413, 422] {
            XCTAssertFalse(VoiceMessage.keepsForRetry(attemptedSend: true, status: status), "\(status) is a definitive refusal")
        }
        XCTAssertEqual(VoiceMessage.retryNotice(unconfirmed: false), "Voice message didn’t send.")
        XCTAssertEqual(VoiceMessage.retryNotice(unconfirmed: true), "Voice message delivery isn’t confirmed. Retry resends the same message.")
    }

    func testSendOnStopSettingDefaultsOffAndStagesByDefault() {
        XCTAssertFalse(VoiceMessage.sendsOnStop(stored: nil), "off until chosen")
        XCTAssertFalse(VoiceMessage.sendsOnStop(stored: false))
        XCTAssertTrue(VoiceMessage.sendsOnStop(stored: true))
        XCTAssertFalse(VoiceMessage.sendsOnStop(stored: "yes"), "anything else is off")
        XCTAssertEqual(VoiceMessage.finish(sendsOnStop: false), .stage)
        XCTAssertEqual(VoiceMessage.finish(sendsOnStop: true), .send)
        XCTAssertEqual(VoiceMessage.releaseHint(sendsOnStop: true), "Release to send · Slide up to lock")
        XCTAssertEqual(VoiceMessage.releaseHint(sendsOnStop: false), "Release to add · Slide up to lock")
        XCTAssertTrue(VoiceMessage.canStage(staged: 0, voiceStaged: false))
        XCTAssertTrue(VoiceMessage.canStage(staged: 9, voiceStaged: false), "beside staged photos and files")
        XCTAssertFalse(VoiceMessage.canStage(staged: 1, voiceStaged: true), "one voice note per message")
        XCTAssertFalse(VoiceMessage.canStage(staged: 10, voiceStaged: false), "within the attachment limit")
        XCTAssertEqual(VoiceMessage.oneVoicePerMessage, "Remove the voice message to record another")
        XCTAssertEqual(VoiceMessage.sendOnStopTitle, "Send voice messages as soon as I stop recording")
    }

    func testRecordingRules() {
        XCTAssertFalse(VoiceMessage.keeps(durationMs: 999), "under a second is discarded")
        XCTAssertTrue(VoiceMessage.keeps(durationMs: 1_000))
        XCTAssertEqual(VoiceMessage.uploadDuration(measuredMs: 4_321, elapsedMs: 4_400), 4_321, "the measured file wins")
        XCTAssertEqual(VoiceMessage.uploadDuration(measuredMs: nil, elapsedMs: 4_400), 4_400)
        XCTAssertEqual(VoiceMessage.uploadDuration(measuredMs: 0, elapsedMs: 4_400), 4_400)
        XCTAssertEqual(VoiceMessage.uploadDuration(measuredMs: 600_040, elapsedMs: 600_000), 600_000, "never above the server's limit")
        XCTAssertEqual(VoiceMessage.release(pressDuration: 0.1, recordedMs: 0, locked: false), .handsFree, "a quick tap records until Stop")
        XCTAssertEqual(VoiceMessage.release(pressDuration: 0.1, recordedMs: 80, locked: false), .handsFree)
        XCTAssertEqual(VoiceMessage.release(pressDuration: 0.8, recordedMs: 700, locked: false), .discard, "a hold under a second is discarded")
        XCTAssertEqual(VoiceMessage.release(pressDuration: 2, recordedMs: 1_900, locked: false), .stop)
        XCTAssertEqual(VoiceMessage.release(pressDuration: 2, recordedMs: 1_900, locked: true), .handsFree, "a locked hold keeps recording")
        XCTAssertFalse(VoiceMessage.locksBySliding(translation: -40))
        XCTAssertTrue(VoiceMessage.locksBySliding(translation: -VoiceMessage.lockDistance))
        XCTAssertFalse(VoiceMessage.locksBySliding(translation: 200), "sliding down never locks")
        XCTAssertEqual(VoiceMessage.lockProgress(translation: -VoiceMessage.lockDistance / 2), 0.5)
        XCTAssertEqual(VoiceMessage.lockProgress(translation: 30), 0)
    }

    func testRecordingsUseTheCallsSavedVoiceEnhancementStrength() {
        XCTAssertEqual(VoiceMessage.processingStrengthKey, "caper.voice.processingStrength", "the call's own setting")
        XCTAssertEqual(VoiceMessage.processingStrength(stored: nil), 25, "25% until the person changes it")
        XCTAssertEqual(VoiceMessage.processingStrength(stored: 0), 0, "0% is a choice, not absent")
        XCTAssertEqual(VoiceMessage.processingStrength(stored: 60), 60)
        XCTAssertEqual(VoiceMessage.processingStrength(stored: 150), 100)
        XCTAssertEqual(VoiceMessage.processingStrength(stored: -5), 0)
        XCTAssertEqual(VoiceMessage.processingStrength(stored: "40"), 40, "a launch argument's text")
        XCTAssertEqual(VoiceMessage.processingStrength(stored: "loud"), 25)
        XCTAssertEqual(VoiceMessage.processingStrength(stored: true), 25)
    }

    func testRecordingsTakeTheCallsProcessingAndFallBackToThePlainRecorder() {
        XCTAssertEqual(VoiceMessage.capture(iOS: true, sharesCallSession: false), .processed(voiceProcessing: true),
                       "iPhone outside a call: the call's voice-processing I/O (echo cancellation, gain control)")
        XCTAssertEqual(VoiceMessage.capture(iOS: true, sharesCallSession: true), .processed(voiceProcessing: false),
                       "beside a call: its session and voice processing stay the call's")
        XCTAssertEqual(VoiceMessage.capture(iOS: false, sharesCallSession: false), .processed(voiceProcessing: false),
                       "Mac calls capture without Apple's voice processing")
        XCTAssertEqual(VoiceMessage.capture(iOS: true, sharesCallSession: false, processedFailed: true), .plain,
                       "recording never breaks when the processed chain can't start")
        XCTAssertEqual(VoiceMessage.capture(iOS: false, sharesCallSession: false, processedFailed: true), .plain)
        XCTAssertEqual(VoiceMessage.maximumFrames, 28_800_000, "10:00 of the 48 kHz file")
    }

    func testPendingTranscriptBreathesOnceEveryTwoSeconds() {
        XCTAssertEqual(VoiceTranscript.pendingLabel, "Transcribing…")
        XCTAssertEqual(VoiceTranscript.pendingBreathOpacity, 0.4)
        XCTAssertEqual(VoiceTranscript.pendingBreathSeconds, 2)
    }

    func testPlaybackPositions() {
        XCTAssertEqual(VoicePlaybackPosition.fraction(x: 50, width: 200), 0.25)
        XCTAssertEqual(VoicePlaybackPosition.fraction(x: -10, width: 200), 0)
        XCTAssertEqual(VoicePlaybackPosition.fraction(x: 500, width: 200), 1)
        XCTAssertEqual(VoicePlaybackPosition.fraction(x: 10, width: 0), 0)
        XCTAssertEqual(VoicePlaybackPosition.progress(elapsedMs: 1_000, durationMs: 4_000), 0.25)
        XCTAssertEqual(VoicePlaybackPosition.progress(elapsedMs: 9_000, durationMs: 4_000), 1)
        XCTAssertEqual(VoicePlaybackPosition.progress(elapsedMs: 1_000, durationMs: nil), 0)
        XCTAssertEqual(VoicePlaybackPosition.playedFill(progress: 0, bar: 0, count: 40), 0)
        XCTAssertEqual(VoicePlaybackPosition.playedFill(progress: 0.5, bar: 19, count: 40), 1)
        XCTAssertEqual(VoicePlaybackPosition.playedFill(progress: 0.5, bar: 20, count: 40), 0)
        XCTAssertEqual(VoicePlaybackPosition.playedFill(progress: 0.5125, bar: 20, count: 40), 0.5, accuracy: 0.0001,
                       "the boundary bar fills partly")
        XCTAssertEqual(VoicePlaybackPosition.playedFill(progress: 1, bar: 39, count: 40), 1)
        XCTAssertEqual(VoicePlaybackPosition.playedFill(progress: .nan, bar: 0, count: 40), 0)
    }

    // MARK: Transcripts

    func testTranscriptStatesDecodeAndUnknownOnesAreAbsent() throws {
        func decode(_ json: String) throws -> ChatAttachment {
            try JSONDecoder().decode(ChatAttachment.self, from: Data(json.utf8))
        }
        let base = #""id":"Asset0000001","kind":"audio","contentType":"audio/mp4","name":"Voice message.m4a","size":31337,"durationMs":4200,"url":"https://cdn.caper.chat/original/Asset0000001?exp=1&sig=o""#
        let plain = try decode("{\(base)}")
        XCTAssertFalse(plain.voice, "absent voice is a regular audio file")
        XCTAssertNil(plain.waveform)
        XCTAssertNil(plain.transcript)

        let pending = try decode(#"{\#(base),"voice":true,"waveform":"/wCA","transcript":{"status":"pending"}}"#)
        XCTAssertTrue(pending.voice)
        XCTAssertEqual(pending.waveformBytes, [255, 0, 128])
        XCTAssertEqual(pending.transcript, .pending)
        XCTAssertEqual(pending.transcript?.display, .pending)

        let ready = try decode(#"{\#(base),"voice":true,"transcript":{"status":"ready","text":"  Yo what's going on  "}}"#)
        XCTAssertEqual(ready.transcript, .ready("  Yo what's going on  "))
        XCTAssertEqual(ready.transcript?.display, .text("Yo what's going on"))
        XCTAssertEqual(try decode(#"{\#(base),"voice":true,"transcript":{"status":"ready","text":" \n "}}"#).transcript?.display, .hidden,
                       "empty text shows nothing")
        XCTAssertEqual(try decode(#"{\#(base),"voice":true,"transcript":{"status":"ready"}}"#).transcript?.display, .hidden)
        XCTAssertEqual(try decode(#"{\#(base),"voice":true,"transcript":{"status":"failed"}}"#).transcript?.display, .hidden)
        XCTAssertNil(try decode(#"{\#(base),"voice":true,"transcript":{"status":"translating"}}"#).transcript, "unknown status is absent")
        XCTAssertNil(try decode(#"{\#(base),"voice":true,"transcript":"ready"}"#).transcript)
        XCTAssertNil(try decode(#"{\#(base),"voice":true,"transcript":{"status":7}}"#).transcript)

        // Malformed voice fields never drop the file.
        let odd = try decode(#"{\#(base),"voice":"yes","waveform":42}"#)
        XCTAssertFalse(odd.voice)
        XCTAssertNil(odd.waveform)
        XCTAssertNil(try decode(#"{\#(base),"voice":true,"waveform":"%%%"}"#).waveformBytes, "an invalid waveform draws flat bars")

        // Round trip, as cached messages are re-encoded.
        for value in [pending, ready, plain] {
            XCTAssertEqual(try JSONDecoder().decode(ChatAttachment.self, from: JSONEncoder().encode(value)), value)
        }
        let failed = ChatAttachment(id: "A", kind: .audio, contentType: "audio/mp4", name: "Voice message.m4a", size: 1,
                                    voice: true, transcript: .failed)
        XCTAssertEqual(try JSONDecoder().decode(ChatAttachment.self, from: JSONEncoder().encode(failed)), failed)
    }

    func testAttachmentsEventKeepsVoiceFieldsAndReplacesAPendingTranscript() throws {
        let channelID = "Chan12345678"
        let author = ChatAuthor(id: "u", name: "U", isGuest: false)
        let pending = ChatAttachment(id: "Asset0000001", kind: .audio, contentType: "audio/mp4", name: "Voice message.m4a", size: 31337,
                                     durationMs: 4200, url: "https://cdn.caper.chat/original/Asset0000001?exp=1&sig=o",
                                     voice: true, waveform: "/wCA", transcript: .pending)
        let message = ChatMessage(id: "Message00000001", channelId: channelID, seq: "1", author: author,
                                  content: ChatContent(version: 1, type: "text", text: "", attachments: [pending]),
                                  createdAt: "now", clientMessageId: "c", attachmentsSeq: "1")
        let event = try JSONDecoder().decode(MessageAttachmentsEvent.self, from: Data(#"""
        {"type":"message.attachments","schemaVersion":1,"channelId":"Chan12345678","seq":"7","messageId":"Message00000001",
         "attachments":[{"id":"Asset0000001","kind":"audio","contentType":"audio/mp4","name":"Voice message.m4a","size":31337,
                         "durationMs":4200,"url":"https://cdn.caper.chat/original/Asset0000001?exp=2&sig=n","voice":true,
                         "waveform":"/wCA","transcript":{"status":"ready","text":"Yo what's going on"}}]}
        """#.utf8))
        XCTAssertTrue(event.isValid)
        var snapshots = AttachmentSnapshots()
        snapshots.seed([message])
        XCTAssertTrue(snapshots.apply(messageID: event.messageId, seq: event.seq, attachments: event.attachments))
        let updated = try XCTUnwrap(snapshots.overlay(message).content.attachments?.first)
        XCTAssertTrue(updated.voice)
        XCTAssertEqual(updated.waveformBytes, [255, 0, 128])
        XCTAssertEqual(updated.durationMs, 4200)
        XCTAssertEqual(updated.transcript?.display, .text("Yo what's going on"))
        XCTAssertNotEqual(updated, pending, "the visible message changes, so it re-renders")
        // A replayed older event can't bring "Transcribing…" back.
        XCTAssertFalse(snapshots.apply(messageID: event.messageId, seq: "6", attachments: [pending]))
        XCTAssertEqual(snapshots.overlay(message).content.attachments?.first?.transcript, .ready("Yo what's going on"))
    }

    // MARK: Upload and excerpts

    func testPreparedVoiceUploadsTheRecordingAsIsOrNothing() throws {
        let file = try AttachmentStaging.stage(data: Data(repeating: 3, count: 4_096), filename: "recording.m4a")
        defer { AttachmentStaging.remove(file.url) }
        let waveform: [UInt8] = [0, 128, 255]
        let prepared = try XCTUnwrap(PreparedAttachment.voice(VoiceRecording(fileURL: file.url, durationMs: 4_200, waveform: waveform)))
        XCTAssertTrue(prepared.voice)
        XCTAssertEqual(prepared.fileURL, file.url, "never compressed or copied")
        XCTAssertEqual(prepared.name, VoiceMessage.uploadFilename)
        XCTAssertEqual(prepared.contentType, "audio/mp4")
        XCTAssertEqual(prepared.kind, .audio)
        XCTAssertEqual(prepared.byteSize, 4_096)
        XCTAssertEqual(prepared.durationMs, 4_200)
        XCTAssertEqual(prepared.waveform, VoiceWaveform.encode(waveform))
        XCTAssertNil(prepared.preview)
        let local = prepared.localVoiceAttachment(id: "local")
        XCTAssertTrue(local.voice)
        XCTAssertEqual(local.name, VoiceMessage.storedName)
        XCTAssertEqual(local.url, file.url.absoluteString)
        XCTAssertEqual(local.waveformBytes, waveform)
        XCTAssertNil(PreparedAttachment.voice(VoiceRecording(fileURL: file.url, durationMs: 400, waveform: waveform)))
        let missing = file.url.deletingLastPathComponent().appendingPathComponent("missing.m4a")
        XCTAssertNil(PreparedAttachment.voice(VoiceRecording(fileURL: missing, durationMs: 4_200, waveform: waveform)))
    }

    func testVoiceUploadDeclaresVoiceDurationAndWaveformOnlyForVoice() throws {
        let voice = AssetCreateInput(channelId: "Chan12345678", filename: VoiceMessage.uploadFilename, contentType: "audio/mp4",
                                     byteSize: 12345, durationMs: 4200, voice: true, waveform: "/wCA")
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(voice)) as? [String: Any])
        XCTAssertEqual(body["voice"] as? Bool, true)
        XCTAssertEqual(body["durationMs"] as? Int, 4200)
        XCTAssertEqual(body["waveform"] as? String, "/wCA")
        XCTAssertEqual(body["contentType"] as? String, "audio/mp4")
        let file = AssetCreateInput(channelId: "Chan12345678", filename: "song.mp3", contentType: "audio/mpeg", byteSize: 5)
        let plain = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(file)) as? [String: Any])
        XCTAssertNil(plain["voice"], "regular files never declare voice")
        XCTAssertNil(plain["waveform"])
    }

    func testReplyExcerptsNameVoiceMessages() {
        XCTAssertEqual(VoiceMessage.excerptFallback(hasAttachments: true, hasVoice: true), "Voice message")
        XCTAssertEqual(VoiceMessage.excerptFallback(hasAttachments: true, hasVoice: false), "Attachment")
        XCTAssertEqual(VoiceMessage.excerptFallback(hasAttachments: false, hasVoice: false), "Message")
        let author = ChatAuthor(id: "u", name: "U", isGuest: false)
        let note = ChatAttachment(id: "Asset0000001", kind: .audio, contentType: "audio/mp4", name: "Voice message.m4a", size: 1, voice: true)
        let song = ChatAttachment(id: "Asset0000002", kind: .audio, contentType: "audio/mpeg", name: "song.mp3", size: 1)
        func reply(_ attachments: [ChatAttachment]) -> MessageReply {
            MessageReply(message: ChatMessage(id: "Message00000001", channelId: "Chan12345678", seq: "1", author: author,
                                              content: ChatContent(version: 1, type: "text", text: "", attachments: attachments),
                                              createdAt: "now", clientMessageId: "c"))
        }
        XCTAssertTrue(reply([note]).hasVoice)
        XCTAssertFalse(reply([song]).hasVoice)
        XCTAssertFalse(reply([]).hasVoice)
        var transcribed = note
        transcribed.transcript = .pending
        XCTAssertEqual(VoiceMessage.excerptFallback(hasAttachments: true, hasVoice: true, attachments: [transcribed]), "Transcribing…")
        transcribed.transcript = .ready("  North entrance.  ")
        XCTAssertEqual(VoiceMessage.excerptFallback(hasAttachments: true, hasVoice: true, attachments: [transcribed]), "North entrance.")
        transcribed.spoiler = true
        XCTAssertEqual(VoiceMessage.excerptFallback(hasAttachments: true, hasVoice: true, attachments: [transcribed]), "Voice message")
    }

    func testDictatedWordsGoInAtTheCaretSpacedFromTheirNeighbours() {
        XCTAssertEqual(VoiceMessage.dictationText(before: "", after: "", words: "  hello there "), "hello there")
        XCTAssertEqual(VoiceMessage.dictationText(before: "see you", after: "", words: "at five"), " at five")
        XCTAssertEqual(VoiceMessage.dictationText(before: "see you ", after: "", words: "at five"), "at five")
        XCTAssertEqual(VoiceMessage.dictationText(before: "", after: "tomorrow", words: "see you"), "see you ")
        XCTAssertEqual(VoiceMessage.dictationText(before: "ok", after: ", thanks", words: "sounds good"), " sounds good")
        XCTAssertEqual(VoiceMessage.dictationText(before: "a", after: "b", words: " \n"), "")
        // UTF-16 selections: the emoji counts two, and a selection is replaced.
        XCTAssertEqual(VoiceMessage.dictationText(in: "👋hi", selection: NSRange(location: 2, length: 0), words: "oh"), " oh ")
        XCTAssertEqual(VoiceMessage.dictationText(in: "say hi", selection: NSRange(location: 4, length: 2), words: "hello"), "hello")
        XCTAssertEqual(VoiceMessage.dictationText(in: "x", selection: NSRange(location: 9, length: 3), words: "y"), " y")
    }

    func testDictationFailuresExplainThemselves() {
        XCTAssertTrue(VoiceMessage.dictationError(status: 429).contains("this hour"))
        XCTAssertTrue(VoiceMessage.dictationError(status: 413).contains("10 minutes"))
        XCTAssertTrue(VoiceMessage.dictationError(status: 503).contains("isn’t available"))
        XCTAssertEqual(VoiceMessage.dictationError(status: 502), "Transcription failed. Try again.")
        XCTAssertTrue(VoiceMessage.dictationError(status: nil).contains("connection"))
    }

    #if os(macOS)
    @MainActor
    private func appKitVoiceChat() async -> ChatModel {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [VoiceComposerURLProtocol.self]
        let session = URLSession(configuration: configuration)
        let api = APIClient(baseURL: URL(string: "https://caper.invalid")!, session: session, tokenStore: VoiceComposerTokenStore())
        let chat = ChatModel(api: api)
        addTeardownBlock {
            await chat.stop(discardingDrafts: true)
            session.invalidateAndCancel()
        }
        await chat.open(channelID: "Chan12345678", displayName: "Me")
        XCTAssertTrue(chat.canInteract, "obtain a real ChatModel session through the simulated API, not a preview or a forced flag")
        XCTAssertEqual(chat.currentAuthor?.id, "self")
        XCTAssertEqual(chat.channelID, "Chan12345678")
        return chat
    }

    @MainActor
    func testAppKitDictationReplacesUTF16SelectionWithNativeUndoAndRespectsEditingLimits() async throws {
        let chat = await appKitVoiceChat()
        let native = VoiceComposerNativeEditor(chat: chat, inThread: false)
        defer { native.composer.cancel(); native.window.close() }
        let prefix = "🧑🏽‍🚀left", selected = "old🧭", suffix = "RIGHT🦊!"
        let original = prefix + selected + suffix
        native.seed(original, selection: (original as NSString).range(of: selected))
        let expected = prefix + " hello 🌿 " + suffix
        let undo = try XCTUnwrap(native.editor.undoManager, "a window-backed text view must support native undo")
        let groupsByEvent = undo.groupsByEvent
        undo.groupsByEvent = false
        undo.beginUndoGrouping()
        native.assertInsertion("  hello 🌿 \n", expected: expected)
        undo.endUndoGrouping()
        XCTAssertEqual(native.editor.selectedRange(), NSRange(location: (prefix + " hello 🌿 ").utf16.count, length: 0))
        XCTAssertTrue(undo.canUndo)
        undo.undo()
        XCTAssertEqual(native.editor.string, original)
        XCTAssertEqual(chat.draft, original, "undo also reaches the model through textDidChange")
        XCTAssertEqual(native.changes, [expected, original])
        XCTAssertFalse(undo.canUndo, "one dictation is one native undo step")
        undo.groupsByEvent = groupsByEvent

        // Both sides survive replacement. Emoji and a combining mark distinguish
        // Unicode scalars from both UTF-16 units and grapheme clusters.
        let left = String(repeating: "🦊", count: 1_997) + " "
        let right = " " + String(repeating: "z", count: 1_997)
        native.seed(left + selected + right, selection: NSRange(location: left.utf16.count, length: selected.utf16.count))
        native.assertRefusal("e\u{301}🌿🪴!", reason: "4,001 scalars is refused without touching either side")
        XCTAssertFalse(undo.canUndo, "a refused insertion does not register an edit")
        let atLimit = left + "e\u{301}🌿🪴" + right
        native.assertInsertion("e\u{301}🌿🪴", expected: atLimit)
        XCTAssertEqual(native.editor.string.unicodeScalars.count, 4_000)
        XCTAssertGreaterThan(native.editor.string.utf16.count, 4_000)
        XCTAssertLessThan(native.editor.string.count, 4_000)
        XCTAssertEqual(native.editor.selectedRange(), NSRange(location: (left + "e\u{301}🌿🪴").utf16.count, length: 0))

        native.seed("editable 🐙")
        native.editor.isEditable = false
        native.assertRefusal("read only", reason: "native read-only editors cannot accept dictation")
        native.editor.isEditable = true
        XCTAssertTrue(native.window.makeFirstResponder(native.editor))
        native.editor.setMarkedText("かな", selectedRange: NSRange(location: 2, length: 0),
                                    replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(native.editor.hasMarkedText(), "use a real native composition, not a hasMarkedText override")
        native.assertRefusal("interrupt", reason: "an IME composition stays intact")
        XCTAssertTrue(native.editor.hasMarkedText())
        native.editor.unmarkText()
        XCTAssertFalse(native.editor.hasMarkedText())
        native.assertInsertion("again", expected: native.editor.string + " again")
    }

    @MainActor
    func testAppKitDictationLifetimeRefusesStaleAndHiddenOwnersWithoutCancellingTheNeighbor() async throws {
        let chat = await appKitVoiceChat()
        await chat.openThread("Root00000001")
        XCTAssertEqual(chat.threadRootID, "Root00000001")
        XCTAssertNil(chat.threadError)
        let channel = VoiceComposerNativeEditor(chat: chat, inThread: false)
        let thread = VoiceComposerNativeEditor(chat: chat, inThread: true)
        let recorder = VoiceMessageRecorder.shared
        var simulatedFiles: [URL] = []
        defer {
            channel.composer.cancel(); thread.composer.cancel()
            channel.window.close(); thread.window.close()
            simulatedFiles.forEach { AttachmentStaging.remove($0) }
        }
        guard recorder.phase == .idle, recorder.dictation == nil, recorder.notice == nil else {
            XCTFail("this no-microphone test requires an idle shared recorder"); return
        }
        @MainActor
        func simulateCompletedDictation(inThread: Bool) throws -> URL {
            // Explicit simulation: disposable bytes stand for a completed failed
            // dictation. Never start capture, playback, transcription or upload.
            let file = try AttachmentStaging.stage(data: Data("simulated recording".utf8), filename: VoiceMessage.uploadFilename)
            simulatedFiles.append(file.url)
            let target = VoiceTarget.chat(chat, inThread: inThread)
            recorder.dictation = VoiceDictation(target: target,
                recording: VoiceRecording(fileURL: file.url, durationMs: 1_000, waveform: []), error: "Simulated failure")
            recorder.show("Simulated dictation notice", for: target.owner, seconds: 60)
            return file.url
        }

        channel.seed("channel 🧩")
        thread.seed("thread 🐙")
        channel.assertInsertion("one", expected: "channel 🧩 one")
        XCTAssertEqual(chat.threadDraft, "thread 🐙")
        thread.assertInsertion("two", expected: "thread 🐙 two")
        XCTAssertEqual(chat.draft, "channel 🧩 one")
        let neighborFile = try simulateCompletedDictation(inThread: true)
        XCTAssertTrue(thread.composer.hasNotice)
        XCTAssertFalse(channel.composer.hasNotice)
        channel.composer.cancel()
        channel.composer.synchronize(visible: false)
        channel.assertRefusal("hidden", reason: "a hidden channel composer is not an insertion destination")
        XCTAssertTrue(thread.composer.hasNotice)
        XCTAssertEqual(recorder.dictation?.recording.fileURL, neighborFile)
        XCTAssertTrue(FileManager.default.fileExists(atPath: neighborFile.path))
        thread.assertInsertion("reply", expected: "thread 🐙 two reply")
        channel.composer.synchronize(visible: true)
        channel.assertInsertion("visible", expected: "channel 🧩 one visible")
        thread.composer.cancel()
        XCTAssertFalse(thread.composer.hasNotice)
        XCTAssertNil(recorder.dictation)
        XCTAssertNil(recorder.notice)
        XCTAssertFalse(FileManager.default.fileExists(atPath: neighborFile.path))
        XCTAssertEqual(chat.draft, "channel 🧩 one visible")
        XCTAssertEqual(chat.threadDraft, "thread 🐙 two reply", "cancel never clears typed text")

        let rootFile = try simulateCompletedDictation(inThread: true)
        await chat.openThread("Root00000002")
        XCTAssertEqual(chat.threadRootID, "Root00000002")
        XCTAssertNil(chat.threadError)
        XCTAssertTrue(chat.canInteract)
        thread.seed("new root 🐬")
        thread.assertRefusal("old root", reason: "a changed root refuses insertion before synchronize runs")
        channel.assertInsertion("independent", expected: "channel 🧩 one visible independent")
        thread.composer.synchronize(visible: true)
        XCTAssertNil(recorder.dictation)
        XCTAssertNil(recorder.notice)
        XCTAssertFalse(FileManager.default.fileExists(atPath: rootFile.path))
        thread.assertInsertion("current", expected: "new root 🐬 current")

        let contextFile = try simulateCompletedDictation(inThread: false)
        let context = chat.editingContext, owner = VoiceComposerID(chat: chat, inThread: false)
        await chat.open(channelID: "Chan12345678", displayName: "Me")
        XCTAssertTrue(chat.canInteract)
        XCTAssertNotEqual(chat.editingContext, context)
        XCTAssertEqual(VoiceComposerID(chat: chat, inThread: false), owner, "same owner, but a new access generation")
        channel.seed("reopened 🦋")
        channel.assertRefusal("old context", reason: "same-channel reopening still invalidates the old context")
        channel.composer.synchronize(visible: true)
        XCTAssertNil(recorder.dictation)
        XCTAssertNil(recorder.notice)
        XCTAssertFalse(FileManager.default.fileExists(atPath: contextFile.path))
        channel.assertInsertion("fresh", expected: "reopened 🦋 fresh")

        let hiddenFile = try simulateCompletedDictation(inThread: false)
        channel.composer.synchronize(visible: false)
        XCTAssertFalse(channel.composer.hasNotice)
        XCTAssertNil(recorder.dictation)
        XCTAssertNil(recorder.notice)
        XCTAssertFalse(FileManager.default.fileExists(atPath: hiddenFile.path))
        channel.assertRefusal("hidden owner", reason: "hiding cancels only this owner's completed dictation")
        channel.composer.synchronize(visible: true)
        channel.assertInsertion("back", expected: "reopened 🦋 fresh back")

        let history = try XCTUnwrap(chat.currentSnapshot())
        await chat.preview(history: history)
        channel.composer.synchronize(visible: true)
        XCTAssertFalse(chat.canInteract)
        XCTAssertTrue(channel.editor.isEditable)
        channel.assertRefusal("preview", reason: "a current, editable native editor still needs permission to interact")
    }
    #endif
}

#if os(macOS)
/// Real AppKit editing and delegate-to-model binding; only HTTP is simulated.
@MainActor
private final class VoiceComposerNativeEditor: NSObject, NSTextViewDelegate {
    let chat: ChatModel
    let inThread: Bool
    let editor: NSTextView
    let window: NSWindow
    let composer: AppKitVoiceComposer
    var changes: [String] = []
    var draft: String { inThread ? chat.threadDraft : chat.draft }

    init(chat: ChatModel, inThread: Bool) {
        self.chat = chat; self.inThread = inThread
        _ = NSApplication.shared
        let editor = NSTextView(frame: NSRect(x: 0, y: 0, width: 480, height: 120))
        self.editor = editor
        window = NSWindow(contentRect: editor.frame, styleMask: [.titled], backing: .buffered, defer: false)
        composer = AppKitVoiceComposer(chat: chat, inThread: inThread, editor: editor)
        super.init()
        window.isReleasedWhenClosed = false
        window.contentView = editor
        editor.isRichText = false; editor.isEditable = true; editor.allowsUndo = true
        editor.delegate = self
        XCTAssertTrue(window.makeFirstResponder(editor))
    }

    func textDidChange(_ notification: Notification) {
        changes.append(editor.string)
        if inThread { chat.threadDraft = editor.string } else { chat.draft = editor.string }
    }

    func seed(_ text: String, selection: NSRange? = nil) {
        editor.string = text
        if inThread { chat.threadDraft = text } else { chat.draft = text }
        editor.setSelectedRange(selection ?? NSRange(location: text.utf16.count, length: 0))
        editor.undoManager?.removeAllActions()
        changes = []
    }

    func assertInsertion(_ words: String, expected: String, file: StaticString = #filePath, line: UInt = #line) {
        let count = changes.count
        XCTAssertTrue(composer.insert(words), file: file, line: line)
        XCTAssertEqual(editor.string, expected, file: file, line: line)
        XCTAssertEqual(draft, expected, file: file, line: line)
        XCTAssertEqual(changes.count, count + 1, "native textDidChange fires for each accepted edit", file: file, line: line)
        XCTAssertEqual(changes.last, expected, file: file, line: line)
    }

    func assertRefusal(_ words: String, reason: String, file: StaticString = #filePath, line: UInt = #line) {
        let text = editor.string, selection = editor.selectedRange(), modelText = draft, notifications = changes
        let marked = editor.markedRange()
        XCTAssertFalse(composer.insert(words), reason, file: file, line: line)
        XCTAssertEqual(editor.string, text, reason, file: file, line: line)
        XCTAssertEqual(editor.selectedRange(), selection, reason, file: file, line: line)
        XCTAssertEqual(editor.markedRange(), marked, reason, file: file, line: line)
        XCTAssertEqual(draft, modelText, reason, file: file, line: line)
        XCTAssertEqual(changes, notifications, "refusals must not emit textDidChange", file: file, line: line)
    }
}

private final class VoiceComposerTokenStore: TokenStore, @unchecked Sendable {
    private var token: String? = "simulated-account-token"
    func load() throws -> String? { token }
    func save(_ token: String) throws { self.token = token }
    func clear() throws { token = nil }
}

/// Stateless responses follow AttachmentTests' composer setup, without live HTTP.
private final class VoiceComposerURLProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let path = request.url!.path
        let components = path.split(separator: "/")
        let body: String
        switch (request.httpMethod ?? "GET", path) {
        case ("POST", "/api/chat/session"):
            body = #"{"token":"simulated-chat-token","author":{"id":"self","name":"Me","isGuest":false}}"#
        case ("GET", _) where path.hasSuffix("/messages") && components.count == 5:
            body = #"{"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\#(components[3])","name":"general"},"messages":[],"cursor":"0","hasMore":false}"#
        case ("GET", _) where path.hasSuffix("/thread") && components.count == 7:
            body = #"{"root":{"id":"\#(components[5])","channelId":"\#(components[3])","seq":"1","author":{"id":"self","name":"Me","isGuest":false},"content":{"version":1,"type":"text","text":"Root"},"createdAt":"now","clientMessageId":"root"},"messages":[],"cursor":"1","hasMore":false}"#
        case ("GET", _) where path.hasSuffix("/threads/unread"):
            body = #"{"threads":[]}"#
        default:
            client?.urlProtocol(self, didFailWithError: URLError(.unsupportedURL)); return
        }
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil,
                                                             headerFields: ["content-type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
#endif
