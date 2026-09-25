import Foundation
import Observation
import AVFoundation

/// Decorative UI feedback shares the web assets, gain, pitch and timing limits.
/// It never participates in voice capture or changes the selected call route.
@MainActor @Observable final class CaperEffects {
    enum Effect: String, CaseIterable {
        case toggleOff = "toggle-off", toggleOn = "toggle-on", slider = "slider-tick"
        case leave = "channel-leave", warning, join = "channel-join", message = "new-message", delete, disconnect
    }
    static let shared = CaperEffects(enabled: ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] != "parity")
    private let enabled: Bool
    private let defaults: UserDefaults
    var soundsEnabled: Bool {
        didSet {
            defaults.set(soundsEnabled, forKey: "caper.soundEffects")
            if !soundsEnabled {
                for voice in voices { voice.player.stop() }
                engine.pause()
            }
        }
    }
    private var lastSlider = -Double.infinity
    private let engine = AVAudioEngine()
    private var buffers: [Effect: AVAudioPCMBuffer] = [:]
    private var voices: [(player: AVAudioPlayerNode, speed: AVAudioUnitVarispeed)] = []
    private var nextVoice = 0

    static func decode(_ effect: Effect) -> AVAudioPCMBuffer? {
        guard let url = Bundle(for: CaperEffects.self).url(forResource: effect.rawValue, withExtension: "wav"),
              let file = try? AVAudioFile(forReading: url),
              let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)),
              (try? file.read(into: buffer)) != nil else { return nil }
        return buffer
    }
    #if os(iOS)
    private var idlePause: Task<Void, Never>?
    #endif

    init(enabled: Bool, defaults: UserDefaults = .standard) {
        self.enabled = enabled
        self.defaults = defaults
        soundsEnabled = defaults.object(forKey: "caper.soundEffects") as? Bool ?? true
    }

    func preload() {
        guard enabled, soundsEnabled, buffers.isEmpty else { return }
        for effect in Effect.allCases { buffers[effect] = Self.decode(effect) }
    }

    func toggle(_ on: Bool) { play(on ? .toggleOn : .toggleOff) }

    func slider(_ normalized: Double) {
        let now = ProcessInfo.processInfo.systemUptime
        guard now - lastSlider >= 0.040 else { return }
        lastSlider = now
        let value = min(1, max(0, normalized))
        play(.slider, volume: Float(0.1 + value * 0.22), rate: Float(0.75 + value * 0.6))
    }

    func play(_ effect: Effect, volume: Float = 0.45, rate: Float = 1) {
        guard enabled, soundsEnabled else { return }
        #if os(iOS)
        // Web (audio/session.ts): UI sounds mix with other apps' audio, but a call
        // or microphone test keeps its own session category untouched.
        let session = AVAudioSession.sharedInstance()
        if session.category != .playAndRecord && session.category != .record && session.category != .ambient {
            try? session.setCategory(.ambient)
        }
        #endif
        let requested = ProcessInfo.processInfo.systemUptime
        preload()
        guard let buffer = buffers[effect] else { return }
        if voices.isEmpty {
            for _ in 0..<4 {
                let player = AVAudioPlayerNode()
                let speed = AVAudioUnitVarispeed()
                engine.attach(player); engine.attach(speed)
                engine.connect(player, to: speed, format: buffer.format)
                engine.connect(speed, to: engine.mainMixerNode, format: buffer.format)
                voices.append((player, speed))
            }
        }
        if !engine.isRunning { do { try engine.start() } catch { return } }
        guard ProcessInfo.processInfo.systemUptime - requested <= 0.120 else { return }
        let voice = voices[nextVoice]
        nextVoice = (nextVoice + 1) % voices.count
        voice.player.stop()
        // Web: gain is the requested volume × 0.6.
        voice.player.volume = max(0, min(1, volume)) * 0.6
        voice.speed.rate = max(0.5, min(2, rate))
        voice.player.scheduleBuffer(buffer)
        voice.player.play()
        #if os(iOS)
        // Do not hold the phone's audio hardware between sounds.
        idlePause?.cancel()
        idlePause = Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled, let self else { return }
            if !self.voices.contains(where: { $0.player.isPlaying }) { self.engine.pause() }
        }
        #endif
    }
}
