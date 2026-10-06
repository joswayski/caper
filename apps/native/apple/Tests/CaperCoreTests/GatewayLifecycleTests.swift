import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import XCTest
@testable import CaperCore

private enum TestSocketError: Error { case closed }

private final class TestSocket: GatewaySocket, @unchecked Sendable {
    private let lock = NSLock()
    private var inbox: [Result<GatewaySocketMessage, Error>] = []
    private var waiter: CheckedContinuation<GatewaySocketMessage, Error>?
    private var sent: [[String: Any]] = []
    private var isCancelled = false
    var cancelled: Bool { lock.lock(); defer { lock.unlock() }; return isCancelled }

    func resume() {}
    func receive() async throws -> GatewaySocketMessage {
        try await withCheckedThrowingContinuation { continuation in
            lock.lock()
            if !inbox.isEmpty {
                let value = inbox.removeFirst(); lock.unlock(); continuation.resume(with: value)
            } else if isCancelled {
                lock.unlock(); continuation.resume(throwing: TestSocketError.closed)
            } else {
                waiter = continuation; lock.unlock()
            }
        }
    }
    func send(_ message: GatewaySocketMessage) async throws {
        guard case .string(let text) = message,
              let value = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] else { return }
        append(value)
    }
    func cancel() { finish(.failure(TestSocketError.closed), markCancelled: true) }
    func push(_ value: [String: Any]) {
        let data = try! JSONSerialization.data(withJSONObject: value)
        finish(.success(.string(String(decoding: data, as: UTF8.self))), markCancelled: false)
    }
    func fail() { finish(.failure(TestSocketError.closed), markCancelled: false) }
    func frames(type: String) -> [[String: Any]] {
        lock.lock(); defer { lock.unlock() }; return sent.filter { $0["type"] as? String == type }
    }
    private func append(_ value: [String: Any]) { lock.lock(); defer { lock.unlock() }; sent.append(value) }
    private func finish(_ result: Result<GatewaySocketMessage, Error>, markCancelled: Bool) {
        lock.lock()
        if markCancelled { isCancelled = true }
        if let continuation = waiter { waiter = nil; lock.unlock(); continuation.resume(with: result) }
        else { inbox.append(result); lock.unlock() }
    }
}

private final class SocketFactory: @unchecked Sendable {
    private let lock = NSLock()
    private(set) var sockets: [TestSocket] = []
    func make(_: URLRequest) -> any GatewaySocket {
        lock.lock(); defer { lock.unlock() }
        let socket = TestSocket(); sockets.append(socket); return socket
    }
    func socket(_ index: Int) -> TestSocket? {
        lock.lock(); defer { lock.unlock() }; return sockets.indices.contains(index) ? sockets[index] : nil
    }
    var count: Int { lock.lock(); defer { lock.unlock() }; return sockets.count }
}

private actor TokenRequests {
    private(set) var count = 0
    private var continuations: [CheckedContinuation<String?, Never>] = []
    func fetch() async -> String? {
        count += 1
        return await withCheckedContinuation { continuations.append($0) }
    }
    func finishNext() { if !continuations.isEmpty { continuations.removeFirst().resume(returning: nil) } }
    func finishAll() { continuations.forEach { $0.resume(returning: nil) }; continuations = [] }
}

@MainActor
final class GatewayLifecycleTests: XCTestCase {
    private func gateway(_ factory: SocketFactory, states: ((GatewayState) -> Void)? = nil) -> Gateway {
        Gateway(baseURL: URL(string: "https://caper.invalid")!, socketFactory: { factory.make($0) },
                token: { nil }, state: { state, _ in states?(state) })
    }

    private func eventually(_ message: String = "condition", timeout: Duration = .seconds(2),
                            _ condition: @escaping () -> Bool) async {
        let deadline = ContinuousClock.now.advanced(by: timeout)
        while !condition(), ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(condition(), message)
    }

    private func subscribeID(_ socket: TestSocket, kind: String) async -> String {
        await eventually("missing \(kind) subscription") { socket.frames(type: "subscribe").contains { $0["kind"] as? String == kind } }
        return socket.frames(type: "subscribe").first { $0["kind"] as? String == kind }!["id"] as! String
    }

    func testHandoffWaitsForChatReadyAndMediaWhileOldDeliversAndSuppressesDuplicates() async {
        let factory = SocketFactory(); var chatEvents: [String] = []; var mediaRevisions: [Int] = []
        var states: [GatewayState] = []
        let subject = gateway(factory, states: { states.append($0) })
        _ = await subject.subscribeChat(channelID: "chat", after: "0") { chatEvents.append($0["type"] as! String) }
        _ = await subject.subscribeMedia(channelID: "voice", token: "cap") { mediaRevisions.append($0["revision"] as! Int) }
        await eventually { factory.socket(0) != nil }; let old = factory.socket(0)!
        old.push(["type": "hello"])
        let chat = await subscribeID(old, kind: "chat"), media = await subscribeID(old, kind: "media")
        old.push(["type": "subscribed", "id": chat]); old.push(["type": "event", "id": chat, "event": ["type": "ready", "cursor": "0"]])
        old.push(["type": "subscribed", "id": media]); old.push(["type": "event", "id": media, "event": ["type": "snapshot", "revision": 1]])
        old.push(["type": "migrating"])
        await eventually { factory.socket(1) != nil }; let candidate = factory.socket(1)!; candidate.push(["type": "hello"])
        _ = await subscribeID(candidate, kind: "chat"); _ = await subscribeID(candidate, kind: "media")
        XCTAssertEqual(candidate.frames(type: "subscribe").first { $0["kind"] as? String == "media" }?["token"] as? String, "cap")
        candidate.push(["type": "subscribed", "id": chat]); candidate.push(["type": "subscribed", "id": media])
        await eventually { !old.cancelled }; XCTAssertFalse(old.cancelled, "subscribed at cursor zero must not promote before ready")
        old.push(["type": "event", "id": chat, "event": ["type": "message.created", "seq": "1"]])
        candidate.push(["type": "event", "id": chat, "event": ["type": "message.created", "seq": "1"]])
        await eventually { chatEvents.contains("message.created") }
        let reaction: [String: Any] = ["type": "message.reactions", "schemaVersion": 1, "channelId": "chat",
                                       "seq": "2", "messageId": "message-1", "reactions": [["emoji": "🎉", "authorIds": ["peer"]]]]
        old.push(["type": "event", "id": chat, "event": reaction])
        await eventually { chatEvents.contains("message.reactions") }
        let attachments: [String: Any] = ["type": "message.attachments", "schemaVersion": 1, "channelId": "chat",
                                          "seq": "3", "messageId": "message-1", "attachments": [[String: Any]]()]
        old.push(["type": "event", "id": chat, "event": attachments])
        await eventually { chatEvents.contains("message.attachments") }
        candidate.push(["type": "event", "id": chat, "event": reaction])
        candidate.push(["type": "event", "id": chat, "event": attachments])
        candidate.push(["type": "event", "id": chat, "event": ["type": "ready", "cursor": "3"]])
        XCTAssertEqual(chatEvents.filter { $0 == "message.created" }.count, 1)
        XCTAssertFalse(old.cancelled, "chat catch-up alone must not drop voice")
        candidate.push(["type": "event", "id": media, "event": ["type": "snapshot", "revision": 1]])
        await eventually("candidate did not promote") { old.cancelled }
        XCTAssertEqual(chatEvents.filter { $0 == "message.reactions" }.count, 1, "candidate reaction replay must be deduplicated")
        XCTAssertEqual(chatEvents.filter { $0 == "message.attachments" }.count, 1, "attachment updates are sequenced like reactions")
        XCTAssertEqual(mediaRevisions, [1], "stale candidate snapshot must not be redelivered")
        XCTAssertEqual(states, [.connecting, .connected], "planned handoff must not publish disconnection")
        await subject.stop()
    }

    func testChatCandidateNeedsReadyCheckpointEvenAtZero() async {
        let factory = SocketFactory(); var delivered = 0; let subject = gateway(factory)
        _ = await subject.subscribeChat(channelID: "chat", after: "0") { if $0["type"] as? String == "message.created" { delivered += 1 } }
        await eventually { factory.socket(0) != nil }; let old = factory.socket(0)!; old.push(["type": "hello"])
        let id = await subscribeID(old, kind: "chat")
        old.push(["type": "subscribed", "id": id]); old.push(["type": "event", "id": id, "event": ["type": "ready", "cursor": "0"]])
        old.push(["type": "migrating"])
        await eventually { factory.socket(1) != nil }; let candidate = factory.socket(1)!; candidate.push(["type": "hello"])
        _ = await subscribeID(candidate, kind: "chat"); candidate.push(["type": "subscribed", "id": id])
        try? await Task.sleep(for: .milliseconds(50))
        XCTAssertFalse(old.cancelled, "subscription acknowledgment is not a replay checkpoint")
        old.push(["type": "event", "id": id, "event": ["type": "message.created", "seq": "1"]])
        await eventually { delivered == 1 }
        candidate.push(["type": "event", "id": id, "event": ["type": "message.created", "seq": "1"]])
        candidate.push(["type": "event", "id": id, "event": ["type": "ready", "cursor": "1"]])
        await eventually { old.cancelled }; XCTAssertEqual(delivered, 1)
        await subject.stop()
    }

    func testCandidateThatReceivesHelloButNeverCatchesUpIsReplaced() async {
        let factory = SocketFactory(); var revisions: [Int] = []
        let subject = Gateway(baseURL: URL(string: "https://caper.invalid")!, socketFactory: { factory.make($0) },
                              catchupTimeout: .milliseconds(100), token: { nil }, state: { _, _ in })
        _ = await subject.subscribeMedia(channelID: "voice", token: "cap") { revisions.append($0["revision"] as! Int) }
        await eventually { factory.socket(0) != nil }; let old = factory.socket(0)!; old.push(["type": "hello"])
        let id = await subscribeID(old, kind: "media")
        old.push(["type": "subscribed", "id": id]); old.push(["type": "event", "id": id, "event": ["type": "snapshot", "revision": 0]])
        old.push(["type": "migrating"])
        await eventually { factory.socket(1) != nil }; let stalled = factory.socket(1)!; stalled.push(["type": "hello"])
        _ = await subscribeID(stalled, kind: "media"); stalled.push(["type": "subscribed", "id": id])
        await eventually { stalled.cancelled }
        old.push(["type": "event", "id": id, "event": ["type": "snapshot", "revision": 1]])
        await eventually { revisions == [0, 1] }; XCTAssertFalse(old.cancelled)
        await eventually { factory.socket(2) != nil }
        await subject.stop()
    }

    func testCandidateFailureLeavesOldStreamDelivering() async {
        let factory = SocketFactory(); var delivered = 0; let subject = gateway(factory)
        _ = await subject.subscribeChat(channelID: "chat", after: "0") { if $0["type"] as? String == "message.created" { delivered += 1 } }
        await eventually { factory.socket(0) != nil }; let old = factory.socket(0)!; old.push(["type": "hello"])
        let id = await subscribeID(old, kind: "chat"); old.push(["type": "subscribed", "id": id]); old.push(["type": "event", "id": id, "event": ["type": "ready", "cursor": "0"]]); old.push(["type": "migrating"])
        await eventually { factory.socket(1) != nil }; factory.socket(1)!.fail()
        old.push(["type": "event", "id": id, "event": ["type": "message.created", "seq": "1"]])
        await eventually { delivered == 1 }; XCTAssertFalse(old.cancelled)
        await subject.stop()
    }

    func testCandidateOpenTimeoutLeavesOldStreamAndRetriesReplacement() async {
        let factory = SocketFactory(); var delivered = 0
        // This timeout also covers the initial healthy socket. Leave enough
        // time for the fixture to send hello on a busy hosted runner.
        let subject = Gateway(baseURL: URL(string: "https://caper.invalid")!, socketFactory: { factory.make($0) },
                              openTimeout: .seconds(1), token: { nil }, state: { _, _ in })
        _ = await subject.subscribeChat(channelID: "chat", after: "0") { if $0["type"] as? String == "message.created" { delivered += 1 } }
        await eventually { factory.socket(0) != nil }; let old = factory.socket(0)!; old.push(["type": "hello"])
        let id = await subscribeID(old, kind: "chat"); old.push(["type": "subscribed", "id": id]); old.push(["type": "event", "id": id, "event": ["type": "ready", "cursor": "0"]]); old.push(["type": "migrating"])
        await eventually { factory.socket(1) != nil }; let timedOut = factory.socket(1)!
        await eventually("candidate watchdog did not close socket") { timedOut.cancelled }
        old.push(["type": "event", "id": id, "event": ["type": "message.created", "seq": "1"]])
        await eventually { delivered == 1 }; XCTAssertFalse(old.cancelled)
        await eventually("replacement was not retried", timeout: .seconds(1)) { factory.socket(2) != nil }
        await subject.stop()
    }

    func testStableCommandRetriesAcrossPendingAndDrainingHandoff() async throws {
        let factory = SocketFactory(); let subject = gateway(factory)
        _ = await subject.subscribeMedia(channelID: "voice", token: nil) { _ in }
        await eventually { factory.socket(0) != nil }; let old = factory.socket(0)!; old.push(["type": "hello"])
        let media = await subscribeID(old, kind: "media"); old.push(["type": "subscribed", "id": media]); old.push(["type": "event", "id": media, "event": ["type": "snapshot", "revision": 0]])
        let command = Task { try await subject.command(method: "mute", timeout: .seconds(3)) }
        await eventually { old.frames(type: "command").count == 1 }; let first = old.frames(type: "command")[0]; let id = first["id"] as! String
        old.push(["type": "result", "id": id, "status": 409, "body": ["code": "command_pending"]])
        await eventually { old.frames(type: "command").count == 2 }
        XCTAssertEqual(old.frames(type: "command")[1]["id"] as? String, id); XCTAssertEqual(old.frames(type: "command")[1]["issuedAt"] as? Int, first["issuedAt"] as? Int)
        old.push(["type": "result", "id": id, "status": 503, "body": ["code": "gateway_draining"]])
        await eventually { factory.socket(1) != nil }; let next = factory.socket(1)!; next.push(["type": "hello"])
        _ = await subscribeID(next, kind: "media"); next.push(["type": "subscribed", "id": media]); next.push(["type": "event", "id": media, "event": ["type": "snapshot", "revision": 0]])
        await eventually { next.frames(type: "command").count == 1 }; let retried = next.frames(type: "command")[0]
        XCTAssertEqual(retried["id"] as? String, id); XCTAssertEqual(retried["issuedAt"] as? Int, first["issuedAt"] as? Int)
        next.push(["type": "result", "id": id, "status": 200, "body": ["ok": true]])
        let result = try await command.value
        XCTAssertEqual(result["ok"] as? Bool, true); await subject.stop()
    }

    func testRepeatedHandoffs() async {
        let factory = SocketFactory(); let subject = gateway(factory)
        _ = await subject.subscribeMedia(channelID: "voice", token: nil) { _ in }
        await eventually { factory.socket(0) != nil }; var current = factory.socket(0)!; current.push(["type": "hello"])
        let id = await subscribeID(current, kind: "media"); current.push(["type": "subscribed", "id": id]); current.push(["type": "event", "id": id, "event": ["type": "snapshot", "revision": 0]])
        for index in 1...2 {
            current.push(["type": "migrating"]); await eventually { factory.socket(index) != nil }
            let next = factory.socket(index)!; next.push(["type": "hello"]); _ = await subscribeID(next, kind: "media")
            next.push(["type": "subscribed", "id": id]); next.push(["type": "event", "id": id, "event": ["type": "snapshot", "revision": index]])
            await eventually { current.cancelled }; current = next
        }
        XCTAssertFalse(current.cancelled); await subject.stop()
    }

    func testStopDuringPendingAuthorizationThenResubscribeDoesNotClearNewOpeningTask() async {
        let tokens = TokenRequests(); let factory = SocketFactory()
        let subject = Gateway(baseURL: URL(string: "https://caper.invalid")!, socketFactory: { factory.make($0) },
                              token: { await tokens.fetch() }, state: { _, _ in })
        _ = await subject.subscribeMedia(channelID: "one", token: nil) { _ in }
        while await tokens.count != 1 { try? await Task.sleep(for: .milliseconds(10)) }
        await subject.stop()
        _ = await subject.subscribeMedia(channelID: "two", token: nil) { _ in }
        while await tokens.count != 2 { try? await Task.sleep(for: .milliseconds(10)) }
        await tokens.finishNext(); try? await Task.sleep(for: .milliseconds(30))
        _ = await subject.subscribeMedia(channelID: "three", token: nil) { _ in }
        try? await Task.sleep(for: .milliseconds(30)); let tokenCount = await tokens.count
        XCTAssertEqual(tokenCount, 2)
        await tokens.finishAll(); await eventually { factory.socket(0) != nil }; XCTAssertEqual(factory.count, 1); await subject.stop()
    }

    func testActiveFailureDuringReplacementAuthorizationDoesNotBlockRecovery() async {
        let tokens = TokenRequests(); let factory = SocketFactory()
        let subject = Gateway(baseURL: URL(string: "https://caper.invalid")!, socketFactory: { factory.make($0) },
                              token: { factory.count == 0 ? nil : await tokens.fetch() }, state: { _, _ in })
        _ = await subject.subscribeMedia(channelID: "voice", token: "cap") { _ in }
        await eventually { factory.socket(0) != nil }; let old = factory.socket(0)!; old.push(["type": "hello"])
        let id = await subscribeID(old, kind: "media")
        old.push(["type": "subscribed", "id": id]); old.push(["type": "event", "id": id, "event": ["type": "snapshot", "revision": 0]])
        old.push(["type": "migrating"])
        let authorizationDeadline = ContinuousClock.now.advanced(by: .seconds(3))
        while await tokens.count != 1, ContinuousClock.now < authorizationDeadline { try? await Task.sleep(for: .milliseconds(10)) }
        old.fail()
        let recoveryDeadline = ContinuousClock.now.advanced(by: .seconds(3))
        while await tokens.count != 2, ContinuousClock.now < recoveryDeadline { try? await Task.sleep(for: .milliseconds(10)) }
        let requests = await tokens.count
        XCTAssertEqual(requests, 2, "cancelled replacement authorization must not block an active reconnect")
        await tokens.finishAll()
        await eventually { factory.socket(1) != nil }
        XCTAssertEqual(factory.count, 2, "the stale authorization must not open an extra socket")
        await subject.stop()
    }

    func testStopDuringPendingHandoffClosesBothStreams() async {
        let factory = SocketFactory(); let subject = gateway(factory)
        _ = await subject.subscribeMedia(channelID: "voice", token: nil) { _ in }
        await eventually { factory.socket(0) != nil }; let old = factory.socket(0)!; old.push(["type": "hello"])
        let id = await subscribeID(old, kind: "media"); old.push(["type": "subscribed", "id": id]); old.push(["type": "event", "id": id, "event": ["type": "snapshot", "revision": 0]]); old.push(["type": "migrating"])
        await eventually { factory.socket(1) != nil }; let candidate = factory.socket(1)!; await subject.stop()
        await eventually { old.cancelled && candidate.cancelled }; XCTAssertEqual(factory.count, 2)
    }
}
