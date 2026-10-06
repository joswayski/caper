import XCTest
import AVFoundation
@testable import CaperCore
#if os(macOS)
import CaperRTCBridge
#endif

final class ProtocolTests: XCTestCase {
    func testContentEditsPreserveIndependentMetadataAndOverlayStalePages() throws {
        let author = ChatAuthor(id: "author", name: "Author", isGuest: false)
        var original = ChatMessage(id: "message", channelId: "channel", seq: "3", author: author,
                                   content: ChatContent(version: 1, type: "text", text: "original"),
                                   createdAt: "2026-10-01T00:00:00Z", clientMessageId: "client")
        original.reactionSeq = "12"; original.pinSeq = "13"
        original.threadRootId = "root"; original.broadcast = true
        original.thread = ThreadSummary(replyCount: 4, participants: [author], seq: "11")
        var edited = original
        edited.content = ChatContent(version: 1, type: "text", text: "corrected")
        edited.revision = 2; edited.editSeq = "14"; edited.editedAt = "2026-10-06T00:00:00.123Z"
        edited.reactionSeq = nil; edited.pinSeq = nil
        edited.thread = ThreadSummary(replyCount: 1, participants: [author], seq: "4")
        var snapshots = EditSnapshots()
        snapshots.apply(edited) // Arrives before an older thread/history page.
        snapshots.seed([original])
        let merged = snapshots.overlay(original)
        XCTAssertEqual(merged.content.text, "corrected")
        XCTAssertEqual(merged.seq, "3")
        XCTAssertEqual(merged.createdAt, "2026-10-01T00:00:00Z")
        XCTAssertEqual(merged.reactionSeq, "12")
        XCTAssertEqual(merged.pinSeq, "13")
        XCTAssertEqual(merged.thread?.seq, "11")
        XCTAssertEqual(merged.threadRootId, "root")
        XCTAssertEqual(merged.broadcast, true)
        let raw = try JSONSerialization.jsonObject(with: JSONEncoder().encode(edited))
        var event: [String: Any] = ["type": "message.edited", "schemaVersion": 1, "channelId": "channel", "seq": "14", "message": raw]
        XCTAssertEqual(EditEvent.message(event, channelID: "channel")?.content.text, "corrected")
        event["seq"] = "3"
        XCTAssertNil(EditEvent.message(event, channelID: "channel"))
        event["seq"] = "14"
        XCTAssertNil(EditEvent.message(event, channelID: "other"))
    }

    func testEditCacheBoundsOnlyUnloadedMessagesAndRecoversAfterRefresh() {
        let author = ChatAuthor(id: "author", name: "Author", isGuest: false)
        var edited = ChatMessage(id: "message", channelId: "channel", seq: "3", author: author,
                                 content: ChatContent(version: 1, type: "text", text: "corrected"),
                                 createdAt: "2026-10-01T00:00:00Z", clientMessageId: "client")
        edited.revision = 2; edited.editSeq = "14"; edited.editedAt = "2026-10-06T00:00:00Z"
        func row(_ id: String) -> ChatMessage {
            let value = edited
            return ChatMessage(id: id, channelId: value.channelId, seq: value.seq, author: value.author,
                               content: value.content, createdAt: value.createdAt, clientMessageId: id,
                               revision: value.revision, editedAt: value.editedAt, editSeq: value.editSeq)
        }
        var snapshots = EditSnapshots()
        snapshots.seed((0...256).map { row("loaded-\($0)") })
        for index in 0..<256 { snapshots.apply(row("unseen-\(index)")) }
        XCTAssertFalse(snapshots.unseenOverflowed)
        snapshots.apply(row("overflow"))
        XCTAssertTrue(snapshots.unseenOverflowed)
        var old = row("unseen-0")
        old.revision = nil; old.editedAt = nil; old.editSeq = nil
        old.content = ChatContent(version: 1, type: "text", text: "old")
        XCTAssertEqual(snapshots.overlay(old).content.text, "corrected", "Overflow must not evict older corrections")
        snapshots.reset() // Fresh history clears overflow before reseeding loaded rows.
        snapshots.seed([row("loaded-0")])
        snapshots.apply(row("overflow"))
        XCTAssertFalse(snapshots.unseenOverflowed)
    }

    @MainActor
    func testSoundEffectsPreferencePersistsBothDirectionsWithoutOpeningOutput() throws {
        let suite = "caper-effects-test-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let effects = CaperEffects(enabled: false, defaults: defaults)
        XCTAssertTrue(effects.soundsEnabled)
        effects.soundsEnabled = false
        let reopened = CaperEffects(enabled: false, defaults: defaults)
        XCTAssertFalse(reopened.soundsEnabled)
        reopened.soundsEnabled = true
        XCTAssertTrue(CaperEffects(enabled: false, defaults: defaults).soundsEnabled)
    }

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

    @MainActor
    func testActiveRosterPreviewRequiresLoopbackFixture() throws {
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let channel = Channel(id: "chan00000001", spaceId: "space0000001", name: "general", private: false)
        let space = Space(id: channel.spaceId, name: "Fixture", ownerId: "owner0000001", demo: nil)
        model.detail = SpaceDetail(space: space, channels: [channel], members: [])
        model.selectedChannelID = channel.id
        let fixture = ["CAPER_TEST_MODE": "parity", "CAPER_UI_FIXTURE": "voice-roster", "CAPER_API_BASE_URL": "http://127.0.0.1:3001"]
        CaperRuntime.showVoiceRosterPreview(model, environment: fixture.merging(["CAPER_API_BASE_URL": "https://caper.chat"]) { _, new in new })
        XCTAssertNil(model.voice.context)
        CaperRuntime.showVoiceRosterPreview(model, environment: fixture)
        XCTAssertEqual(model.voice.context?.channelID, channel.id)
        XCTAssertEqual(model.voice.phase, .connected)
        XCTAssertEqual(model.voice.participants.count, 2)
        XCTAssertTrue(model.voice.isSelf(participantID: "fixture-self"))
        XCTAssertFalse(model.voice.isSelf(participantID: "fixture-remote"))
        XCTAssertTrue(model.voice.participants.allSatisfy { !$0.muted || $0.id == "fixture-remote" })
        model.voice.leaveImmediately()
    }

    #if os(macOS)
    @MainActor
    func testAllBundledWebEffectsDecodeWithoutOpeningOutput() throws {
        for effect in CaperEffects.Effect.allCases {
            let buffer = try XCTUnwrap(CaperEffects.decode(effect), "Missing canonical \(effect.rawValue) WAV")
            XCTAssertGreaterThan(buffer.frameLength, 100)
            let samples = try XCTUnwrap(buffer.floatChannelData).pointee
            let values = UnsafeBufferPointer(start: samples, count: Int(buffer.frameLength))
            XCTAssertTrue(values.allSatisfy { $0.isFinite && abs($0) <= 1 })
            XCTAssertTrue(values.contains { abs($0) > 0.01 })
        }
    }

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
        XCTAssertEqual(MacMicrophoneTest.playerVolume(for: 0), 0, "The EQ floor alone is not exact silence")
        XCTAssertEqual(MacMicrophoneTest.playerVolume(for: 200), 1)
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
            "selected": VoiceStatistic(type: "candidate-pair", values: ["localCandidateId": "relay", "currentRoundTripTime": 0.042,
                                                                           "requestsSent": 5, "responsesReceived": 4]),
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
        XCTAssertEqual(current.checks, "5 sent · 4 answered", "Web's connectivity checks on the selected pair")
        XCTAssertNil(current.timing, "The parser never invents join timing")
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

    func testThreadRetryKeepsDestinationAndSharedBroadcastIdentity() {
        var delivery = ChatDeliveryState(cursor: "41")
        let command = delivery.begin(text: "original", threadRootId: "root", broadcast: true, makeID: { "stable" })
        XCTAssertEqual(delivery.begin(text: "edited", threadRootId: "other", broadcast: false), command)
        var reply = ChatMessage(id: "reply", channelId: "channel", seq: "42",
            author: ChatAuthor(id: "author", name: "Author", isGuest: false),
            content: ChatContent(version: 1, type: "text", text: "original"),
            createdAt: "2026-10-06T00:00:00Z", clientMessageId: command.id,
            threadRootId: "root", broadcast: true)
        XCTAssertTrue(MessageValidation.acceptsResponse(reply, channelID: "channel", command: command, authorID: "author"))
        reply.broadcast = false
        XCTAssertFalse(MessageValidation.acceptsResponse(reply, channelID: "channel", command: command, authorID: "author"))
        reply.broadcast = true; reply.threadRootId = "different"
        XCTAssertFalse(MessageValidation.acceptsResponse(reply, channelID: "channel", command: command, authorID: "author"))
        delivery.confirmHTTP(id: command.id)
        XCTAssertEqual(delivery.cursor, "41")
    }

    func testReactionProtocolAndInterleavedCursorBookkeeping() {
        func event(_ seq: String = "2", channel: String = "channel") -> [String: Any] {
            ["type": "message.reactions", "schemaVersion": 1, "channelId": channel, "seq": seq,
             "messageId": "message", "reactions": [["emoji": "👍", "authorIds": ["author"]]]]
        }
        var delivery = ChatDeliveryState()
        XCTAssertTrue(delivery.receive(seq: "1"))
        XCTAssertEqual(ReactionEvent.sequence(event(), channelID: "channel"), "2")
        XCTAssertTrue(delivery.receive(seq: "2"))
        XCTAssertTrue(delivery.receive(seq: "2"), "duplicate reaction does not move or resync")
        XCTAssertTrue(delivery.receive(seq: "3"), "message after reaction remains contiguous")
        XCTAssertFalse(delivery.receive(seq: "5"), "reaction gap is rejected")
        XCTAssertNil(ReactionEvent.sequence(event(channel: "other"), channelID: "channel"))
        XCTAssertNil(ReactionEvent.sequence(event("02"), channelID: "channel"), "sequence must be canonical")
        var invalid = event()
        invalid["reactions"] = [["emoji": "👍", "authorIds": []]]
        XCTAssertNil(ReactionEvent.sequence(invalid, channelID: "channel"))
        invalid["reactions"] = [["emoji": "👍", "authorIds": ["author", "author"]]]
        XCTAssertNil(ReactionEvent.sequence(invalid, channelID: "channel"))
        XCTAssertFalse(MessageReactionsEvent(type: "message.reactions", schemaVersion: 1, channelId: "channel", seq: "02",
                                             messageId: "message", reactions: [MessageReaction(emoji: "👍", authorIds: ["author"])]).isValid)
        XCTAssertFalse(MessageReactionsEvent(type: "message.reactions", schemaVersion: 1, channelId: "channel", seq: "2",
                                             messageId: "message", reactions: [MessageReaction(emoji: "👍", authorIds: [])]).isValid)
    }

    func testReactionSnapshotsRejectStaleAckAndOverlayOlderPage() {
        let author = ChatAuthor(id: "other", name: "Other", isGuest: false)
        let content = ChatContent(version: 1, type: "text", text: "hello")
        let old = ChatMessage(id: "Message00000001", channelId: "Channel12345", seq: "1", author: author,
                              content: content, createdAt: "now", clientMessageId: "client",
                              reactions: [MessageReaction(emoji: "👍", authorIds: ["old"])], reactionSeq: "4")
        var snapshots = ReactionSnapshots()
        XCTAssertTrue(snapshots.apply(messageID: old.id, seq: "8", reactions: [MessageReaction(emoji: "👍", authorIds: ["self", "other"])]))
        XCTAssertFalse(snapshots.apply(messageID: old.id, seq: "7", reactions: []), "late HTTP acknowledgement must not revert replay")
        snapshots.seed([old])
        XCTAssertEqual(snapshots.overlay(old).reactionSeq, "8")
        XCTAssertEqual(snapshots.overlay(old).reactions?.first?.authorIds, ["self", "other"], "an older page must retain an unseen newer reaction")
    }

    func testPinProtocolAndSnapshotsKeepUnpinNewerThanStaleHistory() throws {
        let author = ChatAuthor(id: "other", name: "Other", isGuest: false)
        let oldPin = MessagePin(author: author, createdAt: "2026-10-01T00:00:00Z")
        var old = ChatMessage(id: "Message00000001", channelId: "Channel12345", seq: "1", author: author,
                              content: ChatContent(version: 1, type: "text", text: "old"), createdAt: "now",
                              clientMessageId: "client", pin: oldPin, pinSeq: "4")
        var unpinned = old; unpinned.pin = nil; unpinned.pinSeq = "8"
        var snapshots = PinSnapshots()
        XCTAssertTrue(snapshots.apply(unpinned))
        XCTAssertFalse(snapshots.apply(old), "an older page must not resurrect a newer unpin")
        XCTAssertNil(snapshots.overlay(old).pin)
        XCTAssertEqual(snapshots.overlay(old).pinSeq, "8")

        let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(unpinned)) as! [String: Any]
        let event: [String: Any] = ["type": "message.pin", "schemaVersion": 1, "channelId": old.channelId,
                                    "seq": "8", "message": encoded]
        XCTAssertEqual(PinEvent.sequence(event, channelID: old.channelId), "8")
        var mismatch = event; mismatch["seq"] = "9"
        XCTAssertNil(PinEvent.sequence(mismatch, channelID: old.channelId), "event sequence must equal message.pinSeq")
    }

    func testCompletePinHistoryRejectsOldAcknowledgementsAndPreservesNewerHTTP() {
        let author = ChatAuthor(id: "other", name: "Other", isGuest: false)
        let old = ChatMessage(id: "Message00000001", channelId: "Channel12345", seq: "1", author: author,
                              content: ChatContent(version: 1, type: "text", text: "old"), createdAt: "now",
                              clientMessageId: "client", pin: MessagePin(author: author, createdAt: "now"), pinSeq: "4")
        var snapshots = PinSnapshots()
        snapshots.replace([], cursor: "60")
        XCTAssertFalse(snapshots.apply(old), "absence from complete history covers unloaded messages")
        var boundary = old; boundary.pinSeq = "60"
        XCTAssertFalse(snapshots.apply(boundary))
        snapshots.seed([old])
        XCTAssertNil(snapshots.overlay(old).pin, "a stale page cannot restore the inline marker")
        XCTAssertEqual(snapshots.overlay(old).pinSeq, "60")
        var newer = old; newer.pinSeq = "61"
        XCTAssertTrue(snapshots.apply(newer))
        snapshots.replace([], cursor: "60")
        XCTAssertEqual(snapshots.overlay(old).pinSeq, "61", "an acknowledgement after snapshot capture survives refresh")
        XCTAssertEqual(snapshots.overlay(old).pin, newer.pin)
        snapshots.replace([], cursor: "62")
        XCTAssertNil(snapshots.overlay(old).pin, "an offline unpin supersedes the retained acknowledgement")
    }

    func testBundledEmojiCatalogHasCanonicalSelectableArtwork() {
        XCTAssertEqual(EmojiArtwork.choices.count, 1_870)
        XCTAssertEqual(EmojiArtwork.id(for: "❤️"), "2764")
        XCTAssertEqual(EmojiArtwork.id(for: "👨‍👩‍👧‍👦"), "1f468-200d-1f469-200d-1f467-200d-1f466")
        XCTAssertNotNil(EmojiArtwork.entry(for: "👍"))
        XCTAssertNotNil(EmojiArtwork.entry(for: "👍🏽"), "non-picker variants still need reaction artwork")
    }

    func testEmojiArtworkCropsPixelCoordinatesAtAsymmetricPosition() {
        let entry = EmojiArtwork.entry(for: "👍")
        XCTAssertEqual(entry?.x, 64)
        XCTAssertEqual(entry?.y, 128)
        guard let entry, let image = EmojiArtwork.image(for: entry) else {
            return XCTFail("Expected bundled thumbs-up artwork")
        }
        XCTAssertEqual(image.width, 64)
        XCTAssertEqual(image.height, 64)
        var pixels = [UInt8](repeating: 0, count: 64 * 64 * 4)
        pixels.withUnsafeMutableBytes { bytes in
            let context = CGContext(data: bytes.baseAddress, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 64 * 4,
                                    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            context?.draw(image, in: CGRect(x: 0, y: 0, width: 64, height: 64))
        }
        XCTAssertTrue(stride(from: 3, to: pixels.count, by: 4).contains { pixels[$0] != 0 }, "crop must contain visible pixels")
    }

    func testReactionSnapshotsRetainLoadedMessagesAndBoundOnlyUnseen() {
        let author = ChatAuthor(id: "author", name: "Author", isGuest: false)
        let content = ChatContent(version: 1, type: "text", text: "hello")
        let messages = (0..<600).map { index in
            ChatMessage(id: "message-\(index)", channelId: "channel", seq: "\(index + 1)", author: author,
                        content: content, createdAt: "now", clientMessageId: "client-\(index)",
                        reactions: [], reactionSeq: "1")
        }
        var snapshots = ReactionSnapshots()
        snapshots.seed(messages)
        XCTAssertTrue(snapshots.apply(messageID: messages[599].id, seq: "2", reactions: [MessageReaction(emoji: "👍", authorIds: ["author"])]))
        XCTAssertEqual(snapshots.overlay(messages[599]).reactions?.first?.emoji, "👍")

        for index in 0..<ReactionSnapshots.maximumUnseen {
            XCTAssertTrue(snapshots.apply(messageID: "unseen-\(index)", seq: "3", reactions: []))
        }
        XCTAssertFalse(snapshots.apply(messageID: "unseen-overflow", seq: "3", reactions: []))
        XCTAssertTrue(snapshots.unseenOverflowed, "ChatModel uses this marker to request a resync")
    }

    func testSwiftUUIDMatchesCanonicalServerHTTPAndGatewayConfirmation() {
        let uppercaseID = "AB12CD34-EF56-4789-8ABC-DEF012345678"
        let canonicalID = "ab12cd34-ef56-4789-8abc-def012345678"
        var delivery = ChatDeliveryState(cursor: "41")
        let command = delivery.begin(text: "Swag", makeID: { uppercaseID })
        XCTAssertEqual(command.id, canonicalID, "Rust parses UUIDs and returns lowercase, not Swift's original casing")
        XCTAssertEqual(delivery.begin(text: "next draft", makeID: { "other" }), command, "retries keep the same canonical identity")
        let author = ChatAuthor(id: "SelfAbC12345", name: "Self", isGuest: false)
        let message = ChatMessage(id: "m", channelId: "ChannelAbC12", seq: "42", author: author,
                                  content: ChatContent(version: 1, type: "text", text: "Swag"),
                                  createdAt: "2026-10-01T12:47:00Z", clientMessageId: canonicalID)
        XCTAssertTrue(MessageValidation.acceptsResponse(message, channelID: "ChannelAbC12", command: command, authorID: author.id))
        XCTAssertFalse(MessageValidation.acceptsResponse(message, channelID: "channelabc12", command: command, authorID: author.id),
                       "Only UUIDs are normalized; channel and author IDs are case sensitive")
        XCTAssertFalse(MessageValidation.acceptsResponse(message, channelID: "ChannelAbC12", command: command, authorID: "selfabc12345"))
        XCTAssertFalse(delivery.confirmGateway(clientMessageID: canonicalID, authorID: "other", ownAuthorID: author.id))
        XCTAssertFalse(delivery.confirmGateway(clientMessageID: "ab12cd34-ef56-4789-8abc-def012345679", authorID: author.id, ownAuthorID: author.id))
        XCTAssertTrue(delivery.receive(seq: "42"))
        XCTAssertTrue(delivery.confirmGateway(clientMessageID: canonicalID, authorID: author.id, ownAuthorID: author.id))
        XCTAssertNil(delivery.pending, "a successful send must not leave a duplicate Retry send row")
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
        XCTAssertEqual(delivery.pending, rejected)
        XCTAssertTrue(delivery.rejected)
        XCTAssertEqual(delivery.discardRejected(), "invalid")
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

    func testProfileValidationMatchesServerScalarAndASCIIBoundaries() {
        XCTAssertNil(ProfileValidation.error(username: " Jo_ ", displayName: String(repeating: "🪐", count: 64)))
        XCTAssertNil(ProfileValidation.error(username: String(repeating: "a", count: 32), displayName: " Name "))
        XCTAssertNotNil(ProfileValidation.error(username: "ab", displayName: "Name"))
        XCTAssertNotNil(ProfileValidation.error(username: String(repeating: "a", count: 33), displayName: "Name"))
        XCTAssertNotNil(ProfileValidation.error(username: "Kelvin", displayName: "Name"), "Only ASCII letters are normalized by the server")
        XCTAssertNotNil(ProfileValidation.error(username: "user", displayName: String(repeating: "🪐", count: 65)))
        XCTAssertNotNil(ProfileValidation.error(username: "user", displayName: String(repeating: "e\u{301}", count: 33)))
        XCTAssertNotNil(ProfileValidation.error(username: "user", displayName: " \n "))
        XCTAssertNotNil(ProfileValidation.error(username: "user", displayName: "Name\u{0007}"))
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
    func testDeafenAndMuteFollowPlatformIntent() async {
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
        XCTAssertTrue(voice.muted)
        await voice.setDeafened(false)
        XCTAssertTrue(voice.muted, "undeafen restores a pre-deafen explicit mute")
        await voice.setDeafened(false)
        XCTAssertTrue(voice.muted, "repeated undeafen must not clear that mute")

        await voice.setDeafened(true)
        await voice.setMuted(false)
        XCTAssertFalse(voice.muted, "explicit unmute overrides saved deafen intent")
        XCTAssertFalse(voice.deafened)
        await voice.setDeafened(false)
        XCTAssertFalse(voice.muted, "stale pre-deafen intent must not return")

        await voice.setMuted(true)
        await voice.setDeafened(false)
        XCTAssertTrue(voice.muted, "repeating an already-false deafen state must not restore stale intent")

        await voice.setDeafened(true)
        voice.phase = .connected
        voice.leaveImmediately()
        XCTAssertTrue(voice.muted)
        await voice.setDeafened(false)
        XCTAssertTrue(voice.muted, "transfer teardown retains pre-deafen mute intent")

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
