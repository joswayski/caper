import Foundation
import Observation
import WebRTC
import AVFoundation
#if os(iOS)
import AVFAudio
#endif

public struct IceServer: Decodable, Sendable {
    public let urls: [String]
    public let username: String?
    public let credential: String?

    enum CodingKeys: String, CodingKey { case urls, username, credential }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        if let list = try? values.decode([String].self, forKey: .urls) { urls = list }
        else { urls = [try values.decode(String.self, forKey: .urls)] }
        username = try values.decodeIfPresent(String.self, forKey: .username)
        credential = try values.decodeIfPresent(String.self, forKey: .credential)
    }
}

private struct JoinResponse: Decodable { let token: String; let id: String; let iceServers: [IceServer] }
private struct JoinBody: Encodable { let name: String; let muted: Bool; let deafened: Bool }
private struct SDP: Codable { let type: String; let sdp: String }
private struct PublishBody: Encodable { let kind = "microphone"; let mid: String; let sessionDescription: SDP }
private struct TrackRef: Decodable { let mid: String }
private struct SignalingResponse: Decodable { let sessionDescription: SDP?; let tracks: [TrackRef]? }
private struct SubscribeBody: Encodable { let trackId: String }
private struct NegotiateBody: Encodable { let sessionDescription: SDP }
private struct StateBody: Encodable { let muted: Bool; let deafened: Bool; let sequence: Int }
private struct EmptyBody: Encodable {}
private struct VoiceTrack: Decodable, Hashable { let id: String; let kind: String }
private struct VoiceParticipant: Decodable, Identifiable { let id: String; let name: String; let muted: Bool; let deafened: Bool; let tracks: [VoiceTrack] }
private struct VoiceSnapshot: Decodable { let participants: [VoiceParticipant] }

@MainActor @Observable
public final class VoiceClient {
    public enum Phase: Equatable { case idle, joining, connected, leaving, failed }
    public var phase: Phase = .idle
    public var muted = false
    public var deafened = false
    public var participantNames: [String] = []
    public var error: String?

    private let api: APIClient
    private var channelID: String?
    private var token: String?
    private var selfID: String?
    private var peer: RTCPeerConnection?
    private var microphone: RTCAudioTrack?
    private var publishedMID: String?
    private var subscribed: [String: String] = [:]
    private var remoteAudio: [RTCAudioTrack] = []
    private var pollTask: Task<Void, Never>?
    private var stateSequence = 0
    private var generation = 0
    private let delegate = PeerDelegate()
    private let factory: RTCPeerConnectionFactory

    public init(api: APIClient) {
        self.api = api
        RTCInitializeSSL()
        factory = RTCPeerConnectionFactory(encoderFactory: RTCDefaultVideoEncoderFactory(), decoderFactory: RTCDefaultVideoDecoderFactory())
        delegate.receivedAudio = { [weak self] track in
            Task { @MainActor in
                guard let self, self.phase == .connected else { track.isEnabled = false; return }
                track.isEnabled = !self.deafened
                self.remoteAudio.append(track)
            }
        }
    }

    public func join(channelID: String?, name: String) async {
        guard phase == .idle || phase == .failed else { return }
        generation += 1
        let attempt = generation
        phase = .joining; error = nil; self.channelID = channelID
        do {
            guard await Self.microphonePermission() else { throw VoiceError.permission }
            #if os(iOS)
            try Self.activateAudioSession()
            #endif
            let joined: JoinResponse = try await api.media(channelID: channelID, operation: "join", body: JoinBody(name: name, muted: muted, deafened: deafened))
            guard generation == attempt, phase == .joining else {
                try? await api.media(channelID: channelID, operation: "leave", token: joined.token, body: EmptyBody())
                return
            }
            token = joined.token; selfID = joined.id
            let configuration = RTCConfiguration()
            configuration.sdpSemantics = .unifiedPlan
            configuration.continualGatheringPolicy = .gatherContinually
            configuration.iceServers = joined.iceServers.map { RTCIceServer(urlStrings: $0.urls, username: $0.username, credential: $0.credential) }
            let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: ["DtlsSrtpKeyAgreement": kRTCMediaConstraintsValueTrue])
            guard let peer = factory.peerConnection(with: configuration, constraints: constraints, delegate: delegate) else { throw VoiceError.setup }
            self.peer = peer
            let source = factory.audioSource(with: RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil))
            let track = factory.audioTrack(with: source, trackId: "caper-microphone")
            // Capture remains locally silenced until the server answer is applied
            // and this exact join attempt is still current.
            track.isEnabled = false
            microphone = track
            guard let transceiver = peer.addTransceiver(with: track, init: RTCRtpTransceiverInit()) else { throw VoiceError.setup }
            try transceiver.setDirection(.sendOnly)
            let offer = try await peer.offer(for: RTCMediaConstraints(mandatoryConstraints: [kRTCMediaConstraintsOfferToReceiveAudio: kRTCMediaConstraintsValueTrue], optionalConstraints: nil))
            try await peer.setLocalDescription(offer)
            try await Self.waitForGathering(peer)
            guard let local = peer.localDescription, let mid = transceiver.mid else { throw VoiceError.setup }
            let published: SignalingResponse = try await api.media(channelID: channelID, operation: "publish", token: joined.token, body: PublishBody(mid: mid, sessionDescription: SDP(type: "offer", sdp: local.sdp)))
            guard let answer = published.sessionDescription else { throw VoiceError.invalidAnswer }
            try await peer.setRemoteDescription(RTCSessionDescription(type: .answer, sdp: answer.sdp))
            guard generation == attempt, phase == .joining else {
                track.isEnabled = false; peer.close()
                try? await api.media(channelID: channelID, operation: "leave", token: joined.token, body: EmptyBody())
                return
            }
            try await Self.waitForConnected(peer)
            publishedMID = mid
            phase = .connected
            track.isEnabled = !muted
            startRosterPolling()
        } catch {
            guard generation == attempt else { return }
            await teardown(sendLeave: true)
            phase = .failed; self.error = error.localizedDescription
        }
    }

    public func setMuted(_ value: Bool) async {
        muted = value || deafened; microphone?.isEnabled = !muted
        await syncState()
    }

    public func setDeafened(_ value: Bool) async {
        deafened = value
        remoteAudio.forEach { $0.isEnabled = !value }
        if value { muted = true; microphone?.isEnabled = false }
        await syncState()
    }

    public func leave() async {
        guard phase != .idle else { return }
        generation += 1
        phase = .leaving
        await teardown(sendLeave: true)
        phase = .idle
    }

    private func startRosterPolling() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshRoster()
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    private func refreshRoster() async {
        guard phase == .connected, let token, let peer else { return }
        let attempt = generation
        do {
            let snapshot: VoiceSnapshot = try await api.media(channelID: channelID, operation: "snapshot", token: token, body: EmptyBody())
            guard generation == attempt, self.peer === peer, phase == .connected else { return }
            participantNames = snapshot.participants.map(\.name)
            let liveTracks = Set(snapshot.participants.filter { $0.id != selfID }.flatMap(\.tracks).filter { $0.kind == "microphone" }.map(\.id))
            for departed in Set(subscribed.keys).subtracting(liveTracks) {
                if let mid = subscribed.removeValue(forKey: departed) {
                    try? await api.media(channelID: channelID, operation: "close", token: token, body: CloseBody(mid: mid))
                }
            }
            for participant in snapshot.participants where participant.id != selfID {
                for track in participant.tracks where track.kind == "microphone" && subscribed[track.id] == nil {
                    let response: SignalingResponse = try await api.media(channelID: channelID, operation: "subscribe", token: token, body: SubscribeBody(trackId: track.id))
                    guard generation == attempt, self.peer === peer, let mid = response.tracks?.first?.mid else { return }
                    if let offer = response.sessionDescription {
                        try await peer.setRemoteDescription(RTCSessionDescription(type: .offer, sdp: offer.sdp))
                        let answer = try await peer.answer(for: RTCMediaConstraints(mandatoryConstraints: [kRTCMediaConstraintsOfferToReceiveAudio: kRTCMediaConstraintsValueTrue], optionalConstraints: nil))
                        try await peer.setLocalDescription(answer)
                        try await Self.waitForGathering(peer)
                        guard generation == attempt, self.peer === peer, let local = peer.localDescription else { return }
                        try await api.media(channelID: channelID, operation: "negotiate", token: token, body: NegotiateBody(sessionDescription: SDP(type: "answer", sdp: local.sdp)))
                    }
                    guard generation == attempt, self.peer === peer else { return }
                    subscribed[track.id] = mid
                }
            }
        } catch {
            guard generation == attempt else { return }
            if let apiError = error as? APIError, [401, 403].contains(apiError.status) {
                await teardown(sendLeave: false); phase = .failed; self.error = "Voice access ended. Rejoin to recover."
            } else { self.error = "Voice updates interrupted: \(error.localizedDescription)" }
        }
    }

    private func syncState() async {
        guard phase == .connected, let token else { return }
        stateSequence += 1
        do { try await api.media(channelID: channelID, operation: "state", token: token, body: StateBody(muted: muted, deafened: deafened, sequence: stateSequence)) }
        catch { self.error = "Voice state did not sync: \(error.localizedDescription)" }
    }

    private func teardown(sendLeave: Bool) async {
        pollTask?.cancel(); pollTask = nil
        let oldToken = token
        microphone?.isEnabled = false; peer?.close()
        remoteAudio.forEach { $0.isEnabled = false }
        peer = nil; microphone = nil; remoteAudio = []; token = nil; selfID = nil; subscribed = [:]; participantNames = []; publishedMID = nil
        if sendLeave, let oldToken { try? await api.media(channelID: channelID, operation: "leave", token: oldToken, body: EmptyBody()) }
        #if os(iOS)
        let audio = RTCAudioSession.sharedInstance()
        audio.lockForConfiguration()
        try? audio.setActive(false)
        audio.unlockForConfiguration()
        #endif
    }

    private static func microphonePermission() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return true
        case .notDetermined: return await AVCaptureDevice.requestAccess(for: .audio)
        default: return false
        }
    }

    #if os(iOS)
    private static func activateAudioSession() throws {
        let audio = RTCAudioSession.sharedInstance()
        audio.lockForConfiguration()
        defer { audio.unlockForConfiguration() }
        try audio.setCategory(AVAudioSession.Category.playAndRecord.rawValue, with: [.allowBluetooth, .defaultToSpeaker])
        try audio.setMode(AVAudioSession.Mode.voiceChat.rawValue)
        try audio.setActive(true)
    }
    #endif

    private static func waitForGathering(_ peer: RTCPeerConnection) async throws {
        let deadline = ContinuousClock.now + .seconds(10)
        while peer.iceGatheringState != .complete {
            try Task.checkCancellation()
            guard ContinuousClock.now < deadline else { throw VoiceError.timeout }
            try await Task.sleep(for: .milliseconds(50))
        }
    }

    private static func waitForConnected(_ peer: RTCPeerConnection) async throws {
        let deadline = ContinuousClock.now + .seconds(12)
        while peer.connectionState != .connected {
            try Task.checkCancellation()
            guard peer.connectionState != .failed, ContinuousClock.now < deadline else { throw VoiceError.timeout }
            try await Task.sleep(for: .milliseconds(50))
        }
    }
}

private struct CloseBody: Encodable { let mid: String }
private enum VoiceError: LocalizedError { case setup, invalidAnswer, permission, timeout
    var errorDescription: String? {
        switch self {
        case .setup: return "The audio connection could not be prepared."
        case .invalidAnswer: return "The voice service returned an invalid answer."
        case .permission: return "Microphone permission is required to join voice."
        case .timeout: return "The voice connection timed out."
        }
    }
}

private final class PeerDelegate: NSObject, RTCPeerConnectionDelegate, @unchecked Sendable {
    var receivedAudio: (@Sendable (RTCAudioTrack) -> Void)?
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}
    func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCPeerConnectionState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didStartReceivingOn transceiver: RTCRtpTransceiver) {
        if let track = transceiver.receiver.track as? RTCAudioTrack { receivedAudio?(track) }
    }
}

private extension RTCPeerConnection {
    func offer(for constraints: RTCMediaConstraints) async throws -> RTCSessionDescription { try await withCheckedThrowingContinuation { continuation in offer(for: constraints) { value, error in value.map { continuation.resume(returning: $0) } ?? continuation.resume(throwing: error ?? VoiceError.setup) } } }
    func answer(for constraints: RTCMediaConstraints) async throws -> RTCSessionDescription { try await withCheckedThrowingContinuation { continuation in answer(for: constraints) { value, error in value.map { continuation.resume(returning: $0) } ?? continuation.resume(throwing: error ?? VoiceError.setup) } } }
    func setLocalDescription(_ description: RTCSessionDescription) async throws { try await withCheckedThrowingContinuation { continuation in setLocalDescription(description) { error in error.map { continuation.resume(throwing: $0) } ?? continuation.resume() } } }
    func setRemoteDescription(_ description: RTCSessionDescription) async throws { try await withCheckedThrowingContinuation { continuation in setRemoteDescription(description) { error in error.map { continuation.resume(throwing: $0) } ?? continuation.resume() } } }
}
