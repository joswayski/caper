#if os(macOS)
import AVFoundation
import AudioToolbox
import CaperRTCBridge
import Observation

/// Bounded local comparison of raw input and the same processed PCM submitted to WebRTC.
/// Neither recording is transmitted by the test; VoiceClient suspends publication in a call.
@MainActor @Observable
final class MacMicrophoneTest {
    private(set) var recording = false
    private(set) var hasRecording = false
    var error: String?
    /// When the current recording started, for web's elapsed clock.
    private(set) var startedAt: Date?
    /// Web: "No audible signal detected" when no natural sample exceeds 0.001.
    private(set) var silent = false
    /// The sample being played back: false natural, true enhanced.
    private(set) var playing: Bool?
    private var advanceToEnhanced = false

    private var voice: VoiceClient?
    private var voiceAttempt: Int?
    private let device: CaperMacAudioDevice
    private let fileURL: URL
    private var timer: Task<Void, Never>?
    private var generation = 0
    private var playbackGeneration = 0
    private var engine: AVAudioEngine?
    private var player: AVAudioPlayerNode?
    private var playbackOutputID: UInt32 = 0
    private var playbackGain = 100

    init(device: CaperMacAudioDevice = CaperMacAudioDevice(),
         fileURL: URL = FileManager.default.temporaryDirectory.appendingPathComponent("caper-mic-test-\(UUID().uuidString).caf")) {
        self.device = device
        self.fileURL = fileURL
    }

    func start(voice: VoiceClient? = nil) async {
        guard !recording else { return }
        generation += 1
        let attempt = generation
        self.voice = voice
        stopPlayback()
        timer?.cancel()
        clearFiles()
        hasRecording = false; error = nil
        let permitted: Bool
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: permitted = true
        case .notDetermined: permitted = await AVCaptureDevice.requestAccess(for: .audio)
        default: permitted = false
        }
        guard attempt == generation else { return }
        guard permitted else { error = "Microphone access is required for a local test."; return }
        let ready: Bool
        if let voice { ready = await voice.prepareMicrophoneDenoise() }
        else {
            let device = self.device
            ready = await Task.detached(priority: .userInitiated) { device.prepareDenoise() }.value
        }
        guard attempt == generation else { return }
        guard ready else { error = "On-device noise suppression could not start. No microphone audio was captured."; return }
        if let voice {
            guard let attempt = voice.beginMicrophoneComparison() else {
                error = "Could not start recording from the selected microphone."
                return
            }
            voiceAttempt = attempt
        } else {
            guard device.beginComparison() else {
                error = "Could not start recording from the selected microphone."
                return
            }
        }
        recording = true
        startedAt = Date()
        timer = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(30)) } catch { return }
            self?.stopRecording()
        }
    }

    func stopRecording() {
        guard recording else { return }
        finishRecording()
    }

    /// RMS of the latest natural chunk while recording.
    var level: Float { recording ? (voice?.microphoneComparisonLevel() ?? device.comparisonLevel) : 0 }

    func finishRecording() {
        recording = false
        startedAt = nil
        timer?.cancel(); timer = nil
        playbackOutputID = voice?.comparisonOutputDeviceID() ?? device.resolvedOutputDeviceID
        playbackGain = voice?.outputGain ?? 100
        let samples: CaperAudioComparison?
        if let voice, let voiceAttempt { samples = voice.finishMicrophoneRecording(generation: voiceAttempt) }
        else if voice == nil { samples = device.endComparison() }
        else { samples = nil }
        saveComparison(samples)
    }

    func saveComparison(_ samples: CaperAudioComparison?) {
        guard let samples,
              Double(samples.natural.count) > 0.2 * samples.sampleRate * 2,
              samples.natural.count == samples.enhanced.count else {
            error = "Record a little longer to hear your voice."
            return
        }
        do {
            try Self.write(samples.natural, rate: samples.sampleRate, to: fileURL)
            try Self.write(samples.enhanced, rate: samples.sampleRate, to: enhancedURL)
            silent = !MicrophoneSignal.audible(samples.natural)
            hasRecording = true
            playComparison()
        } catch {
            clearFiles()
            self.error = "Could not save the local comparison."
        }
    }

    private static func write(_ pcm: Data, rate: Double, to url: URL) throws {
        guard let format = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: rate, channels: 1, interleaved: true),
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(pcm.count / 2)),
              let channel = buffer.int16ChannelData?.pointee else { throw CocoaError(.fileWriteUnknown) }
        pcm.copyBytes(to: UnsafeMutableRawBufferPointer(start: channel, count: pcm.count))
        buffer.frameLength = AVAudioFrameCount(pcm.count / 2)
        try AVAudioFile(forWriting: url, settings: format.settings, commonFormat: .pcmFormatInt16, interleaved: true).write(from: buffer)
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
            let file = try AVAudioFile(forReading: enhanced ? enhancedURL : fileURL)
            let engine = AVAudioEngine()
            let player = AVAudioPlayerNode()
            player.volume = Self.playerVolume(for: playbackGain)
            engine.mainMixerNode.outputVolume = Self.playerVolume(for: playbackGain)
            let volume = AVAudioUnitEQ(numberOfBands: 1)
            volume.bands[0].bypass = true
            volume.globalGain = Self.playbackDecibels(for: playbackGain)
            engine.attach(player)
            engine.attach(volume)
            engine.connect(player, to: volume, format: file.processingFormat)
            engine.connect(volume, to: engine.mainMixerNode, format: file.processingFormat)
            guard playbackOutputID != 0, let output = engine.outputNode.audioUnit else {
                throw CocoaError(.fileReadUnknown)
            }
            var route = AudioDeviceID(playbackOutputID)
            guard AudioUnitSetProperty(output, kAudioOutputUnitProperty_CurrentDevice,
                                       kAudioUnitScope_Global, 0, &route, UInt32(MemoryLayout.size(ofValue: route))) == noErr else {
                throw CocoaError(.fileReadUnknown)
            }
            let playback = playbackGeneration
            player.scheduleFile(file, at: nil, completionCallbackType: .dataPlayedBack) { [weak self, weak player] _ in
                Task { @MainActor in
                    guard let self, let player, self.playbackGeneration == playback, self.player === player else { return }
                    // Web plays the natural sample, then the enhanced one.
                    if self.advanceToEnhanced && !enhanced { self.advanceToEnhanced = false; self.play(enhanced: true) }
                    else { self.stopPlayback() }
                }
            }
            try engine.start()
            self.engine = engine; self.player = player
            player.play()
            playing = enhanced
        } catch {
            stopPlayback()
            self.error = "Could not play the local recording."
        }
    }

    static func playbackDecibels(for gain: Int) -> Float {
        gain == 0 ? -96 : Float(20 * log10(Double(max(0, min(200, gain))) / 100))
    }

    static func playerVolume(for gain: Int) -> Float { gain == 0 ? 0 : 1 }

    func playComparison() {
        guard hasRecording, !recording else { return }
        play(enhanced: false)
        advanceToEnhanced = playing == false
    }

    func stopPlayback() {
        playing = nil; advanceToEnhanced = false
        playbackGeneration += 1
        player?.stop(); engine?.stop()
        player = nil; engine = nil
    }

    func close() {
        generation += 1
        timer?.cancel(); timer = nil
        recording = false; startedAt = nil; silent = false; advanceToEnhanced = false
        stopPlayback()
        if let voice, let voiceAttempt { voice.endMicrophoneComparison(generation: voiceAttempt) }
        else { _ = device.endComparison() }
        voice = nil
        voiceAttempt = nil
        hasRecording = false
        clearFiles()
    }

    private var enhancedURL: URL {
        fileURL.deletingPathExtension().appendingPathExtension("enhanced.caf")
    }

    private func clearFiles() {
        try? FileManager.default.removeItem(at: fileURL)
        try? FileManager.default.removeItem(at: enhancedURL)
    }
}
#endif
