import AVFoundation
import Foundation
import Observation
#if os(iOS)
import UIKit
#endif

// Recording and playing voice messages (the shared voice-message contract). The rules are
// in `VoiceMessages.swift`; the views are in `VoiceMessageViews.swift`.

/// Which composer a recording belongs to: one conversation's channel
/// composer, one thread's, or a profile card's message to one person.
/// Opening another conversation or thread changes it, which discards the
/// recording.
struct VoiceComposerID: Hashable {
    let chat: ObjectIdentifier?
    let channelID: String?
    let threadRootID: String?
    let inThread: Bool
    /// The person a profile card's composer messages.
    let profilePersonID: String?

    @MainActor init(chat: ChatModel, inThread: Bool) {
        self.chat = ObjectIdentifier(chat)
        channelID = chat.channelID
        threadRootID = inThread ? chat.threadRootID : nil
        self.inThread = inThread
        profilePersonID = nil
    }

    init(profilePersonID: String) {
        chat = nil; channelID = nil; threadRootID = nil; inThread = false
        self.profilePersonID = profilePersonID
    }
}

/// The app's one voice-message recorder. AVAudioRecorder writes AAC in MP4
/// (mono, 48 kHz, ~64 kbps) to a private staging file, with metering for the
/// live bars and the uploaded waveform. It records from the default input and
/// never touches a call's audio: on iOS an active call's play-and-record
/// session is shared as it is, and otherwise a session is configured only for
/// the recording and released afterwards; on macOS the call's HAL route is
/// separate from the recorder's default-device input.
@MainActor @Observable
final class VoiceMessageRecorder {
    static let shared = VoiceMessageRecorder()

    enum Phase: Equatable { case idle, starting, recording }

    private(set) var phase: Phase = .idle
    /// The composer recording (or asking for permission).
    private(set) var owner: VoiceComposerID?
    private(set) var elapsedMs = 0
    /// Recent live input levels (0...1), newest last.
    private(set) var levels: [Float] = []
    /// A short inline message for one composer: a denied permission, a failure or "Hold to record".
    private(set) var notice: (owner: VoiceComposerID, text: String)?
    /// How far a touch recording's finger has slid left (points, 0 or negative).
    var slide: Double = 0

    @ObservationIgnored private var recorder: AVAudioRecorder?
    @ObservationIgnored private var fileURL: URL?
    @ObservationIgnored private var peaks: [Float] = []
    @ObservationIgnored private var meterTask: Task<Void, Never>?
    @ObservationIgnored private var noticeTask: Task<Void, Never>?
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var onLimit: (() -> Void)?
    /// iOS: this recorder configured the audio session (no call was using it).
    @ObservationIgnored private var ownsSession = false
    @ObservationIgnored private var interruptionObserver: NSObjectProtocol?

    private init() {}

    func isActive(_ owner: VoiceComposerID) -> Bool { phase != .idle && self.owner == owner }
    func isRecording(_ owner: VoiceComposerID) -> Bool { phase == .recording && self.owner == owner }
    func notice(for owner: VoiceComposerID) -> String? { notice?.owner == owner ? notice?.text : nil }

    /// Shows `text` under the composer for a few seconds.
    func show(_ text: String, for owner: VoiceComposerID, seconds: Double = 3) {
        notice = (owner, text)
        noticeTask?.cancel()
        noticeTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled, let self, self.notice?.owner == owner, self.notice?.text == text else { return }
            self.notice = nil
        }
    }

    func clearNotice(for owner: VoiceComposerID) {
        if notice?.owner == owner { notice = nil }
    }

    /// Asks for the microphone on first use, then starts recording. Returns
    /// false when denied or failed (the composer shows the notice) or when
    /// cancelled meanwhile. `onLimit` runs once the 10-minute limit is reached;
    /// the caller then finishes and sends.
    @discardableResult
    func start(for owner: VoiceComposerID, onLimit: @escaping () -> Void) async -> Bool {
        cancel()
        generation += 1
        let attempt = generation
        self.owner = owner
        phase = .starting
        clearNotice(for: owner)
        // Stop any playing voice message when recording starts.
        VoiceMessagePlayer.shared.stop()
        let permitted: Bool
        var asked = false
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: permitted = true
        case .notDetermined:
            asked = true
            permitted = await AVCaptureDevice.requestAccess(for: .audio)
        default: permitted = false
        }
        guard permitted else {
            if attempt == generation { reset() }
            show(VoiceMessage.permissionDenied, for: owner, seconds: 6)
            return false
        }
        guard attempt == generation else {
            // Released while the system asked (a touch hold): the next hold records.
            if asked { show(VoiceMessage.holdHint, for: owner) }
            return false
        }
        do {
            try beginSession()
            let directory = try AttachmentStaging.newDirectory()
            let url = directory.appendingPathComponent(VoiceMessage.uploadFilename)
            fileURL = url
            let settings: [String: Any] = [
                AVFormatIDKey: Int(kAudioFormatMPEG4AAC),
                AVSampleRateKey: VoiceMessage.sampleRate,
                AVNumberOfChannelsKey: 1,
                AVEncoderBitRateKey: VoiceMessage.bitRate,
                AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue,
            ]
            let recorder = try AVAudioRecorder(url: url, settings: settings)
            recorder.isMeteringEnabled = true
            // The recorder itself also stops at the limit, so a stalled UI never records longer.
            guard recorder.prepareToRecord(), recorder.record(forDuration: Double(VoiceMessage.maximumDurationMs) / 1000) else {
                throw CocoaError(.fileWriteUnknown)
            }
            self.recorder = recorder
        } catch {
            discardFile()
            reset()
            show(VoiceMessage.recordingFailed, for: owner)
            return false
        }
        self.onLimit = onLimit
        peaks = []; levels = []; elapsedMs = 0
        phase = .recording
        observeInterruptions(owner: owner)
        let started = Date()
        meterTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(Int(VoiceMessage.meterInterval * 1000)))
                guard !Task.isCancelled, let self, self.generation == attempt else { return }
                self.sample(started: started)
            }
        }
        return true
    }

    private func sample(started: Date) {
        guard phase == .recording, let recorder else { return }
        elapsedMs = min(VoiceMessage.maximumDurationMs, Int(Date().timeIntervalSince(started) * 1000))
        if recorder.isRecording {
            recorder.updateMeters()
            peaks.append(VoiceWaveform.amplitude(decibels: recorder.peakPower(forChannel: 0)))
            levels.append(VoiceWaveform.meterLevel(decibels: recorder.averagePower(forChannel: 0)))
            if levels.count > 160 { levels.removeFirst(levels.count - 160) }
        }
        if elapsedMs >= VoiceMessage.maximumDurationMs || !recorder.isRecording {
            let limit = onLimit
            onLimit = nil
            limit?()
        }
    }

    /// Stops and returns the recording, or nil when it was under a second
    /// (discarded without uploading) or could not be read.
    func finish() async -> VoiceRecording? {
        guard phase == .recording, let recorder, let url = fileURL else { cancel(); return nil }
        let elapsed = elapsedMs
        recorder.stop()
        let waveform = VoiceWaveform.build(peaks)
        fileURL = nil
        reset()
        let measured = await AttachmentPreparer.durationMs(AVURLAsset(url: url))
        let duration = VoiceMessage.uploadDuration(measuredMs: measured, elapsedMs: elapsed)
        guard VoiceMessage.keeps(durationMs: duration) else {
            AttachmentStaging.remove(url)
            return nil
        }
        return VoiceRecording(fileURL: url, durationMs: duration, waveform: waveform)
    }

    /// Discards the recording (or a pending permission request) without uploading.
    func cancel() {
        generation += 1
        recorder?.stop()
        discardFile()
        reset()
    }

    private func discardFile() {
        recorder?.deleteRecording()
        if let fileURL { AttachmentStaging.remove(fileURL) }
        fileURL = nil
    }

    private func reset() {
        meterTask?.cancel(); meterTask = nil
        recorder = nil; onLimit = nil
        phase = .idle; owner = nil
        elapsedMs = 0; levels = []; peaks = []; slide = 0
        stopObservingInterruptions()
        endSession()
    }

    // MARK: Audio session

    private func beginSession() throws {
        #if os(iOS)
        let session = AVAudioSession.sharedInstance()
        if session.category == .playAndRecord {
            // A voice call (or microphone test) already records: share its
            // session unchanged so the call keeps its route and processing.
            ownsSession = false
        } else {
            try session.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker, .allowBluetooth])
            try session.setActive(true)
            ownsSession = true
        }
        #endif
    }

    private func endSession() {
        #if os(iOS)
        guard ownsSession else { return }
        ownsSession = false
        let session = AVAudioSession.sharedInstance()
        // A call joined while recording owns the session now (voice chat mode).
        guard session.mode != .voiceChat else { return }
        try? session.setActive(false, options: .notifyOthersOnDeactivation)
        // Interface sounds mix with other apps again (see CaperEffects).
        try? session.setCategory(.ambient)
        #endif
    }

    private func observeInterruptions(owner: VoiceComposerID) {
        #if os(iOS)
        interruptionObserver = NotificationCenter.default.addObserver(
            forName: AVAudioSession.interruptionNotification, object: nil, queue: .main
        ) { [weak self] note in
            guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began else { return }
            MainActor.assumeIsolated {
                guard let self, self.owner == owner, self.phase == .recording else { return }
                self.cancel()
                self.show(VoiceMessage.interrupted, for: owner)
            }
        }
        #endif
    }

    private func stopObservingInterruptions() {
        #if os(iOS)
        if let interruptionObserver { NotificationCenter.default.removeObserver(interruptionObserver) }
        interruptionObserver = nil
        #endif
    }

    /// A short tap of feedback where the device has a haptic engine.
    static func haptic() {
        #if os(iOS)
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        #endif
    }
}

/// The app's one voice-message player: starting another note (or recording)
/// stops the current one, and reaching the end returns to idle. AVPlayer
/// streams the signed URL (the server stores AAC in MP4), refreshed first when
/// it is about to expire; the pending row plays its local copy.
@MainActor @Observable
final class VoiceMessagePlayer {
    static let shared = VoiceMessagePlayer()

    /// The note playing or paused, if any.
    private(set) var attachmentID: String?
    private(set) var playing = false
    private(set) var elapsedMs = 0

    @ObservationIgnored private var player: AVPlayer?
    @ObservationIgnored private var timeObserver: Any?
    @ObservationIgnored private var endObserver: NSObjectProtocol?
    @ObservationIgnored private var request = 0
    @ObservationIgnored private var durationMs: Int?
    @ObservationIgnored private var pendingSeekMs: Int?
    /// iOS: this player switched the session to playback.
    @ObservationIgnored private var ownsSession = false

    private init() {}

    func isCurrent(_ attachment: ChatAttachment) -> Bool { attachmentID == attachment.id }
    func isPlaying(_ attachment: ChatAttachment) -> Bool { playing && attachmentID == attachment.id }

    /// Elapsed time of `attachment` when it is the current note.
    func elapsed(for attachment: ChatAttachment) -> Int? { attachmentID == attachment.id ? elapsedMs : nil }

    func toggle(_ attachment: ChatAttachment, chat: ChatModel?) {
        if attachmentID == attachment.id {
            if playing {
                player?.pause()
                playing = false
            } else if let player {
                beginSession()
                player.play()
                playing = true
            }
            return
        }
        start(attachment, chat: chat, atMs: 0)
    }

    /// Moves the playhead; a note that isn't current starts playing from there.
    func seek(_ attachment: ChatAttachment, chat: ChatModel?, fraction: Double) {
        let total = attachment.durationMs ?? durationMs ?? 0
        let target = Int((min(1, max(0, fraction)) * Double(total)).rounded())
        guard attachmentID == attachment.id else {
            start(attachment, chat: chat, atMs: target)
            return
        }
        elapsedMs = target
        guard let player else { pendingSeekMs = target; return }
        player.seek(to: CMTime(value: CMTimeValue(target), timescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero)
    }

    func stop() {
        request += 1
        player?.pause()
        if let timeObserver { player?.removeTimeObserver(timeObserver) }
        if let endObserver { NotificationCenter.default.removeObserver(endObserver) }
        timeObserver = nil; endObserver = nil; player = nil
        attachmentID = nil; playing = false; elapsedMs = 0; durationMs = nil; pendingSeekMs = nil
        endSession()
    }

    private func start(_ attachment: ChatAttachment, chat: ChatModel?, atMs: Int) {
        // Never over a recording; the recorder stops playback when it starts.
        guard VoiceMessageRecorder.shared.phase == .idle else { return }
        stop()
        request += 1
        let attempt = request
        attachmentID = attachment.id
        durationMs = attachment.durationMs
        elapsedMs = atMs
        pendingSeekMs = atMs > 0 ? atMs : nil
        playing = true
        Task { @MainActor [weak self] in
            let url: URL?
            if let chat { url = await chat.currentURL(for: attachment) } else { url = attachment.url.flatMap { URL(string: $0) } }
            guard let self, self.request == attempt else { return }
            guard let url else { self.stop(); return }
            let player = AVPlayer(url: url)
            self.player = player
            self.timeObserver = player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 20), queue: .main) { [weak self] time in
                MainActor.assumeIsolated {
                    guard let self, self.request == attempt, time.seconds.isFinite else { return }
                    self.elapsedMs = max(0, Int(time.seconds * 1000))
                }
            }
            self.endObserver = NotificationCenter.default.addObserver(
                forName: AVPlayerItem.didPlayToEndTimeNotification, object: player.currentItem, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self, self.request == attempt else { return }
                    self.stop()
                }
            }
            if let seek = self.pendingSeekMs {
                self.pendingSeekMs = nil
                _ = await player.seek(to: CMTime(value: CMTimeValue(seek), timescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero)
                guard self.request == attempt else { return }
            }
            self.beginSession()
            if self.playing { player.play() }
        }
    }

    private func beginSession() {
        #if os(iOS)
        let session = AVAudioSession.sharedInstance()
        // A call (or recording) keeps its own session; otherwise play like media.
        guard session.category != .playAndRecord, session.category != .record else { return }
        if session.category != .playback { try? session.setCategory(.playback) }
        try? session.setActive(true)
        ownsSession = true
        #endif
    }

    private func endSession() {
        #if os(iOS)
        guard ownsSession else { return }
        ownsSession = false
        let session = AVAudioSession.sharedInstance()
        guard session.category == .playback else { return }
        try? session.setActive(false, options: .notifyOthersOnDeactivation)
        try? session.setCategory(.ambient)
        #endif
    }
}
