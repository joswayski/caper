import Foundation

/// Web's comparison signal check (media/recording.ts): audible when any sample
/// exceeds 0.001 of full scale.
enum MicrophoneSignal {
    static func audible(_ pcm: Data) -> Bool {
        pcm.withUnsafeBytes { raw in
            raw.bindMemory(to: Int16.self).contains { abs(Int32($0)) > 32 }
        }
    }
}
