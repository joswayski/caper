import Foundation
import Observation

@MainActor @Observable
public final class PresenceModel {
    public var query = ""
    public var online = false
    public var error: String?
    public private(set) var statuses: [String: PresenceStatus] = [:]

    private let api: APIClient
    @ObservationIgnored private lazy var gateway = Gateway(baseURL: api.baseURL, token: { [api] in await api.authorizationToken() }) { [weak self] state, message in
        guard let self, self.spaceID != nil else { return }
        self.online = state == .connected
        if state != .connected { self.statuses = [:] }
        if state == .connected, let message { self.error = message }
    }
    private var subscriptionID: String?
    private var generation = 0
    private var spaceID: String?
    private var members: [Member] = []

    public init(api: APIClient) {
        self.api = api
    }

    public var visibleMembers: [Member] {
        let filter = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return members.filter { filter.isEmpty || $0.displayName.localizedCaseInsensitiveContains(filter) || $0.username.localizedCaseInsensitiveContains(filter) }
    }

    public func status(for member: Member) -> PresenceStatus { statuses[member.id] ?? .unknown }

    public func watch(spaceID: String, members: [Member]) async {
        let sameSpace = self.spaceID == spaceID
        self.members = members
        // Channels share the space roster. Refresh names/avatars without
        // dropping live statuses or restarting the unchanged subscription.
        if sameSpace { return }
        query = ""
        generation += 1
        let attempt = generation
        let previous = subscriptionID
        subscriptionID = nil
        self.spaceID = spaceID
        statuses = [:]; online = false; error = nil
        if let previous { await gateway.unsubscribe(previous) }
        await subscribe(generation: attempt)
    }

    public func stop() async {
        generation += 1
        let previous = subscriptionID
        subscriptionID = nil; spaceID = nil; members = []; statuses = [:]
        query = ""; online = false; error = nil
        if let previous { await gateway.unsubscribe(previous) }
    }

    private func subscribe(generation attempt: Int) async {
        guard let spaceID else { return }
        let subscription = await gateway.subscribeSpacePresence(spaceID: spaceID) { [weak self] event in
            self?.receive(event, generation: attempt)
        }
        guard generation == attempt, self.spaceID == spaceID else {
            await gateway.unsubscribe(subscription)
            return
        }
        subscriptionID = subscription
    }

    func receive(_ event: [String: Any], generation attempt: Int) {
        guard generation == attempt else { return }
        if event["type"] as? String == "subscription.error" {
            statuses = [:]; online = false
            error = event["error"] as? String ?? "Member presence is unavailable."
            return
        }
        guard event["type"] as? String == "snapshot", let rawMembers = event["members"] as? [[String: Any]] else { return }
        var next: [String: PresenceStatus] = [:]
        for value in rawMembers {
            guard let id = value["userId"] as? String,
                  let rawStatus = value["status"] as? String,
                  let status = PresenceStatus(rawValue: rawStatus) else {
                error = "Member presence returned invalid data."
                return
            }
            next[id] = status
        }
        statuses = next; online = true; error = nil
    }
}
