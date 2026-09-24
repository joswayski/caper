import XCTest
import AVFoundation
@testable import CaperCore
#if os(macOS)
import CaperRTCBridge
#endif

final class ProtocolTests: XCTestCase {
    @MainActor
    func testAudioPreviewRequiresExplicitLoopbackParityMode() {
        var environment = ["CAPER_TEST_MODE": "parity", "CAPER_UI_FIXTURE": "audio-recorded", "CAPER_API_BASE_URL": "http://127.0.0.1:3001"]
        XCTAssertTrue(CaperRuntime.isAudioPreview("audio-recorded", environment: environment))
        XCTAssertFalse(CaperRuntime.isAudioPreview("audio-statistics", environment: environment))
        environment["CAPER_API_BASE_URL"] = "https://caper.chat"
        XCTAssertFalse(CaperRuntime.isAudioPreview("audio-recorded", environment: environment))
        environment["CAPER_API_BASE_URL"] = "http://localhost:3001"
        environment.removeValue(forKey: "CAPER_TEST_MODE")
        XCTAssertFalse(CaperRuntime.isAudioPreview("audio-recorded", environment: environment))
    }

    #if os(macOS)
    @MainActor
    func testTimedMicrophoneStopSavesBothNaturalAndEnhancedPCMWithoutHardware() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("caper-mic-test-fixture-\(UUID().uuidString).caf")
        let test = MacMicrophoneTest(fileURL: url)
        defer { test.close() }
        let raw = Array(repeating: Int16(1200), count: 22_050)
        let processed = Array(repeating: Int16(2400), count: 22_050)
        let natural = raw.withUnsafeBytes { Data($0) }
        let enhanced = processed.withUnsafeBytes { Data($0) }
        test.saveComparison(CaperAudioComparison(natural: natural, enhanced: enhanced, sampleRate: 44_100))
        XCTAssertTrue(test.hasRecording, "Timed stop and explicit stop use the same bounded PCM save path")
        XCTAssertEqual(MacMicrophoneTest.recordedDuration(at: url), 0.5, accuracy: 0.01)
        let enhancedURL = url.deletingPathExtension().appendingPathExtension("enhanced.caf")
        XCTAssertEqual(MacMicrophoneTest.recordedDuration(at: enhancedURL), 0.5, accuracy: 0.01)
        let saved = try AVAudioFile(forReading: enhancedURL, commonFormat: .pcmFormatInt16, interleaved: true)
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: saved.processingFormat, frameCapacity: 22_050))
        try saved.read(into: buffer)
        XCTAssertEqual(buffer.int16ChannelData?.pointee[400], 2400, "Enhanced samples must not be replaced by raw input")
        XCTAssertEqual(MacMicrophoneTest.playbackDecibels(for: 0), -96)
        XCTAssertEqual(MacMicrophoneTest.playbackDecibels(for: 100), 0)
        XCTAssertEqual(MacMicrophoneTest.playbackDecibels(for: 200), 6.0206, accuracy: 0.001)
    }
    #endif

    func testVoiceStatisticsAggregatesAudioAndClassifiesOnlySelectedRoute() {
        let stats: [String: VoiceStatistic] = [
            "in": VoiceStatistic(type: "inbound-rtp", values: ["kind": "audio", "bytesReceived": 3_000, "packetsLost": 4, "jitter": 0.017]),
            "out": VoiceStatistic(type: "outbound-rtp", values: ["kind": "audio", "bytesSent": 5_000]),
            "video": VoiceStatistic(type: "inbound-rtp", values: ["kind": "video", "bytesReceived": 900_000, "packetsLost": 300]),
            "transport": VoiceStatistic(type: "transport", values: ["selectedCandidatePairId": "selected"]),
            "selected": VoiceStatistic(type: "candidate-pair", values: ["localCandidateId": "relay", "currentRoundTripTime": 0.042]),
            "old": VoiceStatistic(type: "candidate-pair", values: ["localCandidateId": "direct", "currentRoundTripTime": 3]),
            "relay": VoiceStatistic(type: "local-candidate", values: ["candidateType": "relay", "address": "192.0.2.1"]),
            "direct": VoiceStatistic(type: "local-candidate", values: ["candidateType": "host"])
        ]
        let (_, first) = VoiceDiagnostics.read(stats, timestampUs: 1_000_000, previous: nil)
        let (current, _) = VoiceDiagnostics.read(stats, timestampUs: 3_000_000,
            previous: VoiceStatisticsSample(timestampUs: first.timestampUs, receivedBytes: 1_000, sentBytes: 4_000))
        XCTAssertEqual(current.receivedBytes, 3_000)
        XCTAssertEqual(current.sentBytes, 5_000)
        XCTAssertEqual(current.receiveBitrate, 8_000)
        XCTAssertEqual(current.sendBitrate, 4_000)
        XCTAssertEqual(current.packetsLost, 4)
        XCTAssertEqual(current.maxJitterMs, 17)
        XCTAssertEqual(current.roundTripMs, 42)
        XCTAssertEqual(current.route, "relay", "Only the selected pair determines the route")
        let (reset, _) = VoiceDiagnostics.read(stats, timestampUs: 4_000_000,
            previous: VoiceStatisticsSample(timestampUs: 3_000_000, receivedBytes: 5_000, sentBytes: 6_000))
        XCTAssertNil(reset.receiveBitrate, "A counter reset is not a negative bitrate")
        XCTAssertNil(reset.sendBitrate)
    }

    func testSequenceComparisonDoesNotRoundThroughDouble() throws {
        XCTAssertEqual(try Sequence.compare("9007199254740993", "9007199254740992"), .orderedDescending)
        XCTAssertEqual(try Sequence.compare("184467440737095516160", "99999999999999999999"), .orderedDescending)
        XCTAssertThrowsError(try Sequence.compare("01", "1"))
        XCTAssertThrowsError(try Sequence.compare("-1", "0"))
        XCTAssertThrowsError(try Sequence.compare("١", "1"), "Arabic-Indic digits are not canonical server cursors")
    }

    func testSuccessorRejectsGapAndOverflow() {
        XCTAssertTrue(Sequence.isSuccessor("43", of: "42"))
        XCTAssertFalse(Sequence.isSuccessor("44", of: "42"))
        XCTAssertFalse(Sequence.isSuccessor("18446744073709551616", of: "18446744073709551615"))
    }

    func testHTTPConfirmationDoesNotAdvanceReplayCursor() {
        var delivery = ChatDeliveryState(cursor: "41")
        let command = delivery.begin(text: "hello", makeID: { "stable-id" })
        delivery.confirmHTTP(id: command.id)
        XCTAssertEqual(delivery.cursor, "41")
        XCTAssertTrue(delivery.receive(seq: "42"))
        XCTAssertEqual(delivery.cursor, "42")
    }

    func testUnknownOutcomeRetriesExactCommandAndResetPreventsResurrection() {
        var delivery = ChatDeliveryState(cursor: "8")
        let first = delivery.begin(text: "original", makeID: { "id-one" })
        let retry = delivery.begin(text: "edited draft", makeID: { "id-two" })
        XCTAssertEqual(first, retry)
        XCTAssertFalse(delivery.receive(seq: "10"), "a replay gap must resync instead of skipping seq 9")
        XCTAssertEqual(delivery.cursor, "8")
        delivery.reset()
        delivery.confirmHTTP(id: first.id)
        XCTAssertEqual(delivery.cursor, "0")
        XCTAssertNil(delivery.pending)
    }

    func testGatewayConfirmationClearsOnlyMatchingSenderAndValidationUnlocksEditing() {
        var delivery = ChatDeliveryState(cursor: "3")
        let pending = delivery.begin(text: "first", makeID: { "client-one" })
        XCTAssertFalse(delivery.confirmGateway(clientMessageID: pending.id, authorID: "other", ownAuthorID: "self"))
        XCTAssertEqual(delivery.pending, pending)
        XCTAssertTrue(delivery.confirmGateway(clientMessageID: pending.id, authorID: "self", ownAuthorID: "self"))
        XCTAssertNil(delivery.pending)
        XCTAssertFalse(
            delivery.confirmGateway(clientMessageID: pending.id, authorID: "self", ownAuthorID: "self"),
            "a later HTTP failure can recognize that the gateway already confirmed delivery"
        )

        let rejected = delivery.begin(text: "invalid", makeID: { "client-two" })
        delivery.reject(id: rejected.id)
        let edited = delivery.begin(text: "edited", makeID: { "client-three" })
        XCTAssertEqual(edited.id, "client-three")
        XCTAssertEqual(edited.text, "edited")
    }

    func testResyncResetPreservesUnknownOutcomeButChannelResetDoesNot() {
        var delivery = ChatDeliveryState(cursor: "12")
        let pending = delivery.begin(text: "possibly delivered", makeID: { "stable" })
        delivery.reset(cursor: "14", preservingPending: true)
        XCTAssertEqual(delivery.pending, pending)
        XCTAssertEqual(delivery.cursor, "14")
        delivery.reset(cursor: "0")
        XCTAssertNil(delivery.pending)
    }

    func testKeychainNamespaceIncludesCanonicalOrigin() {
        XCTAssertEqual(APIClient.sessionService(for: URL(string: "https://CAPER.chat")!), "chat.caper.session")
        XCTAssertNotEqual(
            APIClient.sessionService(for: URL(string: "https://staging.caper.chat")!),
            APIClient.sessionService(for: URL(string: "https://caper.chat")!)
        )
        XCTAssertNotEqual(
            APIClient.sessionService(for: URL(string: "https://caper.chat:444")!),
            APIClient.sessionService(for: URL(string: "https://caper.chat")!)
        )
    }

    func testMessageValidationCountsCharactersAndRejectsControls() {
        XCTAssertNil(MessageValidation.error(for: String(repeating: "🪐", count: 4_000)))
        XCTAssertEqual(MessageValidation.error(for: String(repeating: "🪐", count: 4_001)), "Messages can be at most 4,000 characters.")
        XCTAssertEqual(MessageValidation.error(for: String(repeating: "e\u{301}", count: 2_001)), "Messages can be at most 4,000 characters.", "Messages count Unicode scalars, not grapheme clusters.")
        XCTAssertNil(MessageValidation.error(for: String(repeating: "👨‍👩‍👧‍👦", count: 571)), "Joiners are valid message content, not forbidden control characters.")
        XCTAssertEqual(MessageValidation.error(for: String(repeating: "👨‍👩‍👧‍👦", count: 572)), "Messages can be at most 4,000 characters.", "Messages count every scalar in a ZWJ sequence.")
        XCTAssertNil(MessageValidation.error(for: "line one\nline two\tindented"))
        XCTAssertEqual(MessageValidation.error(for: "hidden\u{0007}"), "Messages cannot contain control characters.")
    }

    func testHTTPConfirmationMustMatchTheExactCommandChannelAndAuthor() {
        let command = PendingMessage(id: "client", text: "expected")
        let author = ChatAuthor(id: "self", name: "Self", isGuest: false)
        let content = ChatContent(version: 1, type: "text", text: "expected")
        let valid = ChatMessage(id: "m", channelId: "Channel12345", seq: "1", author: author, content: content, createdAt: "now", clientMessageId: "client")
        XCTAssertTrue(MessageValidation.acceptsResponse(valid, channelID: "Channel12345", command: command, authorID: "self"))
        let wrongChannel = ChatMessage(id: "m", channelId: "Other1234567", seq: "1", author: author, content: content, createdAt: "now", clientMessageId: "client")
        XCTAssertFalse(MessageValidation.acceptsResponse(wrongChannel, channelID: "Channel12345", command: command, authorID: "self"))
        let wrongText = ChatMessage(id: "m", channelId: "Channel12345", seq: "1", author: author, content: ChatContent(version: 1, type: "text", text: "changed"), createdAt: "now", clientMessageId: "client")
        XCTAssertFalse(MessageValidation.acceptsResponse(wrongText, channelID: "Channel12345", command: command, authorID: "self"))
    }

    func testRedirectPolicyAllowsDefaultPortButBlocksCredentialExfiltration() {
        XCTAssertTrue(RedirectPolicy.sameOrigin(URL(string: "https://caper.chat/a")!, URL(string: "https://CAPER.chat:443/b")!))
        XCTAssertFalse(RedirectPolicy.sameOrigin(URL(string: "https://caper.chat/a")!, URL(string: "https://evil.example/b")!))
        XCTAssertFalse(RedirectPolicy.sameOrigin(URL(string: "https://caper.chat/a")!, URL(string: "http://caper.chat/b")!))
        XCTAssertFalse(RedirectPolicy.sameOrigin(URL(string: "https://caper.chat/a")!, URL(string: "https://caper.chat:444/b")!))
    }

    func testIceServerAcceptsSingleURLAndArray() throws {
        let decoder = JSONDecoder()
        XCTAssertEqual(try decoder.decode(IceServer.self, from: Data(#"{"urls":"stun:one"}"#.utf8)).urls, ["stun:one"])
        XCTAssertEqual(try decoder.decode(IceServer.self, from: Data(#"{"urls":["turn:one","turn:two"],"username":"u","credential":"p"}"#.utf8)).urls, ["turn:one", "turn:two"])
    }

    func testSnapshotRevisionCannotMoveBackwardsOrLoseVersioning() {
        var revisions = MonotonicRevision()
        XCTAssertTrue(revisions.accept(nil), "unversioned HTTP snapshots work before the server supplies revisions")
        XCTAssertTrue(revisions.accept(8))
        XCTAssertFalse(revisions.accept(nil), "an unversioned late HTTP response cannot overwrite a pushed snapshot")
        XCTAssertFalse(revisions.accept(7))
        XCTAssertFalse(revisions.accept(8))
        XCTAssertTrue(revisions.accept(9))
        XCTAssertEqual(revisions.latest, 9)
    }

    @MainActor
    func testDeafenRestoresMuteIntentIncludingChangesWhileDeafened() async {
        let key = "caper.voice.outputGain"
        let original = UserDefaults.standard.object(forKey: key)
        defer {
            if let original { UserDefaults.standard.set(original, forKey: key) }
            else { UserDefaults.standard.removeObject(forKey: key) }
        }
        let voice = VoiceClient(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        await voice.setMuted(false)
        await voice.setDeafened(true)
        XCTAssertTrue(voice.muted)
        XCTAssertTrue(voice.deafened)
        await voice.setDeafened(false)
        XCTAssertFalse(voice.muted, "undeafen restores the pre-deafen unmuted intent")

        await voice.setMuted(true)
        await voice.setDeafened(true)
        await voice.setMuted(false)
        XCTAssertTrue(voice.muted, "capture remains locally silent while deafened")
        await voice.setDeafened(false)
        XCTAssertFalse(voice.muted, "an explicit mute change while deafened becomes the restored intent")

        await voice.setMuted(true)
        await voice.setDeafened(false)
        XCTAssertTrue(voice.muted, "repeating an already-false deafen state must not restore stale intent")

        voice.setOutputGain(250)
        voice.setParticipantGain(-10, participantID: "remote")
        voice.setParticipantMuted(true, participantID: "remote")
        XCTAssertEqual(voice.outputGain, 200)
        XCTAssertEqual(VoiceClient(api: APIClient(baseURL: URL(string: "https://caper.invalid")!)).outputGain, 200,
                       "A new call reads the saved output gain")
        voice.setOutputGain(-4)
        XCTAssertEqual(VoiceClient(api: APIClient(baseURL: URL(string: "https://caper.invalid")!)).outputGain, 0,
                       "Saved gain stays within the supported 0–200 range")
        XCTAssertEqual(voice.participantGains["remote"], 0)
        XCTAssertTrue(voice.locallyMutedParticipants.contains("remote"))
    }
}
