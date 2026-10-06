import Foundation

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

enum GatewaySocketMessage: Sendable {
    case string(String)
    case data(Data)
}

protocol GatewaySocket: AnyObject, Sendable {
    func resume()
    func receive() async throws -> GatewaySocketMessage
    func send(_ message: GatewaySocketMessage) async throws
    func cancel()
}

private final class URLSessionGatewaySocket: GatewaySocket, @unchecked Sendable {
    private let task: URLSessionWebSocketTask
    init(_ task: URLSessionWebSocketTask) { self.task = task }
    func resume() { task.resume() }
    func receive() async throws -> GatewaySocketMessage {
        switch try await task.receive() {
        case .string(let value): return .string(value)
        case .data(let value): return .data(value)
        @unknown default: throw URLError(.cannotParseResponse)
        }
    }
    func send(_ message: GatewaySocketMessage) async throws {
        switch message {
        case .string(let value): try await task.send(.string(value))
        case .data(let value): try await task.send(.data(value))
        }
    }
    func cancel() { task.cancel(with: .normalClosure, reason: nil) }
}

public enum GatewayState: Equatable, Sendable { case disconnected, connecting, connected, reconnecting }

public struct GatewayFailure: LocalizedError, Equatable, Sendable {
    public let status: Int
    public let message: String
    public let code: String?
    public var errorDescription: String? { message }
}

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

    private struct Subscription {
        var frame: [String: Any]
        let handler: Handler
        var appliedPosition: UInt64?
        var revision: Int?
    }

    private struct StreamSubscription {
        var subscribed = false
        var chatReady = false
        var position: UInt64?
        var snapshotRevision: Int?
        var pendingPresence: [String: Any]?
    }

    private final class Stream: @unchecked Sendable {
        let id: UInt64
        let socket: any GatewaySocket
        var hello = false
        var closed = false
        var subscriptions: [String: StreamSubscription] = [:]
        var receiveTask: Task<Void, Never>?
        var heartbeatTask: Task<Void, Never>?
        var watchdogTask: Task<Void, Never>?
        var catchupTask: Task<Void, Never>?
        init(id: UInt64, socket: any GatewaySocket) { self.id = id; self.socket = socket }
    }

    private struct PendingCommand {
        let frame: [String: Any]
        let continuation: CheckedContinuation<[String: Any], Error>
        let deadline: ContinuousClock.Instant
        var retries = 0
        var retryTask: Task<Void, Never>?
        let timeoutTask: Task<Void, Never>
    }

    private let baseURL: URL
    private let session: URLSession
    private let socketFactory: @Sendable (URLRequest) -> any GatewaySocket
    private let openTimeout: Duration
    private let catchupTimeout: Duration
    private let token: @Sendable () async -> String?
    private let state: StateHandler
    private var subscriptions: [String: Subscription] = [:]
    private var commands: [String: PendingCommand] = [:]
    private var active: Stream?
    private var candidate: Stream?
    private var openingTask: Task<Void, Never>?
    private var reconnectTask: Task<Void, Never>?
    private var attempts = 0
    private var stopped = false
    private var generation: UInt64 = 0
    private var lastActivity = ContinuousClock.now

    public init(baseURL: URL, session: URLSession? = nil, token: @escaping @Sendable () async -> String?, state: @escaping StateHandler) {
        let resolvedSession = session ?? SecureSession.make()
        self.baseURL = baseURL; self.session = resolvedSession
        self.socketFactory = { URLSessionGatewaySocket(resolvedSession.webSocketTask(with: $0)) }
        self.openTimeout = .seconds(10)
        self.catchupTimeout = .seconds(15)
        self.token = token; self.state = state
    }

    init(baseURL: URL, socketFactory: @escaping @Sendable (URLRequest) -> any GatewaySocket,
         openTimeout: Duration = .seconds(10), catchupTimeout: Duration = .seconds(15),
         token: @escaping @Sendable () async -> String?, state: @escaping StateHandler) {
        self.baseURL = baseURL; self.session = SecureSession.make(); self.socketFactory = socketFactory
        self.openTimeout = openTimeout; self.catchupTimeout = catchupTimeout
        self.token = token; self.state = state
    }

    @discardableResult
    public func subscribeChat(channelID: String, after: String, handler: @escaping Handler) async -> String {
        await subscribe(frame: ["kind": "chat", "channelId": channelID, "after": after], handler: handler)
    }

    @discardableResult
    public func subscribePresence(spaceID: String, userIDs: [String], handler: @escaping Handler) async -> String {
        await subscribe(frame: ["kind": "presence", "spaceId": spaceID, "userIds": userIDs], handler: handler)
    }

    @discardableResult
    public func subscribeMedia(channelID: String?, token: String?, handler: @escaping Handler) async -> String {
        var frame: [String: Any] = ["kind": "media"]
        if let channelID { frame["channelId"] = channelID }
        if let token { frame["token"] = token }
        return await subscribe(frame: frame, handler: handler)
    }

    private func subscribe(frame: [String: Any], handler: @escaping Handler) async -> String {
        let id = UUID().uuidString
        var request = frame
        request["type"] = "subscribe"; request["id"] = id
        let position = (frame["kind"] as? String) == "chat" ? sequence(frame["after"]) : nil
        subscriptions[id] = Subscription(frame: request, handler: handler, appliedPosition: position, revision: nil)
        stopped = false
        if active == nil { open(replacement: false) }
        if let active, active.hello { try? await sendSubscription(id, on: active) }
        if let candidate, candidate.hello { try? await sendSubscription(id, on: candidate) }
        return id
    }

    public func command(method: String, channelID: String? = nil, token capability: String? = nil,
                        chatToken: String? = nil, body: [String: Any] = [:], timeout: Duration = .seconds(2)) async throws -> [String: Any] {
        if method == "typing", active?.hello != true {
            throw GatewayFailure(status: 503, message: "Typing is unavailable while reconnecting.", code: nil)
        }
        let id = UUID().uuidString.lowercased()
        var frame: [String: Any] = ["type": "command", "id": id, "method": method,
                                    "issuedAt": Int(Date().timeIntervalSince1970 * 1_000), "body": body]
        if let channelID { frame["channelId"] = channelID }
        if let capability { frame["token"] = capability }
        if let chatToken { frame["chatToken"] = chatToken }
        if active == nil { stopped = false; open(replacement: false) }
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let deadline = ContinuousClock.now.advanced(by: timeout)
                let timeoutTask = Task { [weak self] in
                    try? await Task.sleep(for: timeout)
                    guard !Task.isCancelled else { return }
                    await self?.finishCommand(id: id, error: CancellationError())
                }
                commands[id] = PendingCommand(frame: frame, continuation: continuation, deadline: deadline, timeoutTask: timeoutTask)
                if let active, active.hello { Task { try? await self.send(frame, on: active) } }
            }
        } onCancel: { Task { [weak self] in await self?.finishCommand(id: id, error: CancellationError()) } }
    }

    public func unsubscribe(_ id: String) async {
        subscriptions.removeValue(forKey: id)
        if let active, active.hello { try? await send(["type": "unsubscribe", "id": id], on: active) }
        if let candidate, candidate.hello { try? await send(["type": "unsubscribe", "id": id], on: candidate) }
        active?.subscriptions.removeValue(forKey: id); candidate?.subscriptions.removeValue(forKey: id)
        if subscriptions.isEmpty && commands.isEmpty { stopSockets(publish: true) }
        else if let candidate { await maybePromote(candidate) }
    }

    public func updateCursor(subscription id: String, after: String) {
        guard var subscription = subscriptions[id], let cursor = sequence(after) else { return }
        subscription.frame["after"] = after
        subscription.appliedPosition = max(subscription.appliedPosition ?? 0, cursor)
        subscriptions[id] = subscription
        if let candidate { Task { await self.maybePromote(candidate) } }
    }

    public func reportActivity() { lastActivity = .now }

    public func stop() {
        stopped = true; subscriptions.removeAll()
        for id in Array(commands.keys) { finishCommand(id: id, error: CancellationError()) }
        stopSockets(publish: true)
    }

    private func open(replacement: Bool) {
        guard !stopped, openingTask == nil, reconnectTask == nil,
              replacement ? (active != nil && candidate == nil) : active == nil else { return }
        generation &+= 1
        let requestGeneration = generation
        openingTask = Task { [weak self] in await self?.authorizeAndOpen(replacement: replacement, generation: requestGeneration) }
        if !replacement { Task { await state(attempts == 0 ? .connecting : .reconnecting, nil) } }
    }

    private func authorizeAndOpen(replacement: Bool, generation requestGeneration: UInt64) async {
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else { clearOpening(generation: requestGeneration); return }
        components.scheme = components.scheme == "http" ? "ws" : "wss"; components.path = "/api/chat/events"
        guard let url = components.url else { clearOpening(generation: requestGeneration); return }
        let bearer = await token()
        guard !Task.isCancelled, !stopped, generation == requestGeneration,
              replacement ? (active != nil && candidate == nil) : active == nil else {
            clearOpening(generation: requestGeneration); return
        }
        var request = URLRequest(url: url, timeoutInterval: 15)
        if let bearer { request.setValue("Bearer \(bearer)", forHTTPHeaderField: "authorization") }
        let socket = socketFactory(request)
        let stream = Stream(id: requestGeneration, socket: socket)
        if replacement {
            candidate = stream
            let delay = catchupTimeout
            stream.catchupTask = Task { [weak self, weak stream] in
                try? await Task.sleep(for: delay)
                guard !Task.isCancelled, let self, let stream else { return }
                await self.failed(stream)
            }
        } else { active = stream }
        openingTask = nil; socket.resume()
        stream.receiveTask = Task { [weak self, weak stream] in
            guard let self, let stream else { return }
            await self.receiveLoop(stream)
        }
        armOpenWatchdog(stream)
    }

    private func clearOpening(generation requestGeneration: UInt64) {
        guard generation == requestGeneration else { return }
        openingTask = nil
    }

    private func receiveLoop(_ stream: Stream) async {
        do {
            while !Task.isCancelled && !stream.closed {
                let message = try await stream.socket.receive()
                guard owns(stream), !stream.closed else { return }
                let data: Data
                switch message { case .string(let text): data = Data(text.utf8); case .data(let value): data = value }
                guard data.count <= 256 * 1024, let frame = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw URLError(.cannotParseResponse) }
                try await receive(frame, from: stream)
            }
        } catch { await failed(stream) }
    }

    private func receive(_ frame: [String: Any], from stream: Stream) async throws {
        guard owns(stream), let type = frame["type"] as? String else { return }
        if type == "hello" {
            guard !stream.hello else { throw URLError(.cannotParseResponse) }
            stream.hello = true; stream.watchdogTask?.cancel(); startHeartbeat(stream)
            for id in subscriptions.keys { try await sendSubscription(id, on: stream) }
            if stream === active {
                attempts = 0; await state(.connected, nil); await sendPendingCommands(on: stream)
            }
            await maybePromote(stream); return
        }
        guard stream.hello else { throw URLError(.cannotParseResponse) }
        switch type {
        case "heartbeat": armHeartbeatWatchdog(stream)
        case "migrating":
            if stream === active { if candidate == nil { open(replacement: true) } }
            else { await failed(stream) }
        case "subscribed":
            guard let id = frame["id"] as? String, subscriptions[id] != nil, var value = stream.subscriptions[id] else { return }
            value.subscribed = true; stream.subscriptions[id] = value; await maybePromote(stream)
        case "event":
            guard let id = frame["id"] as? String, let event = frame["event"] as? [String: Any] else { throw URLError(.cannotParseResponse) }
            try await receiveEvent(event, id: id, from: stream)
        case "result": await receiveResult(frame, from: stream)
        case "error":
            guard let id = frame["id"] as? String, let subscription = subscriptions[id] else { return }
            if stream === candidate { await failed(stream); return }
            subscriptions.removeValue(forKey: id)
            await subscription.handler(["type": "subscription.error", "status": frame["status"] as? Int ?? 0,
                                        "error": frame["error"] as? String ?? "Live updates failed."])
            await state(.connected, frame["error"] as? String)
        default: throw URLError(.cannotParseResponse)
        }
    }

    private func receiveEvent(_ event: [String: Any], id: String, from stream: Stream) async throws {
        guard var logical = subscriptions[id], var streamState = stream.subscriptions[id], let kind = logical.frame["kind"] as? String else { return }
        if kind == "chat" {
            if let eventType = event["type"] as? String, ["message.created", "message.reactions", "message.attachments"].contains(eventType) {
                guard let next = sequence(event["seq"]) else { throw URLError(.cannotParseResponse) }
                if let position = streamState.position, next > position &+ 1 { throw URLError(.cannotParseResponse) }
                streamState.position = max(streamState.position ?? 0, next)
                if next <= (logical.appliedPosition ?? 0) { stream.subscriptions[id] = streamState; await maybePromote(stream); return }
                logical.appliedPosition = next
            } else if event["type"] as? String == "ready" {
                guard let checkpoint = sequence(event["cursor"]), streamState.position == nil || streamState.position == checkpoint else { throw URLError(.cannotParseResponse) }
                streamState.position = checkpoint; streamState.chatReady = true
                logical.appliedPosition = max(logical.appliedPosition ?? 0, checkpoint)
            }
        } else if kind == "presence", stream === candidate {
            streamState.pendingPresence = event; stream.subscriptions[id] = streamState; await maybePromote(stream); return
        } else if kind == "media", event["type"] as? String == "snapshot" {
            guard let revision = event["revision"] as? Int, revision >= 0 else { throw URLError(.cannotParseResponse) }
            streamState.snapshotRevision = revision
            if revision <= (logical.revision ?? -1) { stream.subscriptions[id] = streamState; await maybePromote(stream); return }
            logical.revision = revision
        }
        subscriptions[id] = logical; stream.subscriptions[id] = streamState
        await logical.handler(event); await maybePromote(stream)
    }

    private func receiveResult(_ frame: [String: Any], from stream: Stream) async {
        guard owns(stream), let id = frame["id"] as? String, let status = frame["status"] as? Int, var pending = commands[id] else { return }
        let body = frame["body"] as? [String: Any] ?? [:]
        if (200..<300).contains(status) { finishCommand(id: id, value: body); return }
        let code = body["code"] as? String
        if pending.frame["method"] as? String != "typing",
           (status == 409 && code == "command_pending") || (status == 503 && code == "gateway_draining") {
            if code == "gateway_draining", candidate == nil { open(replacement: true) }
            guard pending.retryTask == nil else { return }
            let base = code == "command_pending" ? 100 : 250
            let delay = min(base * (1 << min(pending.retries, 3)), 1_000)
            pending.retries += 1
            guard ContinuousClock.now.advanced(by: .milliseconds(delay)) < pending.deadline else { commands[id] = pending; return }
            pending.retryTask = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(delay)); guard !Task.isCancelled else { return }
                await self?.retryCommand(id)
            }
            commands[id] = pending; return
        }
        finishCommand(id: id, error: GatewayFailure(status: status, message: body["error"] as? String ?? "Live command failed.", code: code))
    }

    private func retryCommand(_ id: String) async {
        guard var pending = commands[id] else { return }
        pending.retryTask = nil; commands[id] = pending
        guard candidate == nil, let active, active.hello, ContinuousClock.now < pending.deadline else { return }
        try? await send(pending.frame, on: active)
    }

    private func maybePromote(_ stream: Stream) async {
        guard stream === candidate, stream.hello else { return }
        for (id, logical) in subscriptions {
            guard let value = stream.subscriptions[id], value.subscribed else { return }
            switch logical.frame["kind"] as? String {
            case "chat":
                let required = max(logical.appliedPosition ?? 0, active?.subscriptions[id]?.position ?? 0)
                guard value.chatReady, (value.position ?? 0) >= required else { return }
            case "media": guard let revision = value.snapshotRevision, revision >= (logical.revision ?? 0) else { return }
            case "presence": guard value.pendingPresence != nil else { return }
            default: return
            }
        }
        let old = active; active = stream; candidate = nil; attempts = 0
        stream.catchupTask?.cancel()
        let presence = subscriptions.compactMap { id, logical -> (Handler, [String: Any])? in
            guard logical.frame["kind"] as? String == "presence", let event = stream.subscriptions[id]?.pendingPresence else { return nil }
            return (logical.handler, event)
        }
        for (handler, event) in presence { await handler(event) }
        await sendPendingCommands(on: stream); close(old)
    }

    private func failed(_ stream: Stream) async {
        guard owns(stream) else { return }
        let wasCandidate = stream === candidate
        close(stream)
        if wasCandidate { candidate = nil; scheduleReconnect(replacement: active != nil); return }
        active = nil
        generation &+= 1
        openingTask?.cancel(); openingTask = nil
        // A not-yet-ready candidate cannot safely become the sole stream, and opening
        // beside it would exceed the two-socket bound. Replace both after backoff.
        if let candidate { close(candidate); self.candidate = nil }
        if !stopped, !subscriptions.isEmpty || !commands.isEmpty {
            await state(.reconnecting, "Live updates disconnected. Reconnecting…")
            scheduleReconnect(replacement: false)
        }
    }

    private func scheduleReconnect(replacement: Bool) {
        guard reconnectTask == nil, !stopped, !subscriptions.isEmpty || !commands.isEmpty else { return }
        attempts += 1
        let delay = min(pow(2, Double(min(attempts, 5))) * 0.25, 5)
        reconnectTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(delay)); guard !Task.isCancelled, let self else { return }
            await self.reconnect(replacement: replacement)
        }
    }

    private func reconnect(replacement: Bool) {
        reconnectTask = nil
        open(replacement: replacement && active != nil)
    }

    private func sendSubscription(_ id: String, on stream: Stream) async throws {
        guard var logical = subscriptions[id] else { return }
        if logical.frame["kind"] as? String == "chat", let cursor = logical.appliedPosition { logical.frame["after"] = String(cursor) }
        stream.subscriptions[id] = StreamSubscription(position: sequence(logical.frame["after"]))
        try await send(logical.frame, on: stream)
    }

    private func sendPendingCommands(on stream: Stream) async {
        for (id, pending) in commands where ContinuousClock.now < pending.deadline {
            if pending.frame["method"] as? String == "typing" { finishCommand(id: id, error: GatewayFailure(status: 503, message: "Typing update discarded during reconnect.", code: nil)) }
            else { try? await send(pending.frame, on: stream) }
        }
    }

    private func send(_ value: [String: Any], on stream: Stream) async throws {
        guard owns(stream), !stream.closed else { throw URLError(.cancelled) }
        let data = try JSONSerialization.data(withJSONObject: value)
        try await stream.socket.send(.string(String(decoding: data, as: UTF8.self)))
    }

    private func startHeartbeat(_ stream: Stream) {
        armHeartbeatWatchdog(stream)
        stream.heartbeatTask = Task { [weak self, weak stream] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(10)); guard !Task.isCancelled, let self, let stream else { return }
                await self.heartbeat(stream)
            }
        }
    }

    private func heartbeat(_ stream: Stream) async {
        let elapsed = lastActivity.duration(to: .now).components
        let age = max(0, Int(elapsed.seconds * 1_000 + elapsed.attoseconds / 1_000_000_000_000_000))
        do { try await send(["type": "heartbeat", "activityAgeMs": age], on: stream) }
        catch { await failed(stream) }
    }

    private func armOpenWatchdog(_ stream: Stream) { armWatchdog(stream, after: openTimeout, requireHello: false) }
    private func armHeartbeatWatchdog(_ stream: Stream) { armWatchdog(stream, after: .seconds(30), requireHello: true) }
    private func armWatchdog(_ stream: Stream, after delay: Duration, requireHello: Bool) {
        stream.watchdogTask?.cancel()
        stream.watchdogTask = Task { [weak self, weak stream] in
            try? await Task.sleep(for: delay); guard !Task.isCancelled, let self, let stream else { return }
            await self.expire(stream, requireHello: requireHello)
        }
    }

    private func expire(_ stream: Stream, requireHello: Bool) async {
        guard owns(stream), requireHello == stream.hello else { return }
        await failed(stream)
    }

    private func finishCommand(id: String, value: [String: Any]? = nil, error: Error? = nil) {
        guard let pending = commands.removeValue(forKey: id) else { return }
        pending.timeoutTask.cancel(); pending.retryTask?.cancel()
        if let error { pending.continuation.resume(throwing: error) } else { pending.continuation.resume(returning: value ?? [:]) }
        if subscriptions.isEmpty && commands.isEmpty { stopSockets(publish: true) }
    }

    private func stopSockets(publish: Bool) {
        generation &+= 1; attempts = 0
        openingTask?.cancel(); reconnectTask?.cancel(); openingTask = nil; reconnectTask = nil
        close(active); close(candidate); active = nil; candidate = nil
        if publish { let current = generation; Task { [weak self] in await self?.publishDisconnected(generation: current) } }
    }

    private func close(_ stream: Stream?) {
        guard let stream, !stream.closed else { return }
        stream.closed = true; stream.receiveTask?.cancel(); stream.heartbeatTask?.cancel(); stream.watchdogTask?.cancel()
        stream.catchupTask?.cancel()
        stream.socket.cancel()
    }

    private func publishDisconnected(generation expected: UInt64) async {
        guard generation == expected, active == nil, candidate == nil else { return }
        await state(.disconnected, nil)
    }

    private func owns(_ stream: Stream) -> Bool { stream === active || stream === candidate }
    private func sequence(_ value: Any?) -> UInt64? {
        guard let text = value as? String, !text.isEmpty, text.allSatisfy({ $0.isNumber }) else { return nil }
        return UInt64(text)
    }
}
