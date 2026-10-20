import Foundation
import XCTest
@testable import CaperCore

/// Voice messages with transcripts (the shared voice-message contract).
final class VoiceMessageTests: XCTestCase {
    // MARK: Waveform

    func testWaveformKeepsEachSlicePeakNormalisedToTheLoudest() {
        // 128 samples: each of 64 slices covers two; the loudest slice is 0.5.
        var samples = [Float](repeating: 0.05, count: 128)
        samples[0] = 0.25
        samples[127] = 0.5
        samples[64] = 0.125
        let waveform = VoiceWaveform.build(samples)
        XCTAssertEqual(waveform.count, 64)
        XCTAssertEqual(waveform[63], 255, "the loudest slice is 255")
        XCTAssertEqual(waveform[0], 128, "0.25 / 0.5 scales linearly")
        XCTAssertEqual(waveform[32], 64)
        XCTAssertEqual(waveform[1], 26, "0.05 / 0.5 × 255")
    }

    func testWaveformStretchesShortRecordingsAndIsZeroForSilence() {
        XCTAssertEqual(VoiceWaveform.build([0.2, 0.4]), Array(repeating: 128, count: 32) + Array(repeating: 255, count: 32))
        XCTAssertEqual(VoiceWaveform.build([]), Array(repeating: 0, count: 64))
        XCTAssertEqual(VoiceWaveform.build([0, 0, 0]), Array(repeating: 0, count: 64))
        XCTAssertEqual(VoiceWaveform.build([.nan, -1, .infinity, 0.5]).max(), 255, "invalid readings count as silence")
        XCTAssertEqual(VoiceWaveform.build([.nan, -1, .infinity, 0.5]).filter { $0 > 0 }.count, 16)
        XCTAssertEqual(VoiceWaveform.build([1, 2], slices: 0), [])
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

    func testBarsResampleAnyLengthKeepingPeaksAndDrawAFlatPlaceholder() {
        let bytes: [UInt8] = [0, 255, 51, 102]
        XCTAssertEqual(VoiceWaveform.bars(bytes, count: 2), [1, 0.4], "downsampling keeps each range's peak")
        XCTAssertEqual(VoiceWaveform.bars(bytes, count: 8), [0, 0, 1, 1, 0.2, 0.2, 0.4, 0.4], "upsampling repeats")
        XCTAssertEqual(VoiceWaveform.bars(bytes, count: 4), [0, 1, 0.2, 0.4])
        XCTAssertEqual(VoiceWaveform.bars(nil, count: 3), [0, 0, 0], "no waveform draws flat bars")
        XCTAssertEqual(VoiceWaveform.bars([], count: 2), [0, 0])
        XCTAssertEqual(VoiceWaveform.bars(bytes, count: 0), [])
        let long = (0..<256).map { UInt8($0) }
        XCTAssertEqual(VoiceWaveform.bars(long, count: 64).last, 1)
        XCTAssertEqual(VoiceWaveform.bars(long, count: 64).count, 64)
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
        XCTAssertEqual(VoiceWaveform.liveBars([0.5, 1], count: 4), [0, 0, 0.5, 1], "new levels enter at the right")
        XCTAssertEqual(VoiceWaveform.liveBars([0.25, 0.5, 0.75], count: 2), [0.5, 0.75], "only the latest levels fit")
        XCTAssertEqual(VoiceWaveform.barCount(available: 100, width: 3, gap: 2), 20)
        XCTAssertEqual(VoiceWaveform.barCount(available: 0, width: 3, gap: 2), 1)
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

    func testRecordingRules() {
        XCTAssertFalse(VoiceMessage.keeps(durationMs: 999), "under a second is discarded")
        XCTAssertTrue(VoiceMessage.keeps(durationMs: 1_000))
        XCTAssertEqual(VoiceMessage.uploadDuration(measuredMs: 4_321, elapsedMs: 4_400), 4_321, "the measured file wins")
        XCTAssertEqual(VoiceMessage.uploadDuration(measuredMs: nil, elapsedMs: 4_400), 4_400)
        XCTAssertEqual(VoiceMessage.uploadDuration(measuredMs: 0, elapsedMs: 4_400), 4_400)
        XCTAssertEqual(VoiceMessage.uploadDuration(measuredMs: 600_040, elapsedMs: 600_000), 600_000, "never above the server's limit")
        XCTAssertEqual(VoiceMessage.release(pressDuration: 0.1, recordedMs: 0, cancelled: false), .hint, "a quick tap shows Hold to record")
        XCTAssertEqual(VoiceMessage.release(pressDuration: 0.8, recordedMs: 700, cancelled: false), .hint, "under a second is treated as a tap")
        XCTAssertEqual(VoiceMessage.release(pressDuration: 2, recordedMs: 1_900, cancelled: false), .send)
        XCTAssertEqual(VoiceMessage.release(pressDuration: 2, recordedMs: 1_900, cancelled: true), .cancel)
        XCTAssertFalse(VoiceMessage.cancelsBySliding(translation: -40))
        XCTAssertTrue(VoiceMessage.cancelsBySliding(translation: -VoiceMessage.cancelDistance))
        XCTAssertFalse(VoiceMessage.cancelsBySliding(translation: 200), "sliding right never cancels")
        XCTAssertEqual(VoiceMessage.slideProgress(translation: -VoiceMessage.cancelDistance / 2), 0.5)
        XCTAssertEqual(VoiceMessage.slideProgress(translation: 30), 0)
    }

    func testPlaybackPositions() {
        XCTAssertEqual(VoicePlaybackPosition.fraction(x: 50, width: 200), 0.25)
        XCTAssertEqual(VoicePlaybackPosition.fraction(x: -10, width: 200), 0)
        XCTAssertEqual(VoicePlaybackPosition.fraction(x: 500, width: 200), 1)
        XCTAssertEqual(VoicePlaybackPosition.fraction(x: 10, width: 0), 0)
        XCTAssertEqual(VoicePlaybackPosition.progress(elapsedMs: 1_000, durationMs: 4_000), 0.25)
        XCTAssertEqual(VoicePlaybackPosition.progress(elapsedMs: 9_000, durationMs: 4_000), 1)
        XCTAssertEqual(VoicePlaybackPosition.progress(elapsedMs: 1_000, durationMs: nil), 0)
        XCTAssertEqual(VoicePlaybackPosition.playedBars(progress: 0, count: 40), 0)
        XCTAssertEqual(VoicePlaybackPosition.playedBars(progress: 0.5, count: 40), 20)
        XCTAssertEqual(VoicePlaybackPosition.playedBars(progress: 0.01, count: 40), 1, "the bar under the playhead counts as played")
        XCTAssertEqual(VoicePlaybackPosition.playedBars(progress: 1, count: 40), 40)
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
    }
}
