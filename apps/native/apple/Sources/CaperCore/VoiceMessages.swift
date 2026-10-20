import Foundation

// Voice messages with transcripts (the shared voice-message contract): recorded
// from the composer, uploaded as an audio attachment with `voice: true`, a
// duration and a 64-byte loudness waveform, then played inline with the server's
// transcript beneath. By default a finished recording is staged in the
// composer beside any files and goes with the next Send; the "Send voice
// messages as soon as I stop recording" setting sends it on its own instead.
// Everything here is free of Apple SDK types so the rules are unit-tested on
// every platform; the recorder, player and views live in
// `VoiceMessageAudio.swift` and `VoiceMessageViews.swift`.

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
    /// A touch released sooner was a tap: recording carries on hands-free
    /// until Stop. Held longer, releasing stops.
    public static let tapThreshold: TimeInterval = 0.3
    /// Sliding this far left (points) from the microphone cancels a held recording.
    public static let cancelDistance: Double = 90

    public static let slideHint = "Slide to cancel"
    public static let stopLabel = "Stop recording"
    public static let discardLabel = "Discard voice message"
    public static let recordLabel = "Record voice message"
    /// The microphone's tooltip/label while a voice message is already staged.
    public static let oneVoicePerMessage = "Remove the voice message to record another"
    public static let permissionDenied = "Microphone access is needed to record voice messages"
    public static let recordingFailed = "Couldn’t record a voice message. Try again."
    public static let interrupted = "Recording was interrupted."

    /// The per-device setting (off by default) and where it is stored.
    public static let sendOnStopTitle = "Send voice messages as soon as I stop recording"
    public static let sendOnStopDetail = "Otherwise a recording waits in the composer, with your text and files, until you press Send."
    public static let sendOnStopKey = "caper.voiceMessages.sendOnStop"

    /// The stored preference; anything but a stored `true` means off.
    public static func sendsOnStop(stored value: Any?) -> Bool { (value as? Bool) ?? false }

    /// Shown above a held recording.
    public static func releaseHint(sendsOnStop: Bool) -> String {
        sendsOnStop ? "Release to send" : "Release to add to your message"
    }

    /// AVAudioRecorder settings: AAC in MP4, mono, 48 kHz, about 64 kbps.
    public static let sampleRate = 48_000.0
    public static let bitRate = 64_000
    /// The recorder's meters are read at 40 Hz for the live bars and the waveform.
    public static let meterInterval: TimeInterval = 0.025
    /// A new live bar enters at the right this often, scrolling at a constant rate.
    public static let liveBarInterval: TimeInterval = 0.07

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

    /// What lifting a finger off the microphone does: `cancelled` wins (the
    /// slide already cancelled); a quick tap keeps recording hands-free until
    /// Stop; a hold stops, and a recording under a second is discarded.
    public enum Release: Equatable, Sendable { case cancel, handsFree, discard, stop }

    public static func release(pressDuration: TimeInterval, recordedMs: Int, cancelled: Bool) -> Release {
        if cancelled { return .cancel }
        if pressDuration < tapThreshold { return .handsFree }
        return keeps(durationMs: recordedMs) ? .stop : .discard
    }

    /// What a stopped recording (Stop, a released hold or the 10:00 limit)
    /// becomes: staged in the composer by default, sent alone with the setting on.
    public enum Finish: Equatable, Sendable { case stage, send }

    public static func finish(sendsOnStop: Bool) -> Finish { sendsOnStop ? .send : .stage }

    /// Whether the microphone can record into a composer that already holds
    /// `staged` files, `voiceStaged` of them a voice note: one voice note
    /// per message, within the attachment limit.
    public static func canStage(staged: Int, voiceStaged: Bool, limit: Int = 10) -> Bool {
        !voiceStaged && staged < limit
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

/// The 64-byte loudness waveform sent with a voice note, how any length is
/// drawn, and the live recording bars.
public enum VoiceWaveform {
    /// Clients send exactly this many slices.
    public static let sliceCount = 64
    /// The server accepts 1...256 bytes; anything else is treated as absent.
    public static let maximumBytes = 256
    /// The perceptual scale: this many dB below full scale and quieter is 0.
    public static let floorDecibels: Float = -50

    /// The stored waveform from loudness readings spread evenly over the whole
    /// recording (`rms` are linear RMS amplitudes, 0...1, any count): each
    /// of `slices` equal time slices takes the RMS of its readings, mapped to
    /// dB in [-50, 0] → [0, 1], smoothed with a 3-tap moving average, then
    /// normalised so the loudest slice is 255. Silence is all zeros (drawn
    /// as a flat line at the minimum bar height).
    public static func build(rms: [Float], slices: Int = sliceCount) -> [UInt8] {
        guard slices > 0 else { return [] }
        let clean = rms.map { $0.isFinite ? max(0, $0) : 0 }
        guard !clean.isEmpty else { return Array(repeating: 0, count: slices) }
        let sliceRMS: [Float] = (0..<slices).map { slice in
            let start = slice * clean.count / slices
            let end = max(start + 1, (slice + 1) * clean.count / slices)
            let values = clean[start..<min(end, clean.count)]
            return (values.reduce(0) { $0 + $1 * $1 } / Float(values.count)).squareRoot()
        }
        return normalised(sliceRMS.map(level(rms:)))
    }

    /// 0...1 on the perceptual scale for a linear RMS amplitude.
    public static func level(rms: Float) -> Float {
        guard rms.isFinite, rms > 0 else { return 0 }
        return meterLevel(decibels: 20 * log10(rms))
    }

    /// 3-tap moving average (the ends average what they have), then scaled
    /// so the loudest value is 255.
    public static func normalised(_ levels: [Float]) -> [UInt8] {
        guard !levels.isEmpty else { return [] }
        let smoothed: [Float] = levels.indices.map { index in
            let window = levels[max(0, index - 1)...min(levels.count - 1, index + 1)]
            return window.reduce(0, +) / Float(window.count)
        }
        guard let loudest = smoothed.max(), loudest > 0 else { return Array(repeating: 0, count: levels.count) }
        return smoothed.map { UInt8((min(1, max(0, $0 / loudest)) * 255).rounded()) }
    }

    /// Accumulates squared samples into `slices` equal time slices of a
    /// recording `totalFrames` long, for the per-slice RMS of a decoded file.
    public struct RMSAccumulator: Sendable {
        public let totalFrames: Int
        public let slices: Int
        private var sums: [Double]
        private var counts: [Int]

        public init(totalFrames: Int, slices: Int = VoiceWaveform.sliceCount) {
            self.totalFrames = max(1, totalFrames)
            self.slices = max(1, slices)
            sums = Array(repeating: 0, count: self.slices)
            counts = Array(repeating: 0, count: self.slices)
        }

        /// Adds consecutive samples starting at frame `start`.
        public mutating func add(_ samples: UnsafeBufferPointer<Float>, start: Int) {
            for (offset, sample) in samples.enumerated() {
                let frame = start + offset
                let slice = min(slices - 1, max(0, frame * slices / totalFrames))
                let value = sample.isFinite ? Double(sample) : 0
                sums[slice] += value * value
                counts[slice] += 1
            }
        }

        public mutating func add(_ samples: [Float], start: Int) {
            samples.withUnsafeBufferPointer { add($0, start: start) }
        }

        /// Linear RMS of each slice (0 where nothing was read).
        public var rms: [Float] {
            zip(sums, counts).map { sum, count in count > 0 ? Float((sum / Double(count)).squareRoot()) : 0 }
        }
    }

    /// Standard base64 with padding.
    public static func encode(_ bytes: [UInt8]) -> String { Data(bytes).base64EncodedString() }

    /// The bytes of a received waveform; nil when absent, malformed or out of range.
    public static func decode(_ value: String?) -> [UInt8]? {
        guard let value, let data = Data(base64Encoded: value), (1...maximumBytes).contains(data.count) else { return nil }
        return [UInt8](data)
    }

    /// Bar heights (0...1) for `count` bars from a waveform of any length,
    /// linearly interpolated across its values (first and last bars land on
    /// the first and last values). No waveform draws a flat line (all 0).
    public static func resample(_ bytes: [UInt8]?, count: Int) -> [Double] {
        guard count > 0 else { return [] }
        guard let bytes, !bytes.isEmpty else { return Array(repeating: 0, count: count) }
        let values = bytes.map { Double($0) / 255 }
        guard values.count > 1 else { return Array(repeating: values[0], count: count) }
        guard count > 1 else { return [values.max() ?? 0] }
        return (0..<count).map { bar in
            let position = Double(bar) * Double(values.count - 1) / Double(count - 1)
            let lower = Int(position.rounded(.down))
            let upper = min(values.count - 1, lower + 1)
            let fraction = position - Double(lower)
            return values[lower] + (values[upper] - values[lower]) * fraction
        }
    }

    /// Linear amplitude from a recorder's dBFS reading (`averagePower`).
    public static func amplitude(decibels: Float) -> Float {
        guard decibels.isFinite else { return 0 }
        return min(1, max(0, Float(pow(10, Double(decibels) / 20))))
    }

    /// 0...1 from a dBFS reading: `floor` dB and below is silent, 0 dB is
    /// full, so ordinary speech is clearly visible.
    public static func meterLevel(decibels: Float, floor: Float = floorDecibels) -> Float {
        guard decibels.isFinite, floor < 0 else { return 0 }
        return min(1, max(0, (decibels - floor) / -floor))
    }

    /// Attack/release envelope for the live bars: rises quickly toward a
    /// louder reading and falls back slowly, so bars don't jitter. `dt` is
    /// the time since the previous reading.
    public static func follow(_ previous: Float, toward target: Float, dt: Double,
                              attack: Double = 0.03, release: Double = 0.2) -> Float {
        guard target.isFinite, previous.isFinite, dt > 0 else { return target.isFinite ? target : 0 }
        let constant = target > previous ? attack : release
        let blend = Float(1 - exp(-dt / max(0.001, constant)))
        return previous + (target - previous) * blend
    }

    /// The live bars shown while recording: the latest `count` levels,
    /// padded with silence on the left so new levels enter at the right.
    public static func liveBars(_ levels: [Float], count: Int) -> [Double] {
        guard count > 0 else { return [] }
        let recent = levels.suffix(count).map { Double($0.isFinite ? min(1, max(0, $0)) : 0) }
        return Array(repeating: 0, count: count - recent.count) + recent
    }

    /// How far (0...1 of one bar step) the live bars have scrolled left since
    /// the newest bar entered, for a constant right-to-left motion.
    public static func scrollFraction(sinceLastBar elapsed: TimeInterval, interval: TimeInterval = VoiceMessage.liveBarInterval) -> Double {
        guard interval > 0, elapsed.isFinite else { return 0 }
        return min(1, max(0, elapsed / interval))
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

    /// How much of a bar is played (0...1): bars before the playhead are
    /// full, the boundary bar is partly filled, later bars are empty.
    public static func playedFill(progress: Double, bar: Int, count: Int) -> Double {
        guard count > 0, progress.isFinite else { return 0 }
        return min(1, max(0, progress * Double(count) - Double(bar)))
    }
}
