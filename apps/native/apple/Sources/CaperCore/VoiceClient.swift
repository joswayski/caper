import Foundation
import Observation
import WebRTC
import AVFoundation
#if os(macOS)
import CaperRTCBridge
#endif
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

public struct VoiceContext: Equatable, Sendable {
    public let channelID: String
    public let channelName: String
    public let spaceID: String
    public let spaceName: String

    public init(channelID: String, channelName: String, spaceID: String, spaceName: String) {
        self.channelID = channelID; self.channelName = channelName
        self.spaceID = spaceID; self.spaceName = spaceName
    }
}

@MainActor @Observable
public final class VoiceClient {
    public enum Phase: Equatable { case idle, joining, connected, reconnecting, leaving, failed }
    public var phase: Phase = .idle
    public var muted = false
    public var deafened = false
    public var participants: [VoiceParticipant] = []
    public var outputGain = UserDefaults.standard.object(forKey: "caper.voice.outputGain") == nil
        ? 100 : min(200, max(0, UserDefaults.standard.integer(forKey: "caper.voice.outputGain")))
    public var participantGains: [String: Int] = [:]
    public var locallyMutedParticipants: Set<String> = []
    public var error: String?
    public var showAudioPreferences = false
    public var availableInputs: [AudioDevice] = []
    public var availableOutputs: [AudioDevice] = []
    public var selectedInputID: String?
    public var selectedOutputID: String?
    #if os(macOS)
    public private(set) var inputGain = 100
    public private(set) var voiceProcessingStrength = 25
    private let audioDevice: CaperMacAudioDevice
    private var comparisonPeer: RTCPeerConnection?
    private var comparisonGeneration: Int?
    #endif
    public private(set) var diagnostics: VoiceDiagnostics?
    public internal(set) var context: VoiceContext?

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
    private var participantByMID: [String: String] = [:]
    private var pollTask: Task<Void, Never>?
    private var turnTask: Task<Void, Never>?
    private var reconnectTask: Task<Void, Never>?
    private var mediaSubscriptionID: String?
    private var stateSequence = 0
    private var restartSequence = 0
    private var muteBeforeDeafen = false
    private var snapshotRevisions = MonotonicRevision()
    private var reconnectAttempts = 0
    private var joinName = "Guest"
    private var signalingOwner: Int?
    private var generation = 0
    private var previousStatistics: VoiceStatisticsSample?
    private var delegate: PeerDelegate?
    private let factory: RTCPeerConnectionFactory?
    private let gateway: Gateway
    private let requestMicrophonePermission: @MainActor () async -> Bool
    #if os(iOS)
    private var audioObservers: [NSObjectProtocol] = []
    #endif

    public convenience init(api: APIClient) {
        self.init(api: api, requestMicrophonePermission: Self.microphonePermission)
    }

    init(api: APIClient, requestMicrophonePermission: @escaping @MainActor () async -> Bool) {
        self.api = api
        self.requestMicrophonePermission = requestMicrophonePermission
        RTCInitializeSSL()
        #if os(macOS)
        let audioDevice = CaperMacAudioDevice()
        self.audioDevice = audioDevice
        let inputGain = UserDefaults.standard.object(forKey: "caper.voice.inputGain") == nil ? 100 : UserDefaults.standard.integer(forKey: "caper.voice.inputGain")
        let strength = UserDefaults.standard.object(forKey: "caper.voice.processingStrength") == nil ? 25 : UserDefaults.standard.integer(forKey: "caper.voice.processingStrength")
        let initialGain = min(200, max(0, inputGain))
        let initialStrength = min(100, max(0, strength))
        self.inputGain = initialGain
        voiceProcessingStrength = initialStrength
        audioDevice.inputGain = initialGain
        audioDevice.processingStrength = initialStrength
        for (key, select) in [("caper.voice.inputUID", true), ("caper.voice.outputUID", false)] {
            let uid = UserDefaults.standard.string(forKey: key) ?? ""
            if select { _ = audioDevice.selectInputUID(uid) }
            else { _ = audioDevice.selectOutputUID(uid) }
        }
        factory = CaperCreateAudioPeerFactory(audioDevice)
        #else
        factory = RTCPeerConnectionFactory(encoderFactory: RTCDefaultVideoEncoderFactory(), decoderFactory: RTCDefaultVideoDecoderFactory())
        #endif
        gateway = Gateway(baseURL: api.baseURL, token: { [api] in await api.authorizationToken() }) { _, _ in }
    }

    static func configuration(iceServers: [IceServer]) -> RTCConfiguration {
        let configuration = RTCConfiguration()
        configuration.sdpSemantics = .unifiedPlan
        // Network recovery explicitly restarts ICE. Send pending SDP just as
        // the web client does; never wait for all STUN/TURN probes to finish.
        configuration.continualGatheringPolicy = .gatherOnce
        configuration.iceServers = iceServers.map { RTCIceServer(urlStrings: $0.urls, username: $0.username, credential: $0.credential) }
        return configuration
    }

    public func join(channelID: String?, context: VoiceContext, name: String) async {
        guard phase == .idle || phase == .failed else { return }
        generation += 1
        let attempt = generation
        stateSequence = 0; restartSequence = 0
        joinName = name
        self.context = context
        phase = .joining; error = nil; self.channelID = channelID
        do {
            guard let factory else { throw VoiceError.setup }
            guard await requestMicrophonePermission() else { throw VoiceError.permission }
            guard generation == attempt, phase == .joining else { return }
            #if os(iOS)
            try activateAudioSession()
            installAudioObservers(generation: attempt)
            #endif
            let joined: JoinResponse = try await api.media(channelID: channelID, operation: "join", body: JoinBody(name: name, muted: muted, deafened: deafened))
            guard generation == attempt, phase == .joining else {
                try? await api.media(channelID: channelID, operation: "leave", token: joined.token, body: EmptyBody())
                return
            }
            token = joined.token; selfID = joined.id
            let configuration = Self.configuration(iceServers: joined.iceServers)
            let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: ["DtlsSrtpKeyAgreement": kRTCMediaConstraintsValueTrue])
            let delegate = PeerDelegate()
            guard let peer = factory.peerConnection(with: configuration, constraints: constraints, delegate: delegate) else { throw VoiceError.setup }
            self.delegate = delegate
            self.peer = peer
            delegate.receivedAudio = { [weak self, weak peer] callbackPeer, transceiver, track in
                track.isEnabled = false
                Task { @MainActor in
                    guard let self, let peer, callbackPeer === peer, self.peer === peer,
                          self.generation == attempt, (self.phase == .connected || self.phase == .joining) else {
                        track.isEnabled = false
                        return
                    }
                    track.isEnabled = false
                    track.source.volume = 0
                    self.remoteAudio.append(track)
                    if !transceiver.mid.isEmpty {
                        self.remoteAudioByMID[transceiver.mid] = track
                        if let participantID = self.participantByMID[transceiver.mid] {
                            self.applyLocalPlayback(to: track, participantID: participantID)
                        }
                    }
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
            try checkCurrentAttempt(attempt, peer: peer)
            try await peer.setLocalDescription(offer)
            try checkCurrentAttempt(attempt, peer: peer)
            let mid = transceiver.mid
            guard let local = peer.localDescription, !mid.isEmpty else { throw VoiceError.setup }
            let published: SignalingResponse = try await api.media(channelID: channelID, operation: "publish", token: joined.token, body: PublishBody(mid: mid, sessionDescription: SDP(type: "offer", sdp: local.sdp)))
            try checkCurrentAttempt(attempt, peer: peer)
            guard let answer = published.sessionDescription else { throw VoiceError.invalidAnswer }
            try await peer.setRemoteDescription(RTCSessionDescription(type: .answer, sdp: answer.sdp))
            guard generation == attempt, phase == .joining else {
                track.isEnabled = false; peer.close()
                try? await api.media(channelID: channelID, operation: "leave", token: joined.token, body: EmptyBody())
                return
            }
            try await Self.waitForConnected(peer)
            try checkCurrentAttempt(attempt, peer: peer)
            await refreshRoster(expectedGeneration: attempt, expectedPeer: peer)
            guard generation == attempt, self.peer === peer, error == nil else { throw VoiceError.setup }
            let subscriptionID = await gateway.subscribeMedia(channelID: channelID, token: joined.token) { [weak self] event in
                self?.receiveMedia(event, generation: attempt, peer: peer)
            }
            guard generation == attempt, self.peer === peer else {
                await gateway.unsubscribe(subscriptionID)
                return
            }
            mediaSubscriptionID = subscriptionID
            publishedMID = mid
            phase = .connected
            #if os(macOS)
            audioDevice.publicationEnabled = !muted
            #endif
            track.isEnabled = !muted
            reconnectAttempts = 0
            startLeaseRenewal(generation: attempt, peer: peer)
            if let turn = joined.turn { scheduleTurnRenewal(turn, generation: attempt, peer: peer) }
        } catch {
            guard generation == attempt else { return }
            await teardown(sendLeave: true)
            guard generation == attempt else { return }
            phase = .failed; self.error = error.localizedDescription
        }
    }

    public func setMuted(_ value: Bool) async {
        if deafened {
            muteBeforeDeafen = value
            muted = true
        } else { muted = value }
        #if os(macOS)
        audioDevice.publicationEnabled = phase == .connected && !muted && comparisonGeneration == nil
        #endif
        microphone?.isEnabled = phase == .connected && !muted
        await syncState()
    }

    public func setDeafened(_ value: Bool) async {
        guard value != deafened else { return }
        if value, !deafened { muteBeforeDeafen = muted }
        deafened = value
        for (mid, track) in remoteAudioByMID {
            if let participantID = participantByMID[mid] { applyLocalPlayback(to: track, participantID: participantID) }
            else { track.isEnabled = false }
        }
        muted = value ? true : muteBeforeDeafen
        #if os(macOS)
        audioDevice.publicationEnabled = phase == .connected && !muted && comparisonGeneration == nil
        #endif
        microphone?.isEnabled = phase == .connected && !muted
        await syncState()
    }

    public func setOutputGain(_ value: Int) {
        outputGain = min(200, max(0, value))
        UserDefaults.standard.set(outputGain, forKey: "caper.voice.outputGain")
        refreshLocalPlayback()
    }

    public func setParticipantMuted(_ value: Bool, participantID: String) {
        if value { locallyMutedParticipants.insert(participantID) }
        else { locallyMutedParticipants.remove(participantID) }
        refreshLocalPlayback(participantID: participantID)
    }

    public func setParticipantGain(_ value: Int, participantID: String) {
        participantGains[participantID] = min(200, max(0, value))
        refreshLocalPlayback(participantID: participantID)
    }

    public func isSelf(participantID: String) -> Bool { participantID == selfID }

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

    public func isActive(channelID: String) -> Bool {
        context?.channelID == channelID && phase != .idle && phase != .failed
    }

    public func isActive(spaceID: String) -> Bool {
        context?.spaceID == spaceID && phase != .idle && phase != .failed
    }

    private func startLeaseRenewal(generation attempt: Int, peer expectedPeer: RTCPeerConnection) {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(15)) }
                catch { return }
                guard !Task.isCancelled, let self, self.generation == attempt,
                      self.peer === expectedPeer, self.phase == .connected else { return }
                await self.refreshRoster(expectedGeneration: attempt, expectedPeer: expectedPeer)
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
            if let apiError = error as? APIError, apiError.endsVoiceAccess {
                detachLocal(); phase = .failed; self.error = "Voice access ended. Rejoin to recover."
            } else { scheduleReconnect(generation: attempt) }
        }
    }

    private func receiveMedia(_ event: [String: Any], generation attempt: Int, peer expectedPeer: RTCPeerConnection) {
        guard generation == attempt, peer === expectedPeer else { return }
        if event["type"] as? String == "subscription.error" {
            if let status = event["status"] as? Int, [401, 403, 404].contains(status) {
                detachLocal(); phase = .failed; error = "Voice access ended. Rejoin to recover."
            } else { scheduleReconnect(generation: attempt) }
            return
        }
        guard event["type"] as? String == "snapshot",
              let data = try? JSONSerialization.data(withJSONObject: event),
              let snapshot = try? JSONDecoder().decode(VoiceSnapshot.self, from: data) else { return }
        Task { [weak self, weak expectedPeer] in
            guard let self, let expectedPeer, let token = self.token,
                  self.generation == attempt, self.peer === expectedPeer else { return }
            do { try await self.apply(snapshot: snapshot, generation: attempt, peer: expectedPeer, token: token) }
            catch {
                guard self.generation == attempt, self.peer === expectedPeer else { return }
                if let failure = error as? APIError, failure.endsVoiceAccess {
                    self.detachLocal(); self.phase = .failed; self.error = "Voice access ended. Rejoin to recover."
                } else { self.scheduleReconnect(generation: attempt) }
            }
        }
    }

    private func apply(snapshot: VoiceSnapshot, generation attempt: Int, peer: RTCPeerConnection, token: String) async throws {
        guard generation == attempt, self.peer === peer else { return }
        let callChannelID = channelID
        try await acquireSignaling(generation: attempt, peer: peer)
        defer { releaseSignaling(generation: attempt, peer: peer) }
        guard snapshotRevisions.accept(snapshot.revision) else { return }
        participants = snapshot.participants
        let liveTracks = Set(snapshot.participants.filter { $0.id != selfID }.flatMap(\.tracks).filter { $0.kind == "microphone" }.map(\.id))
        for departed in Set(subscribed.keys).subtracting(liveTracks) {
            if let mid = subscribed.removeValue(forKey: departed) {
                if let track = remoteAudioByMID.removeValue(forKey: mid) {
                    track.isEnabled = false
                    remoteAudio.removeAll { $0 === track }
                }
                participantByMID.removeValue(forKey: mid)
                try await api.media(channelID: callChannelID, operation: "close", token: token, body: CloseBody(mid: mid))
                guard generation == attempt, self.peer === peer else { return }
            }
        }
        for participant in snapshot.participants where participant.id != selfID {
            for track in participant.tracks where track.kind == "microphone" && subscribed[track.id] == nil {
                guard generation == attempt, self.peer === peer else { return }
                let response: SignalingResponse
                do { response = try await api.media(channelID: callChannelID, operation: "subscribe", token: token, body: SubscribeBody(trackId: track.id)) }
                catch let failure as APIError where failure.status == 404 && failure.code == "track_gone" {
                    try checkCurrentAttempt(attempt, peer: peer)
                    continue
                }
                try checkCurrentAttempt(attempt, peer: peer)
                guard let mid = response.tracks?.first?.mid, !mid.isEmpty else { throw VoiceError.invalidAnswer }
                if let offer = response.sessionDescription {
                    try await peer.setRemoteDescription(RTCSessionDescription(type: .offer, sdp: offer.sdp))
                    try checkCurrentAttempt(attempt, peer: peer)
                    let answer = try await peer.answer(for: RTCMediaConstraints(mandatoryConstraints: [kRTCMediaConstraintsOfferToReceiveAudio: kRTCMediaConstraintsValueTrue], optionalConstraints: nil))
                    try checkCurrentAttempt(attempt, peer: peer)
                    try await peer.setLocalDescription(answer)
                    try checkCurrentAttempt(attempt, peer: peer)
                    guard generation == attempt, self.peer === peer, let local = peer.localDescription else { return }
                    try await api.media(channelID: callChannelID, operation: "negotiate", token: token, body: NegotiateBody(sessionDescription: SDP(type: "answer", sdp: local.sdp)))
                } else if response.requiresImmediateRenegotiation == true { throw VoiceError.invalidAnswer }
                guard generation == attempt, self.peer === peer else { return }
                subscribed[track.id] = mid
                participantByMID[mid] = participant.id
                if let audio = remoteAudioByMID[mid] { applyLocalPlayback(to: audio, participantID: participant.id) }
            }
        }
    }

    private func applyLocalPlayback(to track: RTCAudioTrack, participantID: String) {
        let participantGain = participantGains[participantID] ?? 100
        track.source.volume = Double(outputGain * participantGain) / 10_000
        track.isEnabled = !deafened && !locallyMutedParticipants.contains(participantID)
    }

    private func refreshLocalPlayback(participantID: String? = nil) {
        for (mid, track) in remoteAudioByMID {
            guard let id = participantByMID[mid], participantID == nil || participantID == id else { continue }
            applyLocalPlayback(to: track, participantID: id)
        }
    }

    private func syncState() async {
        guard phase == .connected, let token, let expectedPeer = peer else { return }
        let attempt = generation
        let expectedChannelID = channelID
        stateSequence += 1
        let body = StateBody(muted: muted, deafened: deafened, sequence: stateSequence)
        do {
            try await api.media(channelID: expectedChannelID, operation: "state", token: token, body: body)
            guard generation == attempt, peer === expectedPeer else { return }
        }
        catch let failure as APIError where failure.endsVoiceAccess {
            guard generation == attempt, peer === expectedPeer else { return }
            generation += 1
            detachLocal(); phase = .failed; error = "Voice access ended. Rejoin to recover."
        } catch {
            guard generation == attempt, peer === expectedPeer else { return }
            self.error = "Voice state did not sync: \(error.localizedDescription)"
        }
    }

    private func teardown(sendLeave: Bool) async {
        let oldToken = token
        let oldChannelID = channelID
        detachLocal()
        if sendLeave, let oldToken { try? await api.media(channelID: oldChannelID, operation: "leave", token: oldToken, body: EmptyBody()) }
    }

    private func detachLocal(preservingContext: Bool = false) {
        #if os(macOS)
        audioDevice.publicationEnabled = false
        _ = audioDevice.endComparison()
        comparisonPeer = nil; comparisonGeneration = nil
        #endif
        diagnostics = nil; previousStatistics = nil
        pollTask?.cancel(); pollTask = nil
        turnTask?.cancel(); turnTask = nil
        reconnectTask?.cancel(); reconnectTask = nil
        if let mediaSubscriptionID { Task { await gateway.unsubscribe(mediaSubscriptionID) } }
        mediaSubscriptionID = nil
        microphone?.isEnabled = false
        remoteAudio.forEach { $0.isEnabled = false }
        peer?.close()
        peer = nil; delegate = nil; microphone = nil
        remoteAudio = []; remoteAudioByMID = [:]; participantByMID = [:]
        participantGains = [:]; locallyMutedParticipants = []
        token = nil; selfID = nil; subscribed = [:]; participants = []
        publishedMID = nil; snapshotRevisions = MonotonicRevision(); signalingOwner = nil
        channelID = nil
        if !preservingContext { context = nil }
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
        let oldToken = token, oldChannelID = channelID, oldName = joinName, oldContext = context
        generation = nextAttempt
        phase = .reconnecting
        detachLocal(preservingContext: true)
        phase = .reconnecting
        if let oldToken { Task { [api] in try? await api.media(channelID: oldChannelID, operation: "leave", token: oldToken, body: EmptyBody()) } }
        reconnectTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(min(pow(2, Double(self?.reconnectAttempts ?? 1)), 5)))
            guard !Task.isCancelled, let self, self.generation == nextAttempt, self.phase == .reconnecting else { return }
            self.reconnectTask = nil
            self.phase = .idle
            guard let oldContext else { return }
            await self.join(channelID: oldChannelID, context: oldContext, name: oldName)
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
        let callChannelID = channelID
        do {
            let response: TurnResponse = try await retryControl(generation: attempt, peer: peer) {
                try await self.api.media(channelID: callChannelID, operation: "turn", token: token, body: TurnRequest(generation: current.generation))
            }
            guard generation == attempt, self.peer === peer else { return }
            if response.turn.generation == current.generation {
                scheduleTurnRenewal(response.turn, generation: attempt, peer: peer)
                return
            }
            try await acquireSignaling(generation: attempt, peer: peer)
            defer { releaseSignaling(generation: attempt, peer: peer) }
            let configuration = peer.configuration
            configuration.iceServers = response.iceServers.map { RTCIceServer(urlStrings: $0.urls, username: $0.username, credential: $0.credential) }
            guard peer.setConfiguration(configuration) else { throw VoiceError.setup }
            restartSequence += 1
            let sequence = restartSequence
            peer.restartIce()
            let offer = try await peer.offer(for: RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil))
            try checkCurrentAttempt(attempt, peer: peer)
            try await peer.setLocalDescription(offer)
            try checkCurrentAttempt(attempt, peer: peer)
            guard generation == attempt, self.peer === peer, let local = peer.localDescription else { throw CancellationError() }
            let restarted: SignalingResponse = try await retryControl(generation: attempt, peer: peer) {
                try await self.api.media(channelID: callChannelID, operation: "restart-ice", token: token, body: RestartBody(generation: response.turn.generation, sequence: sequence, sessionDescription: SDP(type: "offer", sdp: local.sdp)))
            }
            guard generation == attempt, self.peer === peer else { return }
            guard let answer = restarted.sessionDescription else { throw VoiceError.invalidAnswer }
            try await peer.setRemoteDescription(RTCSessionDescription(type: .answer, sdp: answer.sdp))
            guard generation == attempt, self.peer === peer else { return }
            try await retryControl(generation: attempt, peer: peer) {
                try await self.api.media(channelID: callChannelID, operation: "restart-ice-ack", token: token, body: RestartAckBody(generation: response.turn.generation, sequence: sequence))
            }
            guard generation == attempt, self.peer === peer else { return }
            scheduleTurnRenewal(response.turn, generation: attempt, peer: peer)
        } catch {
            guard generation == attempt, self.peer === peer else { return }
            releaseSignaling(generation: attempt, peer: peer)
            if let failure = error as? APIError, failure.endsVoiceAccess {
                detachLocal(); phase = .failed; self.error = "Voice access ended. Rejoin to recover."
            } else { scheduleReconnect(generation: attempt) }
        }
    }

    private func checkCurrentAttempt(_ attempt: Int, peer: RTCPeerConnection) throws {
        try Task.checkCancellation()
        guard generation == attempt, self.peer === peer else { throw CancellationError() }
    }

    private func acquireSignaling(generation attempt: Int, peer: RTCPeerConnection) async throws {
        while signalingOwner != nil {
            try Task.checkCancellation()
            guard generation == attempt, self.peer === peer else { throw CancellationError() }
            try await Task.sleep(for: .milliseconds(25))
        }
        guard generation == attempt, self.peer === peer else { throw CancellationError() }
        signalingOwner = attempt
    }

    private func releaseSignaling(generation attempt: Int, peer: RTCPeerConnection) {
        if signalingOwner == attempt, self.peer === peer { signalingOwner = nil }
    }

    private func retryControl<T>(generation expectedGeneration: Int, peer expectedPeer: RTCPeerConnection, _ operation: () async throws -> T) async throws -> T {
        var delay = 1.0
        for attempt in 0..<4 {
            try Task.checkCancellation()
            guard generation == expectedGeneration, peer === expectedPeer else { throw CancellationError() }
            do {
                let result = try await operation()
                guard generation == expectedGeneration, peer === expectedPeer else { throw CancellationError() }
                return result
            }
            catch is CancellationError { throw CancellationError() }
            catch let failure as APIError {
                guard failure.retryableVoiceControl, attempt < 3 else { throw failure }
            } catch let error as URLError {
                guard attempt < 3 else { throw error }
            }
            try await Task.sleep(for: .seconds(delay)); delay *= 2
        }
        throw VoiceError.timeout
    }

    public func refreshAudioDevices() async {
        #if os(iOS)
        let audio = AVAudioSession.sharedInstance()
        availableInputs = (audio.availableInputs ?? []).map { AudioDevice(id: $0.uid, name: $0.portName) }
        availableOutputs = audio.currentRoute.outputs.map { AudioDevice(id: $0.uid, name: $0.portName) }
        selectedInputID = audio.currentRoute.inputs.first?.uid
        selectedOutputID = audio.currentRoute.outputs.first?.uid
        #else
        availableInputs = CaperMacAudioDevice.inputRoutes().map { AudioDevice(id: $0.uid, name: $0.name) }
        availableOutputs = CaperMacAudioDevice.outputRoutes().map { AudioDevice(id: $0.uid, name: $0.name) }
        selectedInputID = audioDevice.inputUID
        selectedOutputID = audioDevice.outputUID
        #endif
    }

    #if os(macOS)
    /// Publication stays gated before/during comparison; stale tests cannot open a replacement call.
    func beginMicrophoneComparison() -> Bool {
        guard comparisonGeneration == nil else { return false }
        if phase == .idle || phase == .failed { return audioDevice.beginComparison() }
        guard phase == .connected, let peer else { return false }
        audioDevice.publicationEnabled = false
        guard audioDevice.beginComparison() else {
            audioDevice.publicationEnabled = !muted
            return false
        }
        comparisonPeer = peer; comparisonGeneration = generation
        return true
    }

    func endMicrophoneComparison() -> CaperAudioComparison? {
        let samples = audioDevice.endComparison()
        if let attempt = comparisonGeneration, attempt == generation, phase == .connected,
           let oldPeer = comparisonPeer, oldPeer === peer {
            audioDevice.publicationEnabled = !muted
        }
        comparisonPeer = nil; comparisonGeneration = nil
        return samples
    }

    func comparisonOutputDeviceID() -> UInt32 { audioDevice.resolvedOutputDeviceID }

    @discardableResult public func selectInput(_ uid: String) -> Bool {
        guard audioDevice.selectInputUID(uid) else { return false }
        selectedInputID = uid
        UserDefaults.standard.set(uid, forKey: "caper.voice.inputUID")
        return true
    }

    @discardableResult public func selectOutput(_ uid: String) -> Bool {
        guard audioDevice.selectOutputUID(uid) else { return false }
        selectedOutputID = uid
        UserDefaults.standard.set(uid, forKey: "caper.voice.outputUID")
        return true
    }

    public func setInputGain(_ value: Int) {
        inputGain = min(200, max(0, value))
        audioDevice.inputGain = inputGain
        UserDefaults.standard.set(inputGain, forKey: "caper.voice.inputGain")
    }

    public func setVoiceProcessingStrength(_ value: Int) {
        voiceProcessingStrength = min(100, max(0, value))
        audioDevice.processingStrength = voiceProcessingStrength
        UserDefaults.standard.set(voiceProcessingStrength, forKey: "caper.voice.processingStrength")
    }
    #endif

    public func refreshDiagnostics() async {
        guard phase == .connected, let peer else { return }
        let attempt = generation
        let report: RTCStatisticsReport = await withCheckedContinuation { continuation in
            peer.statistics { continuation.resume(returning: $0) }
        }
        guard generation == attempt, self.peer === peer, phase == .connected else { return }
        let stats = report.statistics.mapValues { VoiceStatistic(type: $0.type, values: $0.values) }
        let (current, sample) = VoiceDiagnostics.read(stats, timestampUs: report.timestamp_us, previous: previousStatistics)
        diagnostics = current
        previousStatistics = sample
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

    private func installAudioObservers(generation attempt: Int) {
        removeAudioObservers()
        let center = NotificationCenter.default
        audioObservers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
            Task { @MainActor in
                guard self?.generation == attempt else { return }
                self?.handleAudioInterruption(note, generation: attempt)
            }
        })
        audioObservers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in
                guard self?.generation == attempt else { return }
                await self?.refreshAudioDevices()
            }
        })
        audioObservers.append(center.addObserver(forName: AVAudioSession.mediaServicesWereLostNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in
                guard let self, self.generation == attempt else { return }
                self.microphone?.isEnabled = false
            }
        })
        audioObservers.append(center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in
                guard let self, self.generation == attempt, self.phase == .connected else { return }
                do {
                    try self.activateAudioSession()
                    guard self.generation == attempt, self.phase == .connected else { return }
                    self.microphone?.isEnabled = !self.muted
                } catch { self.scheduleReconnect(generation: attempt) }
            }
        })
    }

    private func removeAudioObservers() {
        let center = NotificationCenter.default
        audioObservers.forEach(center.removeObserver)
        audioObservers = []
    }

    private func handleAudioInterruption(_ note: Notification, generation attempt: Int) {
        guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
              let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
        if type == .began { microphone?.isEnabled = false }
        else if phase == .connected,
                let optionsRaw = note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt,
                AVAudioSession.InterruptionOptions(rawValue: optionsRaw).contains(.shouldResume) {
            do { try activateAudioSession(); microphone?.isEnabled = !muted }
            catch { scheduleReconnect(generation: attempt) }
        } else if phase == .connected {
            muted = true
            microphone?.isEnabled = false
            error = "Audio was interrupted. Unmute when you are ready to resume."
            Task { [weak self] in
                guard let self, self.generation == attempt else { return }
                await self.syncState()
            }
        }
    }
    #endif

    static func waitForConnected(_ peer: RTCPeerConnection) async throws {
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
