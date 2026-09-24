import Foundation
#if os(macOS)
import AVFoundation
#endif

/// Decorative UI feedback shares the web assets, gain, pitch and timing limits.
/// It never participates in voice capture or changes the selected call route.
@MainActor final class CaperEffects {
    enum Effect: String, CaseIterable {
        case toggleOff = "toggle-off", toggleOn = "toggle-on", slider = "slider-tick"
        case leave = "channel-leave", warning, join = "channel-join", message = "new-message", delete
    }
    static let shared = CaperEffects(enabled: ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] != "parity")
    private let enabled: Bool
    private var lastSlider = -Double.infinity
    #if os(macOS)
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
    #endif

    init(enabled: Bool) { self.enabled = enabled }

    func preload() {
        #if os(macOS)
        guard enabled, buffers.isEmpty else { return }
        for effect in Effect.allCases { buffers[effect] = Self.decode(effect) }
        #endif
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
        #if os(macOS)
        guard enabled else { return }
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
        voice.player.volume = volume
        voice.speed.rate = rate
        voice.player.scheduleBuffer(buffer)
        voice.player.play()
        #endif
    }
}
