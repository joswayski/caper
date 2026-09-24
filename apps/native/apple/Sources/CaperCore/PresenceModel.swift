import Foundation
import Observation

@MainActor @Observable
public final class PresenceModel {
    public var page = 0
    public var online = false
    public var error: String?
    public private(set) var statuses: [String: PresenceStatus] = [:]

    public let pageSize = 25
    private let api: APIClient
    @ObservationIgnored private lazy var gateway = Gateway(baseURL: api.baseURL, token: { [api] in await api.authorizationToken() }) { [weak self] state, message in
        self?.online = state == .connected
        if let message { self?.error = message }
    }
    private var subscriptionID: String?
    private var generation = 0
    private var spaceID: String?
    private var members: [Member] = []

    public init(api: APIClient) {
        self.api = api
    }

    public var pageCount: Int { max(1, Int(ceil(Double(members.count) / Double(pageSize)))) }
    public var visibleMembers: [Member] {
        let start = min(page * pageSize, members.count)
        return Array(members.dropFirst(start).prefix(pageSize))
    }

    public func status(for member: Member) -> PresenceStatus { statuses[member.id] ?? .unknown }

    public func watch(spaceID: String, members: [Member]) async {
        generation += 1
        let attempt = generation
        let previous = subscriptionID
        subscriptionID = nil
        self.spaceID = spaceID
        self.members = members
        page = 0; statuses = [:]; online = false; error = nil
        if let previous { await gateway.unsubscribe(previous) }
        await subscribe(generation: attempt)
    }

    public func showPage(_ value: Int) async {
        let next = max(0, min(value, pageCount - 1))
        guard next != page else { return }
        page = next
        generation += 1
        let attempt = generation
        let previous = subscriptionID
        subscriptionID = nil; statuses = [:]; online = false; error = nil
        if let previous { await gateway.unsubscribe(previous) }
        await subscribe(generation: attempt)
    }

    public func stop() async {
        generation += 1
        let previous = subscriptionID
        subscriptionID = nil; spaceID = nil; members = []; statuses = [:]
        page = 0; online = false; error = nil
        if let previous { await gateway.unsubscribe(previous) }
    }

    private func subscribe(generation attempt: Int) async {
        guard let spaceID, !visibleMembers.isEmpty else { return }
        let memberIDs = visibleMembers.map(\.id)
        let subscription = await gateway.subscribePresence(spaceID: spaceID, userIDs: memberIDs) { [weak self] event in
            self?.receive(event, generation: attempt, expectedIDs: Set(memberIDs))
        }
        guard generation == attempt, self.spaceID == spaceID else {
            await gateway.unsubscribe(subscription)
            return
        }
        subscriptionID = subscription
    }

    private func receive(_ event: [String: Any], generation attempt: Int, expectedIDs: Set<String>) {
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
                  expectedIDs.contains(id),
                  let rawStatus = value["status"] as? String,
                  let status = PresenceStatus(rawValue: rawStatus) else {
                error = "Member presence returned invalid data."
                return
            }
            next[id] = status
        }
        guard Set(next.keys) == expectedIDs else {
            error = "Member presence returned an incomplete page."
            return
        }
        statuses = next; online = true; error = nil
    }
}
