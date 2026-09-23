import Foundation

public enum GatewayState: Equatable, Sendable { case disconnected, connecting, connected, reconnecting }

public struct GatewayEvent: Sendable {
    public let subscriptionID: String
    public let value: [String: AnySendable]
}

public struct AnySendable: @unchecked Sendable {
    public let value: Any
    public init(_ value: Any) { self.value = value }
}

public actor Gateway {
    public typealias Handler = @MainActor @Sendable ([String: Any]) -> Void
    public typealias StateHandler = @MainActor @Sendable (GatewayState, String?) -> Void

    private let baseURL: URL
    private let session: URLSession
    private let token: @Sendable () async -> String?
    private var socket: URLSessionWebSocketTask?
    private var subscriptions: [String: (frame: [String: Any], handler: Handler)] = [:]
    private var runTask: Task<Void, Never>?
    private var heartbeatTask: Task<Void, Never>?
    private var attempts = 0
    private var stopped = false
    private var lastActivity = ContinuousClock.now
    private var lastServerHeartbeat = ContinuousClock.now
    private var epoch: UInt64 = 0
    private var readyEpoch: UInt64?
    private let state: StateHandler

    public init(baseURL: URL, session: URLSession? = nil, token: @escaping @Sendable () async -> String?, state: @escaping StateHandler) {
        self.baseURL = baseURL; self.session = session ?? SecureSession.make(); self.token = token; self.state = state
    }

    @discardableResult
    public func subscribeChat(channelID: String, after: String, handler: @escaping Handler) async -> String {
        let id = UUID().uuidString
        subscriptions[id] = (["type": "subscribe", "id": id, "kind": "chat", "channelId": channelID, "after": after], handler)
        if socket == nil { connect() }
        else if readyEpoch == epoch { try? await send(subscriptions[id]!.frame) }
        return id
    }

    public func unsubscribe(_ id: String) async {
        subscriptions.removeValue(forKey: id)
        if readyEpoch == epoch { try? await send(["type": "unsubscribe", "id": id]) }
        if subscriptions.isEmpty { stopSocket() }
    }

    public func updateCursor(subscription id: String, after: String) {
        guard var subscription = subscriptions[id] else { return }
        subscription.frame["after"] = after
        subscriptions[id] = subscription
    }

    public func reportActivity() { lastActivity = ContinuousClock.now }

    public func stop() {
        stopped = true
        subscriptions.removeAll()
        stopSocket()
    }

    private func connect() {
        guard socket == nil, !subscriptions.isEmpty else { return }
        stopped = false
        epoch &+= 1
        let connectionEpoch = epoch
        runTask?.cancel()
        runTask = Task { [weak self] in await self?.run(epoch: connectionEpoch) }
    }

    private func run(epoch connectionEpoch: UInt64) async {
        await state(attempts == 0 ? .connecting : .reconnecting, nil)
        guard isCurrent(connectionEpoch) else { return }
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else { return }
        components.scheme = components.scheme == "http" ? "ws" : "wss"
        components.path = "/api/chat/events"
        guard let url = components.url else { return }
        var request = URLRequest(url: url, timeoutInterval: 15)
        if let token = await token() { request.setValue("Bearer \(token)", forHTTPHeaderField: "authorization") }
        guard isCurrent(connectionEpoch) else { return }
        let current = session.webSocketTask(with: request)
        socket = current
        current.resume()
        startOpenWatchdog(epoch: connectionEpoch, socket: current)
        do {
            while !Task.isCancelled {
                guard isCurrent(connectionEpoch, socket: current) else { return }
                let message = try await current.receive()
                guard isCurrent(connectionEpoch, socket: current) else { current.cancel(with: .normalClosure, reason: nil); return }
                let data: Data
                switch message { case .string(let text): data = Data(text.utf8); case .data(let value): data = value; @unknown default: continue }
                guard data.count <= 256 * 1024,
                      let frame = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let type = frame["type"] as? String else { throw URLError(.cannotParseResponse) }
                if type == "hello" {
                    guard readyEpoch != connectionEpoch else { throw URLError(.cannotParseResponse) }
                    attempts = 0
                    readyEpoch = connectionEpoch
                    lastServerHeartbeat = .now
                    await state(.connected, nil)
                    guard isCurrent(connectionEpoch, socket: current) else { return }
                    for subscription in subscriptions.values { try await send(subscription.frame, on: current) }
                    startHeartbeat(epoch: connectionEpoch, socket: current)
                } else if type == "heartbeat" {
                    lastServerHeartbeat = .now
                    continue
                } else if type == "event", let id = frame["id"] as? String, let event = frame["event"] as? [String: Any], let handler = subscriptions[id]?.handler {
                    await handler(event)
                } else if type == "error", let id = frame["id"] as? String {
                    let message = frame["error"] as? String ?? "Live updates failed."
                    let status = frame["status"] as? Int ?? 0
                    let handler = subscriptions[id]?.handler
                    subscriptions.removeValue(forKey: id)
                    await handler?(["type": "subscription.error", "status": status, "error": message])
                    guard isCurrent(connectionEpoch, socket: current) else { return }
                    await state(.connected, message)
                } else if type == "migrating" { throw URLError(.networkConnectionLost) }
            }
        } catch {
            guard isCurrent(connectionEpoch, socket: current) else { return }
            socket = nil
            readyEpoch = nil
            heartbeatTask?.cancel()
            guard !stopped, !subscriptions.isEmpty else { return }
            attempts += 1
            await state(.reconnecting, "Live updates disconnected. Reconnecting…")
            guard isCurrent(connectionEpoch), socket == nil else { return }
            let delay = min(pow(2, Double(min(attempts, 5))) * 0.25, 5)
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled, isCurrent(connectionEpoch), socket == nil else { return }
            connect()
        }
    }

    private func startHeartbeat(epoch connectionEpoch: UInt64, socket current: URLSessionWebSocketTask) {
        heartbeatTask?.cancel()
        heartbeatTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(10))
                guard let self else { return }
                guard await self.heartbeat(epoch: connectionEpoch, socket: current) else { return }
            }
        }
    }

    private func startOpenWatchdog(epoch connectionEpoch: UInt64, socket current: URLSessionWebSocketTask) {
        heartbeatTask?.cancel()
        heartbeatTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(10))
            guard !Task.isCancelled, let self else { return }
            await self.expireUnready(epoch: connectionEpoch, socket: current)
        }
    }

    private func expireUnready(epoch connectionEpoch: UInt64, socket current: URLSessionWebSocketTask) {
        guard isCurrent(connectionEpoch, socket: current), readyEpoch != connectionEpoch else { return }
        current.cancel(with: .goingAway, reason: nil)
    }

    private func heartbeat(epoch connectionEpoch: UInt64, socket current: URLSessionWebSocketTask) async -> Bool {
        guard isCurrent(connectionEpoch, socket: current) else { return false }
        if lastServerHeartbeat.duration(to: .now) > .seconds(30) {
            current.cancel(with: .goingAway, reason: nil)
            return false
        }
        let elapsed = lastActivity.duration(to: .now).components
        let age = max(0, Int(elapsed.seconds * 1_000 + elapsed.attoseconds / 1_000_000_000_000_000))
        do { try await send(["type": "heartbeat", "activityAgeMs": age], on: current); return true }
        catch { current.cancel(with: .goingAway, reason: nil); return false }
    }

    private func send(_ value: [String: Any], on target: URLSessionWebSocketTask? = nil) async throws {
        guard let target = target ?? socket else { return }
        let data = try JSONSerialization.data(withJSONObject: value)
        let text = String(decoding: data, as: UTF8.self)
        try await target.send(.string(text))
    }

    private func stopSocket() {
        epoch &+= 1
        let stoppedEpoch = epoch
        readyEpoch = nil
        runTask?.cancel(); heartbeatTask?.cancel()
        runTask = nil; heartbeatTask = nil
        socket?.cancel(with: .normalClosure, reason: nil)
        socket = nil
        Task { [weak self] in await self?.publishDisconnected(epoch: stoppedEpoch) }
    }

    private func publishDisconnected(epoch stoppedEpoch: UInt64) async {
        guard epoch == stoppedEpoch, socket == nil else { return }
        await state(.disconnected, nil)
    }

    private func isCurrent(_ connectionEpoch: UInt64, socket expected: URLSessionWebSocketTask? = nil) -> Bool {
        guard !stopped, !Task.isCancelled, epoch == connectionEpoch else { return false }
        return expected.map { socket === $0 } ?? true
    }
}
