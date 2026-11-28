import AVFoundation

/// The voice chimes (shared/sounds): yours as you join or leave, and others'
/// as they come and go while you are in and not deafened.
@MainActor
final class VoiceSounds {
    enum Sound: String { case join = "voice-join", leave = "voice-leave" }

    /// Players stay referenced until they finish.
    private var playing: [AVAudioPlayer] = []

    /// Plays `sound` at the call's output gain (0–200%, capped at full volume).
    func play(_ sound: Sound, gain: Int) {
        playing.removeAll { !$0.isPlaying }
        guard gain > 0,
              let url = Bundle(for: VoiceSounds.self).url(forResource: sound.rawValue, withExtension: "wav"),
              let player = try? AVAudioPlayer(contentsOf: url) else { return }
        player.volume = Float(min(100, gain)) / 100
        if player.play() { playing.append(player) }
    }
}
