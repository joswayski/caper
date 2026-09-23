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

private struct TurnGeneration: Codable { let generation: String; let refreshAfterMs: Int; let expiresInMs: Int }
private struct JoinResponse: Decodable { let token: String; let id: String; let iceServers: [IceServer]; let turn: TurnGeneration? }
private struct TurnResponse: Decodable { let iceServers: [IceServer]; let turn: TurnGeneration }
private struct JoinBody: Encodable { let name: String; let muted: Bool; let deafened: Bool }
private struct SDP: Codable { let type: String; let sdp: String }
private struct PublishBody: Encodable { let kind = "microphone"; let mid: String; let sessionDescription: SDP }
private struct TrackRef: Decodable { let mid: String }
private struct SignalingResponse: Decodable { let sessionDescription: SDP?; let tracks: [TrackRef]?; let requiresImmediateRenegotiation: Bool? }
private struct SubscribeBody: Encodable { let trackId: String }
private struct NegotiateBody: Encodable { let sessionDescription: SDP }
private struct StateBody: Encodable { let muted: Bool; let deafened: Bool; let sequence: Int }
private struct EmptyBody: Encodable {}
private struct VoiceTrack: Decodable, Hashable { let id: String; let kind: String }
public struct VoiceParticipant: Decodable, Identifiable, Sendable {
    public let id: String
    public let name: String
    public let countryCode: String?
    public let muted: Bool
    public let deafened: Bool
    fileprivate let tracks: [VoiceTrack]
}
private struct VoiceSnapshot: Decodable { let participants: [VoiceParticipant]; let revision: Int? }
private struct RestartBody: Encodable { let generation: String; let sequence: Int; let sessionDescription: SDP }
private struct RestartAckBody: Encodable { let generation: String; let sequence: Int }

public struct AudioDevice: Identifiable, Hashable, Sendable {
    public let id: String
    public let name: String
}

@MainActor @Observable
public final class VoiceClient {
    public enum Phase: Equatable { case idle, joining, connected, reconnecting, leaving, failed }
    public var phase: Phase = .idle
    public var muted = false
    public var deafened = false
    public var participants: [VoiceParticipant] = []
    public var error: String?
    public var showAudioPreferences = false
    public var availableInputs: [AudioDevice] = []
    public var availableOutputs: [AudioDevice] = []
    public var selectedInputID: String?
    public var selectedOutputID: String?

    private let api: APIClient
    private var channelID: String?
    private var token: String?
    private var selfID: String?
    private var peer: RTCPeerConnection?
    private var microphone: RTCAudioTrack?
    private var publishedMID: String?
    private var subscribed: [String: String] = [:]
    private var remoteAudio: [RTCAudioTrack] = []
    private var remoteAudioByMID: [String: RTCAudioTrack] = [:]
    private var pollTask: Task<Void, Never>?
    private var turnTask: Task<Void, Never>?
    private var reconnectTask: Task<Void, Never>?
    private var mediaSubscriptionID: String?
    private var stateSequence = 0
    private var restartSequence = 0
    private var snapshotRevision: Int?
    private var reconnectAttempts = 0
    private var joinName = "Guest"
    private var signalingBusy = false
    private var generation = 0
    private var delegate: PeerDelegate?
    private let factory: RTCPeerConnectionFactory
    private let gateway: Gateway
    #if os(iOS)
    private var audioObservers: [NSObjectProtocol] = []
    #endif

    public init(api: APIClient) {
        self.api = api
        RTCInitializeSSL()
        factory = RTCPeerConnectionFactory(encoderFactory: RTCDefaultVideoEncoderFactory(), decoderFactory: RTCDefaultVideoDecoderFactory())
        gateway = Gateway(baseURL: api.baseURL, token: { [api] in await api.authorizationToken() }) { _, _ in }
    }

    public func join(channelID: String?, name: String) async {
        guard phase == .idle || phase == .failed else { return }
        generation += 1
        let attempt = generation
        joinName = name
        phase = .joining; error = nil; self.channelID = channelID
        do {
            guard await Self.microphonePermission() else { throw VoiceError.permission }
            #if os(iOS)
            try activateAudioSession()
            installAudioObservers()
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
            let delegate = PeerDelegate()
            guard let peer = factory.peerConnection(with: configuration, constraints: constraints, delegate: delegate) else { throw VoiceError.setup }
            self.delegate = delegate
            self.peer = peer
            delegate.receivedAudio = { [weak self, weak peer] callbackPeer, transceiver, track in
                Task { @MainActor in
                    guard let self, let peer, callbackPeer === peer, self.peer === peer,
                          self.generation == attempt, (self.phase == .connected || self.phase == .joining) else {
                        track.isEnabled = false
                        return
                    }
                    track.isEnabled = !self.deafened
                    self.remoteAudio.append(track)
                    if !transceiver.mid.isEmpty { self.remoteAudioByMID[transceiver.mid] = track }
                }
            }
            delegate.connectionChanged = { [weak self, weak peer] callbackPeer, state in
                Task { @MainActor in
                    guard let self, let peer, callbackPeer === peer, self.peer === peer,
                          self.generation == attempt, self.phase == .connected else { return }
                    if state == .failed || state == .disconnected { self.scheduleReconnect(generation: attempt) }
                }
            }
            let source = factory.audioSource(with: RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil))
            let track = factory.audioTrack(with: source, trackId: "caper-microphone")
            // Capture remains locally silenced until the server answer is applied
            // and this exact join attempt is still current.
            track.isEnabled = false
            microphone = track
            guard let transceiver = peer.addTransceiver(with: track, init: RTCRtpTransceiverInit()) else { throw VoiceError.setup }
            var directionError: NSError?
            transceiver.setDirection(.sendOnly, error: &directionError)
            if let directionError { throw directionError }
            let offer = try await peer.offer(for: RTCMediaConstraints(mandatoryConstraints: [kRTCMediaConstraintsOfferToReceiveAudio: kRTCMediaConstraintsValueTrue], optionalConstraints: nil))
            try await peer.setLocalDescription(offer)
            try await Self.waitForGathering(peer)
            let mid = transceiver.mid
            guard let local = peer.localDescription, !mid.isEmpty else { throw VoiceError.setup }
            let published: SignalingResponse = try await api.media(channelID: channelID, operation: "publish", token: joined.token, body: PublishBody(mid: mid, sessionDescription: SDP(type: "offer", sdp: local.sdp)))
            guard let answer = published.sessionDescription else { throw VoiceError.invalidAnswer }
            try await peer.setRemoteDescription(RTCSessionDescription(type: .answer, sdp: answer.sdp))
            guard generation == attempt, phase == .joining else {
                track.isEnabled = false; peer.close()
                try? await api.media(channelID: channelID, operation: "leave", token: joined.token, body: EmptyBody())
                return
            }
            try await Self.waitForConnected(peer)
            await refreshRoster(expectedGeneration: attempt, expectedPeer: peer)
            guard generation == attempt, self.peer === peer, error == nil else { throw VoiceError.setup }
            mediaSubscriptionID = await gateway.subscribeMedia(channelID: channelID, token: joined.token) { [weak self] event in
                self?.receiveMedia(event, generation: attempt, peer: peer)
            }
            guard generation == attempt, self.peer === peer else { throw VoiceError.setup }
            publishedMID = mid
            phase = .connected
            track.isEnabled = !muted
            reconnectAttempts = 0
            startLeaseRenewal()
            if let turn = joined.turn { scheduleTurnRenewal(turn, generation: attempt, peer: peer) }
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

    public func leave() async { leaveImmediately() }

    /// Stops capture, playback and transport synchronously. Remote cleanup is
    /// deliberately detached so provider latency can never resurrect or delay leave.
    public func leaveImmediately() {
        guard phase != .idle else { return }
        generation += 1
        let oldToken = token
        let oldChannelID = channelID
        phase = .idle
        detachLocal()
        Task { [api] in
            if let oldToken { try? await api.media(channelID: oldChannelID, operation: "leave", token: oldToken, body: EmptyBody()) }
        }
    }

    private func startLeaseRenewal() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(15))
                guard let self, let peer = self.peer else { return }
                await self.refreshRoster(expectedGeneration: self.generation, expectedPeer: peer)
            }
        }
    }

    private func refreshRoster(expectedGeneration attempt: Int, expectedPeer peer: RTCPeerConnection) async {
        guard (phase == .connected || phase == .joining), let token, self.peer === peer, generation == attempt else { return }
        do {
            let snapshot: VoiceSnapshot = try await api.media(channelID: channelID, operation: "snapshot", token: token, body: EmptyBody())
            guard generation == attempt, self.peer === peer, (phase == .connected || phase == .joining) else { return }
            try await apply(snapshot: snapshot, generation: attempt, peer: peer, token: token)
        } catch {
            guard generation == attempt else { return }
            if let apiError = error as? APIError, [401, 403].contains(apiError.status) {
                detachLocal(); phase = .failed; self.error = "Voice access ended. Rejoin to recover."
            } else { scheduleReconnect(generation: attempt) }
        }
    }

    private func receiveMedia(_ event: [String: Any], generation attempt: Int, peer expectedPeer: RTCPeerConnection) {
        guard generation == attempt, peer === expectedPeer else { return }
        if event["type"] as? String == "subscription.error" {
            if let status = event["status"] as? Int, [401, 403].contains(status) {
                detachLocal(); phase = .failed; error = "Voice access ended. Rejoin to recover."
            } else { scheduleReconnect(generation: attempt) }
            return
        }
        guard event["type"] as? String == "snapshot",
              let data = try? JSONSerialization.data(withJSONObject: event),
              let snapshot = try? JSONDecoder().decode(VoiceSnapshot.self, from: data) else { return }
        if let revision = snapshot.revision, let current = snapshotRevision, revision <= current { return }
        if let revision = snapshot.revision { snapshotRevision = revision }
        participants = snapshot.participants
        Task { [weak self, weak expectedPeer] in
            guard let self, let expectedPeer, let token = self.token,
                  self.generation == attempt, self.peer === expectedPeer else { return }
            do { try await self.apply(snapshot: snapshot, generation: attempt, peer: expectedPeer, token: token) }
            catch { if self.generation == attempt { self.scheduleReconnect(generation: attempt) } }
        }
    }

    private func apply(snapshot: VoiceSnapshot, generation attempt: Int, peer: RTCPeerConnection, token: String) async throws {
        guard generation == attempt, self.peer === peer else { return }
        try await acquireSignaling(generation: attempt, peer: peer)
        defer { signalingBusy = false }
        participants = snapshot.participants
        let liveTracks = Set(snapshot.participants.filter { $0.id != selfID }.flatMap(\.tracks).filter { $0.kind == "microphone" }.map(\.id))
        for departed in Set(subscribed.keys).subtracting(liveTracks) {
            if let mid = subscribed.removeValue(forKey: departed) {
                if let track = remoteAudioByMID.removeValue(forKey: mid) {
                    track.isEnabled = false
                    remoteAudio.removeAll { $0 === track }
                }
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
                } else if response.requiresImmediateRenegotiation == true { throw VoiceError.invalidAnswer }
                guard generation == attempt, self.peer === peer else { return }
                subscribed[track.id] = mid
            }
        }
    }

    private func syncState() async {
        guard phase == .connected, let token else { return }
        stateSequence += 1
        do { try await api.media(channelID: channelID, operation: "state", token: token, body: StateBody(muted: muted, deafened: deafened, sequence: stateSequence)) }
        catch { self.error = "Voice state did not sync: \(error.localizedDescription)" }
    }

    private func teardown(sendLeave: Bool) async {
        let oldToken = token
        let oldChannelID = channelID
        detachLocal()
        if sendLeave, let oldToken { try? await api.media(channelID: oldChannelID, operation: "leave", token: oldToken, body: EmptyBody()) }
    }

    private func detachLocal() {
        pollTask?.cancel(); pollTask = nil
        turnTask?.cancel(); turnTask = nil
        reconnectTask?.cancel(); reconnectTask = nil
        if let mediaSubscriptionID { Task { await gateway.unsubscribe(mediaSubscriptionID) } }
        mediaSubscriptionID = nil
        microphone?.isEnabled = false
        remoteAudio.forEach { $0.isEnabled = false }
        peer?.close()
        peer = nil; delegate = nil; microphone = nil
        remoteAudio = []; remoteAudioByMID = [:]
        token = nil; selfID = nil; subscribed = [:]; participants = []
        publishedMID = nil; snapshotRevision = nil; signalingBusy = false
        channelID = nil
        #if os(iOS)
        removeAudioObservers()
        let audio = RTCAudioSession.sharedInstance()
        audio.lockForConfiguration()
        try? audio.setActive(false)
        audio.unlockForConfiguration()
        #endif
    }

    private func scheduleReconnect(generation attempt: Int) {
        guard generation == attempt, (phase == .connected || phase == .joining) else { return }
        guard reconnectTask == nil else { return }
        reconnectAttempts += 1
        guard reconnectAttempts <= 3 else {
            generation += 1; detachLocal(); phase = .failed; error = "Connection lost. Please join again."
            return
        }
        let nextAttempt = attempt + 1
        let oldToken = token, oldChannelID = channelID, oldName = joinName
        generation = nextAttempt
        phase = .reconnecting
        detachLocal()
        phase = .reconnecting
        if let oldToken { Task { [api] in try? await api.media(channelID: oldChannelID, operation: "leave", token: oldToken, body: EmptyBody()) } }
        reconnectTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(min(pow(2, Double(self?.reconnectAttempts ?? 1)), 5)))
            guard !Task.isCancelled, let self, self.generation == nextAttempt, self.phase == .reconnecting else { return }
            self.reconnectTask = nil
            self.phase = .idle
            await self.join(channelID: oldChannelID, name: oldName)
        }
    }

    private func scheduleTurnRenewal(_ turn: TurnGeneration, generation attempt: Int, peer: RTCPeerConnection) {
        turnTask?.cancel()
        turnTask = Task { [weak self, weak peer] in
            try? await Task.sleep(for: .milliseconds(max(1_000, turn.refreshAfterMs)))
            guard !Task.isCancelled, let self, let peer,
                  self.generation == attempt, self.peer === peer, self.phase == .connected else { return }
            await self.renewTurn(current: turn, generation: attempt, peer: peer)
        }
    }

    private func renewTurn(current: TurnGeneration, generation attempt: Int, peer: RTCPeerConnection) async {
        guard let token, generation == attempt, self.peer === peer else { return }
        do {
            let response: TurnResponse = try await retryControl {
                try await self.api.media(channelID: self.channelID, operation: "turn", token: token, body: TurnRequest(generation: current.generation))
            }
            guard generation == attempt, self.peer === peer else { return }
            if response.turn.generation == current.generation {
                scheduleTurnRenewal(response.turn, generation: attempt, peer: peer)
                return
            }
            try await acquireSignaling(generation: attempt, peer: peer)
            defer { signalingBusy = false }
            let configuration = peer.configuration
            configuration.iceServers = response.iceServers.map { RTCIceServer(urlStrings: $0.urls, username: $0.username, credential: $0.credential) }
            guard peer.setConfiguration(configuration) else { throw VoiceError.setup }
            restartSequence += 1
            peer.restartIce()
            let offer = try await peer.offer(for: RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil))
            try await peer.setLocalDescription(offer)
            try await Self.waitForGathering(peer)
            guard generation == attempt, self.peer === peer, let local = peer.localDescription else { throw CancellationError() }
            let restarted: SignalingResponse = try await retryControl {
                try await self.api.media(channelID: self.channelID, operation: "restart-ice", token: token, body: RestartBody(generation: response.turn.generation, sequence: self.restartSequence, sessionDescription: SDP(type: "offer", sdp: local.sdp)))
            }
            guard let answer = restarted.sessionDescription else { throw VoiceError.invalidAnswer }
            try await peer.setRemoteDescription(RTCSessionDescription(type: .answer, sdp: answer.sdp))
            try await retryControl {
                try await self.api.media(channelID: self.channelID, operation: "restart-ice-ack", token: token, body: RestartAckBody(generation: response.turn.generation, sequence: self.restartSequence))
            }
            guard generation == attempt, self.peer === peer else { return }
            scheduleTurnRenewal(response.turn, generation: attempt, peer: peer)
        } catch {
            guard generation == attempt else { return }
            signalingBusy = false
            scheduleReconnect(generation: attempt)
        }
    }

    private func acquireSignaling(generation attempt: Int, peer: RTCPeerConnection) async throws {
        while signalingBusy {
            try Task.checkCancellation()
            guard generation == attempt, self.peer === peer else { throw CancellationError() }
            try await Task.sleep(for: .milliseconds(25))
        }
        guard generation == attempt, self.peer === peer else { throw CancellationError() }
        signalingBusy = true
    }

    private func retryControl<T>(_ operation: () async throws -> T) async throws -> T {
        var delay = 1.0
        for attempt in 0..<4 {
            do { return try await operation() }
            catch let failure as APIError where failure.status == 401 || failure.status == 403 { throw failure }
            catch let failure as APIError where failure.status == 408 || failure.status == 409 || failure.status == 429 || failure.status >= 500 {
                guard attempt < 3 else { throw failure }
            } catch {
                guard attempt < 3 else { throw error }
            }
            try await Task.sleep(for: .seconds(delay)); delay *= 2
        }
        throw VoiceError.timeout
    }

    public func refreshAudioDevices() async {
        // WebRTC follows the system's selected input/output route. Listing a
        // device that cannot actually be selected would be misleading.
        availableInputs = []
        availableOutputs = []
        selectedInputID = nil
        selectedOutputID = nil
    }

    private static func microphonePermission() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return true
        case .notDetermined: return await AVCaptureDevice.requestAccess(for: .audio)
        default: return false
        }
    }

    #if os(iOS)
    private func activateAudioSession() throws {
        let audio = RTCAudioSession.sharedInstance()
        audio.lockForConfiguration()
        defer { audio.unlockForConfiguration() }
        try audio.setCategory(.playAndRecord, with: [.allowBluetooth, .defaultToSpeaker])
        try audio.setMode(.voiceChat)
        try audio.setActive(true)
    }

    private func installAudioObservers() {
        removeAudioObservers()
        let center = NotificationCenter.default
        audioObservers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
            Task { @MainActor in self?.handleAudioInterruption(note) }
        })
        audioObservers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in await self?.refreshAudioDevices() }
        })
    }

    private func removeAudioObservers() {
        let center = NotificationCenter.default
        audioObservers.forEach(center.removeObserver)
        audioObservers = []
    }

    private func handleAudioInterruption(_ note: Notification) {
        guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
              let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
        if type == .began { microphone?.isEnabled = false }
        else if phase == .connected {
            do { try activateAudioSession(); microphone?.isEnabled = !muted }
            catch { scheduleReconnect(generation: generation) }
        }
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

private struct TurnRequest: Encodable { let generation: String }
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
    var receivedAudio: (@Sendable (RTCPeerConnection, RTCRtpTransceiver, RTCAudioTrack) -> Void)?
    var connectionChanged: (@Sendable (RTCPeerConnection, RTCPeerConnectionState) -> Void)?
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}
    func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCPeerConnectionState) { connectionChanged?(peerConnection, newState) }
    func peerConnection(_ peerConnection: RTCPeerConnection, didStartReceivingOn transceiver: RTCRtpTransceiver) {
        if let track = transceiver.receiver.track as? RTCAudioTrack { receivedAudio?(peerConnection, transceiver, track) }
    }
}

private extension RTCPeerConnection {
    func offer(for constraints: RTCMediaConstraints) async throws -> RTCSessionDescription { try await withCheckedThrowingContinuation { continuation in offer(for: constraints) { value, error in value.map { continuation.resume(returning: $0) } ?? continuation.resume(throwing: error ?? VoiceError.setup) } } }
    func answer(for constraints: RTCMediaConstraints) async throws -> RTCSessionDescription { try await withCheckedThrowingContinuation { continuation in answer(for: constraints) { value, error in value.map { continuation.resume(returning: $0) } ?? continuation.resume(throwing: error ?? VoiceError.setup) } } }
    func setLocalDescription(_ description: RTCSessionDescription) async throws { try await withCheckedThrowingContinuation { continuation in setLocalDescription(description) { error in error.map { continuation.resume(throwing: $0) } ?? continuation.resume() } } }
    func setRemoteDescription(_ description: RTCSessionDescription) async throws { try await withCheckedThrowingContinuation { continuation in setRemoteDescription(description) { error in error.map { continuation.resume(throwing: $0) } ?? continuation.resume() } } }
}
