import Foundation

/// The call's speaking rings for the native roots (UIKit and AppKit), which
/// call `sync()` inside their observation of the model: it samples who is
/// speaking ten times a second while connected.
@MainActor final class NativeVoiceSpeaking {
    private let voice: VoiceClient
    private var phase: VoiceClient.Phase?
    private var speaking: Task<Void, Never>?
    /// False while nothing on screen shows the rings: sampling stops until
    /// it is true again.
    var sampling = true {
        didSet { if sampling != oldValue { restartSampling() } }
    }

    init(voice: VoiceClient) { self.voice = voice }
    isolated deinit { speaking?.cancel() }

    func sync() {
        guard phase != voice.phase else { return }
        phase = voice.phase
        restartSampling()
    }

    /// The root went away: stop sampling, and sample afresh if it comes back.
    func stop() { speaking?.cancel(); speaking = nil; phase = nil; voice.clearSpeaking() }

    private func restartSampling() {
        speaking?.cancel(); speaking = nil
        guard sampling, voice.phase == .connected else { voice.clearSpeaking(); return }
        speaking = Task { [weak voice] in
            while !Task.isCancelled, let voice, voice.phase == .connected {
                await voice.sampleSpeaking()
                do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
            }
        }
    }
}
