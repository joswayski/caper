#if os(iOS)
import AVFoundation
import CaperRTCBridgeIOS
import Observation

/// Explicit local comparison. VoiceClient keeps publication gated through
/// recording and replay; the system audio route remains user-controlled.
@MainActor @Observable
final class IOSMicrophoneTest {
    private(set) var recording = false
    private(set) var hasRecording = false
    var error: String?
    private var voice: VoiceClient?
    private var attempt: Int?
    private var timer: Task<Void, Never>?
    private var audioObservers: [NSObjectProtocol] = []
    private var generation = 0
    private var playbackGeneration = 0
    private var player: AVAudioPlayerNode?
    private var engine: AVAudioEngine?
    private var playbackGain = 100
    private let naturalURL = FileManager.default.temporaryDirectory.appendingPathComponent("caper-ios-mic-\(UUID().uuidString).caf")
    private var enhancedURL: URL { naturalURL.deletingPathExtension().appendingPathExtension("enhanced.caf") }

    func start(voice: VoiceClient) async {
        guard !recording else { return }
        close()
        let next = generation
        error = nil; self.voice = voice
        let permitted: Bool
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: permitted = true
        case .notDetermined: permitted = await AVCaptureDevice.requestAccess(for: .audio)
        default: permitted = false
        }
        guard next == generation else { return }
        guard permitted else { error = "Microphone access is required for a local test."; return }
        let ready = await voice.prepareMicrophoneDenoise()
        guard next == generation else { voice.releaseIdleAudioPreparation(); return }
        guard ready else { error = "On-device noise suppression could not start. No microphone audio was captured."; return }
        guard let attempt = voice.beginMicrophoneComparison() else {
            error = "Could not start recording from this microphone."
            return
        }
        self.attempt = attempt
        recording = true
        let center = NotificationCenter.default
        audioObservers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in
                guard let self, self.recording || self.hasRecording else { return }
                self.close(); self.error = "Audio route changed. Start the local test again."
            }
        })
        audioObservers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
            guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began else { return }
            Task { @MainActor in
                guard let self, self.recording || self.hasRecording else { return }
                self.close(); self.error = "Audio was interrupted. Start the local test again."
            }
        })
        timer = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(30)) } catch { return }
            self?.stopRecording()
        }
    }

    func stopRecording() {
        guard recording else { return }
        recording = false; timer?.cancel(); timer = nil
        playbackGain = voice?.outputGain ?? 100
        guard let voice, let attempt, let samples = voice.finishMicrophoneRecording(generation: attempt),
              samples.natural.count == samples.enhanced.count,
              Double(samples.natural.count) > 0.2 * samples.sampleRate * 2 else {
            error = "Record a little longer to hear your voice."
            return
        }
        do {
            try Self.write(samples.natural, rate: samples.sampleRate, to: naturalURL)
            try Self.write(samples.enhanced, rate: samples.sampleRate, to: enhancedURL)
            hasRecording = true
        } catch {
            clearFiles(); error = "Could not save the local comparison."
        }
    }

    private static func write(_ pcm: Data, rate: Double, to url: URL) throws {
        guard let format = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: rate, channels: 1, interleaved: true),
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(pcm.count / 2)),
              let channel = buffer.int16ChannelData?.pointee else { throw CocoaError(.fileWriteUnknown) }
        pcm.copyBytes(to: UnsafeMutableRawBufferPointer(start: channel, count: pcm.count))
        buffer.frameLength = AVAudioFrameCount(pcm.count / 2)
        try AVAudioFile(forWriting: url, settings: format.settings,
                        commonFormat: .pcmFormatInt16, interleaved: true).write(from: buffer)
    }

    func play(enhanced: Bool) {
        guard hasRecording && !recording else { return }
        stopPlayback(); error = nil
        do {
            let file = try AVAudioFile(forReading: enhanced ? enhancedURL : naturalURL)
            let engine = AVAudioEngine(), player = AVAudioPlayerNode()
            let gain = min(200, max(0, playbackGain))
            player.volume = gain == 0 ? 0 : 1
            let volume = AVAudioUnitEQ(numberOfBands: 1)
            volume.bands[0].bypass = true
            volume.globalGain = gain == 0 ? -96 : Float(20 * log10(Double(gain) / 100))
            engine.attach(player); engine.attach(volume)
            engine.connect(player, to: volume, format: file.processingFormat)
            engine.connect(volume, to: engine.mainMixerNode, format: file.processingFormat)
            let playback = playbackGeneration
            player.scheduleFile(file, at: nil, completionCallbackType: .dataPlayedBack) { [weak self, weak player] _ in
                Task { @MainActor in
                    guard let self, let player, self.playbackGeneration == playback, self.player === player else { return }
                    self.stopPlayback()
                }
            }
            try engine.start()
            self.engine = engine; self.player = player
            player.play()
        } catch { stopPlayback(); error = "Could not play the local recording." }
    }

    func stopPlayback() {
        playbackGeneration += 1
        player?.stop(); engine?.stop()
        player = nil; engine = nil
    }

    func close() {
        generation += 1
        let center = NotificationCenter.default
        audioObservers.forEach(center.removeObserver); audioObservers = []
        timer?.cancel(); timer = nil
        recording = false; stopPlayback()
        if let voice, let attempt { voice.endMicrophoneComparison(generation: attempt) }
        voice?.releaseIdleAudioPreparation()
        voice = nil; attempt = nil; hasRecording = false
        clearFiles()
    }

    private func clearFiles() {
        try? FileManager.default.removeItem(at: naturalURL)
        try? FileManager.default.removeItem(at: enhancedURL)
    }
}
#endif
