import Foundation
import Observation

public struct VoiceSpectator: Decodable, Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let muted: Bool
    public let deafened: Bool
    public let countryCode: String?
}

/// Read-only occupancy. Its one Gateway multiplexes at most 24 token-free media
/// subscriptions; VoiceClient's participant-token subscription owns actual tracks.
@MainActor @Observable
public final class VoicePresenceModel {
    public private(set) var rosters: [String: [VoiceSpectator]] = [:]
    public private(set) var online = false
    private let api: APIClient
    @ObservationIgnored private lazy var gateway = Gateway(baseURL: api.baseURL, token: { [api] in await api.authorizationToken() }) { [weak self] state, _ in
        self?.online = state == .connected && self?.spaceID != nil
        if state != .connected { self?.rosters = [:]; self?.revisions = [:] }
    }
    private var subscriptions: [String: String] = [:]
    private var watching: Set<String> = []
    private var revisions: [String: MonotonicRevision] = [:]
    private var generation = 0
    private var spaceID: String?

    public init(api: APIClient) { self.api = api }

    public func roster(for channelID: String) -> [VoiceSpectator] { rosters[channelID] ?? [] }

    public func watch(spaceID: String, channels: [Channel], demo: Bool) async {
        let wanted = Array(channels.prefix(demo ? 1 : 24)).map(\.id)
        if self.spaceID == spaceID, watching == Set(wanted) { return }
        generation += 1
        let attempt = generation
        let previous = subscriptions.values
        subscriptions = [:]; watching = Set(wanted); rosters = [:]; revisions = [:]; online = false
        self.spaceID = spaceID
        for id in previous { await gateway.unsubscribe(id) }
        guard generation == attempt, self.spaceID == spaceID else { return }
        for channelID in wanted {
            let subscription = await gateway.subscribeMedia(channelID: demo ? nil : channelID, token: nil) { [weak self] event in
                self?.receive(event, generation: attempt, channelID: channelID)
            }
            guard generation == attempt, self.spaceID == spaceID, watching.contains(channelID) else {
                await gateway.unsubscribe(subscription)
                return
            }
            subscriptions[channelID] = subscription
        }
    }

    public func stop() async {
        generation += 1
        let previous = subscriptions.values
        subscriptions = [:]; watching = []; rosters = [:]; revisions = [:]; spaceID = nil; online = false
        for id in previous { await gateway.unsubscribe(id) }
    }

    public func revoke(channelID: String) {
        watching.remove(channelID)
        rosters.removeValue(forKey: channelID)
        revisions.removeValue(forKey: channelID)
        if let id = subscriptions.removeValue(forKey: channelID) {
            Task { await gateway.unsubscribe(id) }
        }
    }

    func receive(_ event: [String: Any], generation attempt: Int, channelID: String) {
        guard generation == attempt, watching.contains(channelID) else { return }
        if event["type"] as? String == "subscription.error" {
            revoke(channelID: channelID)
            return
        }
        guard event["type"] as? String == "snapshot",
              let values = event["participants"] as? [[String: Any]],
              let data = try? JSONSerialization.data(withJSONObject: values),
              let people = try? JSONDecoder().decode([VoiceSpectator].self, from: data),
              people.count <= 256, Set(people.map(\.id)).count == people.count else { return }
        var revision = revisions[channelID] ?? MonotonicRevision()
        guard revision.accept(event["revision"] as? Int) else { return }
        revisions[channelID] = revision
        rosters[channelID] = people
    }
}
