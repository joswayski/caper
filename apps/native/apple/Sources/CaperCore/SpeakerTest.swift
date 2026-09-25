import AVFoundation
import Observation
#if os(macOS)
import AudioToolbox
#endif

/// Web's speaker test (pages/MicPlayback.tsx `SpeakerTest`): loops the join
/// chime on the selected output at the speaker volume until stopped.
@MainActor @Observable
final class SpeakerTest {
    private(set) var playing = false
    var error: String?
    private var engine: AVAudioEngine?
    private var player: AVAudioPlayerNode?
    private var volume: AVAudioUnitEQ?

    func toggle(voice: VoiceClient) {
        if playing { stop() } else { start(voice: voice) }
    }

    func start(voice: VoiceClient) {
        stop(); error = nil
        guard let url = Bundle(for: SpeakerTest.self).url(forResource: "channel-join", withExtension: "wav"),
              let file = try? AVAudioFile(forReading: url),
              let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)),
              (try? file.read(into: buffer)) != nil else { return fail() }
        #if os(iOS)
        // A call or microphone test keeps its own session; otherwise play without
        // pausing other apps' audio.
        let session = AVAudioSession.sharedInstance()
        if session.category != .playAndRecord && session.category != .record {
            try? session.setCategory(.playback, options: [.mixWithOthers])
            try? session.setActive(true)
        }
        #endif
        let engine = AVAudioEngine(), player = AVAudioPlayerNode()
        let volume = AVAudioUnitEQ(numberOfBands: 1)
        volume.bands[0].bypass = true
        engine.attach(player); engine.attach(volume)
        engine.connect(player, to: volume, format: buffer.format)
        engine.connect(volume, to: engine.mainMixerNode, format: buffer.format)
        #if os(macOS)
        var route = AudioDeviceID(voice.comparisonOutputDeviceID())
        guard route != 0, let output = engine.outputNode.audioUnit,
              AudioUnitSetProperty(output, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0,
                                   &route, UInt32(MemoryLayout.size(ofValue: route))) == noErr else { return fail() }
        #endif
        player.scheduleBuffer(buffer, at: nil, options: .loops)
        do { try engine.start() } catch { return fail() }
        self.engine = engine; self.player = player; self.volume = volume
        setGain(voice.outputGain)
        player.play()
        playing = true
    }

    /// Speaker volume 0–200%, applied like local recording playback.
    func setGain(_ value: Int) {
        let gain = min(200, max(0, value))
        player?.volume = gain == 0 ? 0 : 1
        volume?.globalGain = gain == 0 ? -96 : Float(20 * log10(Double(gain) / 100))
    }

    func stop() {
        player?.stop(); engine?.stop()
        player = nil; engine = nil; volume = nil
        playing = false
    }

    private func fail() {
        stop()
        error = "Couldn’t play audio. Check your output and try again."
    }
}
