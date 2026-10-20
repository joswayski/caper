import Foundation

// Voice messages with transcripts (the shared voice-message contract): recorded
// from the composer, uploaded as an audio attachment with `voice: true`, a
// duration and a 64-byte peak waveform, then played inline with the server's
// transcript beneath. Everything here is free of Apple SDK types so the rules
// are unit-tested on every platform; the recorder, player and views live in
// `VoiceRecording.swift` and `VoiceMessageViews.swift`.

/// The constants and decisions every client shares.
public enum VoiceMessage {
    /// Reply previews, pins and forward excerpts name a voice note this way
    /// instead of its file name ("Voice message.m4a").
    public static let label = "Voice message"
    /// What this device uploads; the server renames and transcodes it on `complete`.
    public static let uploadFilename = "voice-message.m4a"
    public static let uploadContentType = "audio/mp4"
    /// The server's name for a completed voice note (until `complete` answers).
    public static let storedName = "Voice message.m4a"

    /// Shorter recordings are discarded without uploading, like a tap.
    public static let minimumDurationMs = 1_000
    /// Recording stops (and sends) at 10:00; the server accepts 1...600000.
    public static let maximumDurationMs = 600_000
    /// A touch released sooner records nothing and shows `holdHint`.
    public static let tapThreshold: TimeInterval = 0.3
    /// Sliding this far left (points) from the microphone cancels a touch recording.
    public static let cancelDistance: Double = 90

    public static let holdHint = "Hold to record"
    public static let releaseHint = "Release to send"
    public static let slideHint = "Slide to cancel"
    public static let permissionDenied = "Microphone access is needed to record voice messages"
    public static let recordingFailed = "Couldn’t record a voice message. Try again."
    public static let interrupted = "Recording was interrupted."

    /// AVAudioRecorder settings: AAC in MP4, mono, 48 kHz, about 64 kbps.
    public static let sampleRate = 48_000.0
    public static let bitRate = 64_000
    /// How often the recorder's meters are read for the live bars and the waveform.
    public static let meterInterval: TimeInterval = 0.05

    /// A recording long enough to send (and within the server's limit once clamped).
    public static func keeps(durationMs: Int) -> Bool { durationMs >= minimumDurationMs }

    /// A finished recording that may be uploaded: some bytes, 1 s to 10 min.
    public static func sendable(durationMs: Int, byteSize: Int) -> Bool {
        byteSize > 0 && (minimumDurationMs...maximumDurationMs).contains(durationMs)
    }

    /// Statuses that definitively refuse a message send (as the text path treats them).
    public static let refusalStatuses: Set<Int> = [400, 401, 403, 404, 409, 413, 422]

    /// After a failed profile-card send, the recording (and any uploaded file
    /// and frozen send) stays for Retry unless the server definitively refused it.
    public static func keepsForRetry(attemptedSend: Bool, status: Int?) -> Bool {
        guard attemptedSend, let status else { return true }
        return !refusalStatuses.contains(status)
    }

    /// The profile card's line for a voice message that didn't go:
    /// `unconfirmed` once the send may have reached the server.
    public static func retryNotice(unconfirmed: Bool) -> String {
        unconfirmed ? "Voice message delivery isn’t confirmed. Retry resends the same message." : "Voice message didn’t send."
    }

    /// The declared duration: the measured file length, else the clock, within the server's range.
    public static func uploadDuration(measuredMs: Int?, elapsedMs: Int) -> Int {
        let value = measuredMs.flatMap { $0 > 0 ? $0 : nil } ?? elapsedMs
        return min(maximumDurationMs, max(1, value))
    }

    /// What a touch release does: `cancelled` wins (the slide already cancelled),
    /// then a quick tap or a recording under a second shows the hint.
    public enum Release: Equatable, Sendable { case cancel, hint, send }

    public static func release(pressDuration: TimeInterval, recordedMs: Int, cancelled: Bool) -> Release {
        if cancelled { return .cancel }
        if pressDuration < tapThreshold || !keeps(durationMs: recordedMs) { return .hint }
        return .send
    }

    /// Sliding left (negative translation) far enough cancels.
    public static func cancelsBySliding(translation: Double) -> Bool { translation <= -cancelDistance }

    /// How far the slide hint has travelled toward cancelling, 0...1.
    public static func slideProgress(translation: Double) -> Double { min(1, max(0, -translation / cancelDistance)) }

    /// The quote line for a message without text: "Voice message" when it carries a voice note.
    public static func excerptFallback(hasAttachments: Bool, hasVoice: Bool) -> String {
        hasVoice ? label : hasAttachments ? "Attachment" : "Message"
    }
}

/// `m:ss` labels for recordings and playback.
public enum VoiceDuration {
    /// Whole elapsed seconds, as `m:ss` (minutes are not padded; 10:00 at the limit).
    public static func format(ms: Int) -> String {
        let seconds = max(0, ms) / 1000
        return "\(seconds / 60):\(String(format: "%02d", seconds % 60))"
    }

    /// Whole seconds still to play, rounded up so the label reaches 0:00 at the end.
    public static func remaining(durationMs: Int, elapsedMs: Int) -> String {
        let left = max(0, durationMs - max(0, elapsedMs))
        return format(ms: (left + 999) / 1000 * 1000)
    }

    /// The player's label: the total while idle, the time left while this note plays or is paused.
    public static func playerLabel(durationMs: Int?, elapsedMs: Int?) -> String {
        guard let durationMs, durationMs > 0 else { return elapsedMs.map { format(ms: $0) } ?? "0:00" }
        guard let elapsedMs else { return format(ms: durationMs) }
        return remaining(durationMs: durationMs, elapsedMs: elapsedMs)
    }
}

/// The 64 normalised peaks sent with a voice note, and how any length is drawn.
public enum VoiceWaveform {
    /// Clients send exactly this many slices.
    public static let sliceCount = 64
    /// The server accepts 1...256 bytes; anything else is treated as absent.
    public static let maximumBytes = 256

    /// `samples` are linear peak amplitudes (0...1) read at regular intervals
    /// over the whole recording. Each of `slices` equal time slices keeps its
    /// loudest sample, scaled linearly so the loudest slice is 255. Silence
    /// (or no samples) is all zeros.
    public static func build(_ samples: [Float], slices: Int = sliceCount) -> [UInt8] {
        guard slices > 0 else { return [] }
        let clean = samples.map { $0.isFinite ? max(0, $0) : 0 }
        guard !clean.isEmpty else { return Array(repeating: 0, count: slices) }
        let peaks: [Float] = (0..<slices).map { slice in
            let start = slice * clean.count / slices
            let end = max(start + 1, (slice + 1) * clean.count / slices)
            return clean[start..<min(end, clean.count)].max() ?? 0
        }
        guard let loudest = peaks.max(), loudest > 0 else { return Array(repeating: 0, count: slices) }
        return peaks.map { UInt8((min(1, $0 / loudest) * 255).rounded()) }
    }

    /// Standard base64 with padding.
    public static func encode(_ bytes: [UInt8]) -> String { Data(bytes).base64EncodedString() }

    /// The bytes of a received waveform; nil when absent, malformed or out of range.
    public static func decode(_ value: String?) -> [UInt8]? {
        guard let value, let data = Data(base64Encoded: value), (1...maximumBytes).contains(data.count) else { return nil }
        return [UInt8](data)
    }

    /// Bar heights (0...1) for `count` bars from a waveform of any length:
    /// each bar keeps the loudest byte it covers (stretched when there are
    /// fewer bytes than bars). No waveform draws a flat placeholder (all 0).
    public static func bars(_ bytes: [UInt8]?, count: Int) -> [Double] {
        guard count > 0 else { return [] }
        guard let bytes, !bytes.isEmpty else { return Array(repeating: 0, count: count) }
        return (0..<count).map { bar in
            let start = bar * bytes.count / count
            let end = max(start + 1, (bar + 1) * bytes.count / count)
            return Double(bytes[start..<min(end, bytes.count)].max() ?? 0) / 255
        }
    }

    /// Linear amplitude from a recorder's dBFS reading (`peakPower`).
    public static func amplitude(decibels: Float) -> Float {
        guard decibels.isFinite else { return 0 }
        return min(1, max(0, Float(pow(10, Double(decibels) / 20))))
    }

    /// Live bar height (0...1) from `averagePower`: `floor` dB and below is
    /// silent, 0 dB is full, so ordinary speech is clearly visible.
    public static func meterLevel(decibels: Float, floor: Float = -50) -> Float {
        guard decibels.isFinite, floor < 0 else { return 0 }
        return min(1, max(0, (decibels - floor) / -floor))
    }

    /// The live bars shown while recording: the latest `count` levels,
    /// padded with silence on the left so new levels enter at the right.
    public static func liveBars(_ levels: [Float], count: Int) -> [Double] {
        guard count > 0 else { return [] }
        let recent = levels.suffix(count).map { Double($0.isFinite ? min(1, max(0, $0)) : 0) }
        return Array(repeating: 0, count: count - recent.count) + recent
    }

    /// How many bars of `width` and `gap` fit in `available` points (at least one).
    public static func barCount(available: Double, width: Double, gap: Double) -> Int {
        guard available.isFinite, width > 0, gap >= 0 else { return 1 }
        return max(1, Int(((available + gap) / (width + gap)).rounded(.down)))
    }
}

/// The server's transcript of a voice note. Unknown statuses decode as
/// absent (nil), so new states never show broken UI.
public enum VoiceTranscript: Codable, Equatable, Sendable {
    case pending
    case ready(String)
    case failed

    /// What the transcript block shows.
    public enum Display: Equatable, Sendable {
        /// Failed, absent, unknown or empty: no transcript UI at all.
        case hidden
        /// "Transcribing…"
        case pending
        case text(String)
    }

    public static let pendingLabel = "Transcribing…"
    public static let expandLabel = "View transcript"
    public static let collapseLabel = "Hide transcript"
    /// Lines shown before "View transcript".
    public static let clampedLines = 3

    public init?(status: String?, text: String?) {
        switch status {
        case "pending": self = .pending
        case "ready": self = .ready(text ?? "")
        case "failed": self = .failed
        default: return nil
        }
    }

    public var display: Display {
        switch self {
        case .pending: return .pending
        case .failed: return .hidden
        case .ready(let text):
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? .hidden : .text(trimmed)
        }
    }

    private enum CodingKeys: String, CodingKey { case status, text }

    /// Throws for an unknown or malformed status; attachments decode it with `try?`.
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let status = try? values.decodeIfPresent(String.self, forKey: .status)
        let text = try? values.decodeIfPresent(String.self, forKey: .text)
        guard let value = VoiceTranscript(status: status, text: text) else {
            throw DecodingError.dataCorruptedError(forKey: .status, in: values, debugDescription: "Unknown transcript status")
        }
        self = value
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .pending: try values.encode("pending", forKey: .status)
        case .failed: try values.encode("failed", forKey: .status)
        case .ready(let text):
            try values.encode("ready", forKey: .status)
            try values.encode(text, forKey: .text)
        }
    }
}

/// A finished recording ready to upload.
public struct VoiceRecording: Equatable, Sendable {
    public let fileURL: URL
    public let durationMs: Int
    /// `VoiceWaveform.build` output (64 normalised peaks).
    public let waveform: [UInt8]

    public init(fileURL: URL, durationMs: Int, waveform: [UInt8]) {
        self.fileURL = fileURL; self.durationMs = durationMs; self.waveform = waveform
    }
}

/// Playback position decisions for the inline player.
public enum VoicePlaybackPosition {
    /// The seek fraction (0...1) for a touch at `x` across a waveform `width` wide.
    public static func fraction(x: Double, width: Double) -> Double {
        guard width > 0, x.isFinite else { return 0 }
        return min(1, max(0, x / width))
    }

    /// Played fraction (0...1) of a note `durationMs` long, `elapsedMs` in.
    public static func progress(elapsedMs: Int, durationMs: Int?) -> Double {
        guard let durationMs, durationMs > 0 else { return 0 }
        return min(1, max(0, Double(elapsedMs) / Double(durationMs)))
    }

    /// Bars at or before the playhead draw in the played colour.
    public static func playedBars(progress: Double, count: Int) -> Int {
        guard count > 0, progress.isFinite else { return 0 }
        return min(count, max(0, Int((progress * Double(count)).rounded(.up))))
    }
}
