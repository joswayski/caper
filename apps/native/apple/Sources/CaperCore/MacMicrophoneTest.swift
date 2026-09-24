#if os(macOS)
import AVFoundation
import AudioToolbox
import Observation

/// Local comparison only. Neither the recorded samples nor this effect graph enter WebRTC.
@MainActor @Observable
final class MacMicrophoneTest {
    private(set) var recording = false
    private(set) var hasRecording = false
    private(set) var level: Float = 0
    var error: String?
    var strength = 25

    private var recorder: AVAudioRecorder?
    private var engine: AVAudioEngine?
    private var player: AVAudioPlayerNode?
    private var timer: Task<Void, Never>?
    private var generation = 0
    private var playbackGeneration = 0
    private let fileURL: URL

    init(fileURL: URL = FileManager.default.temporaryDirectory.appendingPathComponent("caper-mic-test-\(UUID().uuidString).caf")) {
        self.fileURL = fileURL
    }

    func start() async {
        guard !recording else { return }
        generation += 1
        let attempt = generation
        stopPlayback()
        timer?.cancel()
        recorder?.stop()
        recorder = nil
        try? FileManager.default.removeItem(at: fileURL)
        hasRecording = false
        error = nil
        let permitted: Bool
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: permitted = true
        case .notDetermined: permitted = await AVCaptureDevice.requestAccess(for: .audio)
        default: permitted = false
        }
        guard attempt == generation else { return }
        guard permitted else { error = "Microphone access is required for a local test."; return }
        do {
            let recorder = try AVAudioRecorder(url: fileURL, settings: [
                AVFormatIDKey: Int(kAudioFormatLinearPCM),
                AVSampleRateKey: 44_100,
                AVNumberOfChannelsKey: 1,
                AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsFloatKey: false
            ])
            guard recorder.prepareToRecord(), recorder.record(forDuration: 30) else {
                error = "Could not start recording."; return
            }
            recorder.isMeteringEnabled = true
            self.recorder = recorder
            recording = true
            timer = Task { [weak self] in
                for _ in 0..<300 {
                    do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
                    guard let self, self.recording else { return }
                    recorder.updateMeters()
                    self.level = max(0, min(1, (recorder.averagePower(forChannel: 0) + 60) / 60))
                    if !recorder.isRecording { self.finishRecording(); return }
                }
                self?.finishRecording()
            }
        } catch {
            self.error = "Could not access the microphone."
        }
    }

    func stopRecording() {
        guard recording else { return }
        finishRecording()
    }

    // Also called after AVAudioRecorder's own timed stop, when currentTime is zero.
    func finishRecording() {
        recording = false
        timer?.cancel(); timer = nil
        recorder?.stop(); recorder = nil
        level = 0
        hasRecording = Self.recordedDuration(at: fileURL) > 0.2
        if !hasRecording { error = "Record a little longer to hear your voice." }
    }

    static func recordedDuration(at url: URL) -> TimeInterval {
        guard let file = try? AVAudioFile(forReading: url), file.processingFormat.sampleRate > 0 else { return 0 }
        return Double(file.length) / file.processingFormat.sampleRate
    }

    func play(enhanced: Bool) {
        guard hasRecording, !recording else { return }
        stopPlayback()
        error = nil
        do {
            let file = try AVAudioFile(forReading: fileURL)
            let engine = AVAudioEngine()
            let player = AVAudioPlayerNode()
            engine.attach(player)
            if enhanced {
                let eq = AVAudioUnitEQ(numberOfBands: 3)
                let amount = Float(strength) / 100
                let highPass = eq.bands[0]
                highPass.filterType = .highPass; highPass.frequency = 80; highPass.bypass = strength == 0
                let warmth = eq.bands[1]
                warmth.filterType = .parametric; warmth.frequency = 190
                warmth.bandwidth = 1; warmth.gain = 2 * amount; warmth.bypass = false
                let presence = eq.bands[2]
                presence.filterType = .parametric; presence.frequency = 2_900
                presence.bandwidth = 1; presence.gain = 3 * amount; presence.bypass = false
                // This graph affects comparison playback only; the live WebRTC mic is untouched.
                engine.attach(eq)
                engine.connect(player, to: eq, format: file.processingFormat)
                engine.connect(eq, to: engine.mainMixerNode, format: file.processingFormat)
            } else {
                engine.connect(player, to: engine.mainMixerNode, format: file.processingFormat)
            }
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
        } catch {
            stopPlayback()
            self.error = "Could not play the local recording."
        }
    }

    func stopPlayback() {
        playbackGeneration += 1
        player?.stop(); engine?.stop()
        player = nil; engine = nil
    }

    func close() {
        generation += 1
        timer?.cancel(); timer = nil
        recording = false
        recorder?.stop(); recorder = nil
        stopPlayback()
        level = 0; hasRecording = false
        try? FileManager.default.removeItem(at: fileURL)
    }
}
#endif
