import XCTest
@testable import CaperCore

private final class MemoryTokenStore: TokenStore, @unchecked Sendable {
    var token: String?
    init(_ token: String? = nil) { self.token = token }
    func load() throws -> String? { token }
    func save(_ token: String) throws { self.token = token }
    func clear() throws { token = nil }
}

private final class MockURLProtocol: URLProtocol, @unchecked Sendable {
    static var handler: ((URLRequest) throws -> (Int, Data))!
    static var deferred: ((MockURLProtocol, URLRequest) -> Bool)?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        if Self.deferred?(self, request) == true { return }
        do {
            let (status, data) = try Self.handler(request)
            respond(status: status, data: data)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}

    func respond(status: Int, data: Data = Data()) {
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["content-type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
}

final class APIClientTests: XCTestCase {
    override func tearDown() {
        MockURLProtocol.handler = nil
        MockURLProtocol.deferred = nil
        super.tearDown()
    }

    private func client(token: String = "account-secret") -> APIClient {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        return APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration), tokenStore: MemoryTokenStore(token))
    }

    @MainActor
    func testProfileEditPreservesConversationDraftAndRejectedSend() async throws {
        var historyRequests = 0
        var sessionRequests = 0
        MockURLProtocol.handler = { request in
            switch request.url?.path {
            case "/api/chat/session":
                sessionRequests += 1
                return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Old Name","isGuest":false}}"#.utf8))
            case "/api/chat/channels/Design123456/messages":
                if request.httpMethod == "POST" { return (400, Data(#"{"error":"Rejected message"}"#.utf8)) }
                historyRequests += 1
                return (200, Data(#"{"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"Design123456","name":"design"},"messages":[],"cursor":"0","hasMore":false}"#.utf8))
            case "/api/account/profile":
                return (200, Data(#"{"id":"self","username":"new_user","displayName":"New Name"}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let model = AppModel(api: client())
        model.account = Account(id: "self", username: "old_user", displayName: "Old Name")
        model.phase = .ready
        model.selectedSpaceID = "Space1234567"
        model.selectedChannelID = "Design123456"
        await model.chat.open(channelID: "Design123456", displayName: "Old Name")
        model.chat.draft = "rejected first message"
        await model.chat.send()
        let pending = try XCTUnwrap(model.chat.pendingMessage)
        model.chat.draft = "successor draft"
        await model.saveProfile(username: "new_user", displayName: "New Name")
        XCTAssertNil(model.error)
        XCTAssertEqual(model.selectedChannelID, "Design123456")
        XCTAssertEqual(model.chat.draft, "successor draft")
        XCTAssertEqual(model.chat.pendingMessage?.id, pending.id)
        XCTAssertTrue(model.chat.sendRejected)
        XCTAssertEqual(model.chat.currentAuthor?.name, "New Name")
        XCTAssertEqual(historyRequests, 1)
        XCTAssertEqual(sessionRequests, 1)
        await model.chat.stop()
    }

    @MainActor
    func testSendClearsOnlySubmittedDraftAcrossHTTPGatewayAndLateError() async throws {
        let channel = "Aaaaaaaaaaaa"
        MockURLProtocol.handler = { request in
            switch request.url?.path {
            case "/api/chat/session":
                return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case "/api/chat/channels/\(channel)/messages":
                return (200, Data("""
                {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channel)","name":"general"},"messages":[],"cursor":"0","hasMore":false}
                """.utf8))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channel, displayName: "Me")
        let requested = expectation(description: "first message request")
        var held: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.httpMethod == "POST", urlRequest.url?.path == "/api/chat/channels/\(channel)/messages" else { return false }
            held = request; requested.fulfill(); return true
        }
        chat.draft = "submitted first"
        let send = Task { await chat.send() }
        await fulfillment(of: [requested], timeout: 2)
        let command = try XCTUnwrap(chat.pendingMessage)
        XCTAssertEqual(command.text, "submitted first")
        XCTAssertEqual(chat.draft, "")
        chat.draft = "new next draft"
        let payload: [String: Any] = [
            "id": "message-one", "channelId": channel, "seq": "1", "clientMessageId": command.id,
            "author": ["id": "self", "name": "Me", "isGuest": false],
            "content": ["version": 1, "type": "text", "text": command.text], "createdAt": "2026-01-01T00:00:00Z",
        ]
        chat.receive(["type": "message.created", "seq": "1", "message": payload], generation: 1, channelID: channel)
        XCTAssertNil(chat.pendingMessage)
        XCTAssertEqual(chat.draft, "new next draft", "gateway confirmation must not erase the next draft")
        held?.respond(status: 500, data: Data(#"{"error":"late failure"}"#.utf8))
        await send.value
        // The independent WebSocket may report a reconnect while HTTP finishes.
        // Only this already-confirmed send's late failure must be ignored.
        XCTAssertNotEqual(chat.error?.contains("late failure"), true)
        XCTAssertFalse(chat.sendRejected)
        XCTAssertNil(chat.pendingMessage)
        XCTAssertEqual(chat.draft, "new next draft", "late HTTP failure must not erase confirmed successor")
        XCTAssertEqual(chat.messages.map(\.content.text), ["submitted first"])
        await chat.stop()
    }

    @MainActor
    func testHTTPConfirmationCannotClearNextDraftBeforeGatewayReplay() async throws {
        let channel = "Aaaaaaaaaaaa"
        MockURLProtocol.handler = { request in
            switch request.url?.path {
            case "/api/chat/session": return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case "/api/chat/channels/\(channel)/messages":
                return (200, Data("""
                {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channel)","name":"general"},"messages":[],"cursor":"0","hasMore":false}
                """.utf8))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channel, displayName: "Me")
        let requested = expectation(description: "held HTTP confirmation")
        var held: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.httpMethod == "POST" else { return false }
            held = request; requested.fulfill(); return true
        }
        chat.draft = "submitted first"
        let send = Task { await chat.send() }
        await fulfillment(of: [requested], timeout: 2)
        let command = try XCTUnwrap(chat.pendingMessage)
        chat.draft = "different new draft"
        let response = Data("""
        {"id":"m1","channelId":"\(channel)","seq":"1","clientMessageId":"\(command.id)","author":{"id":"self","name":"Me","isGuest":false},"content":{"version":1,"type":"text","text":"submitted first"},"createdAt":"2026-01-01T00:00:00Z"}
        """.utf8)
        held?.respond(status: 200, data: response)
        await send.value
        XCTAssertNil(chat.pendingMessage)
        XCTAssertEqual(chat.draft, "different new draft")
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: response) as? [String: Any])
        chat.receive(["type": "message.created", "seq": "1", "message": payload], generation: 1, channelID: channel)
        XCTAssertEqual(chat.draft, "different new draft")
        XCTAssertEqual(chat.messages.count, 1)
        await chat.stop()
    }

    @MainActor
    func testUncertainRetryPreservesEvenIdenticalNextDraft() async throws {
        let channel = "Aaaaaaaaaaaa"
        MockURLProtocol.handler = { request in
            switch request.url?.path {
            case "/api/chat/session": return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case "/api/chat/channels/\(channel)/messages" where request.httpMethod == "GET":
                return (200, Data("""
                {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channel)","name":"general"},"messages":[],"cursor":"0","hasMore":false}
                """.utf8))
            case "/api/chat/channels/\(channel)/messages": return (500, Data(#"{"error":"unknown outcome"}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channel, displayName: "Me")
        chat.draft = "same text"
        await chat.send()
        let pending = try XCTUnwrap(chat.pendingMessage)
        XCTAssertFalse(chat.sendRejected)
        XCTAssertEqual(chat.draft, "")
        chat.draft = "same text"
        await chat.send()
        XCTAssertEqual(chat.pendingMessage, pending)
        XCTAssertEqual(chat.draft, "same text", "retry owns the pending command, not a newly typed identical draft")
        await chat.stop()
    }

    @MainActor
    func testRejectedSendRemainsEditableOnlyWithEmptyNextDraft() async throws {
        let channel = "Aaaaaaaaaaaa"
        MockURLProtocol.handler = { request in
            switch request.url?.path {
            case "/api/chat/session": return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case "/api/chat/channels/\(channel)/messages" where request.httpMethod == "GET":
                return (200, Data("""
                {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channel)","name":"general"},"messages":[],"cursor":"0","hasMore":false}
                """.utf8))
            case "/api/chat/channels/\(channel)/messages": return (422, Data(#"{"error":"Invalid content"}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channel, displayName: "Me")
        chat.draft = "rejected old text"
        await chat.send()
        let rejected = try XCTUnwrap(chat.pendingMessage)
        XCTAssertTrue(chat.sendRejected)
        XCTAssertEqual(chat.draft, "")
        chat.draft = "unrelated next text"
        XCTAssertFalse(chat.discardRejected(edit: true))
        XCTAssertEqual(chat.pendingMessage, rejected)
        XCTAssertEqual(chat.draft, "unrelated next text")
        XCTAssertTrue(chat.discardRejected())
        XCTAssertNil(chat.pendingMessage)
        XCTAssertEqual(chat.draft, "unrelated next text")
        chat.draft = "corrected text"
        await chat.send()
        XCTAssertNotEqual(chat.pendingMessage?.id, rejected.id, "corrected send must have a new client message ID")
        XCTAssertTrue(chat.sendRejected)
        XCTAssertTrue(chat.discardRejected(edit: true))
        XCTAssertEqual(chat.draft, "corrected text")
        await chat.stop()
    }

    func testVoiceErrorsPreserveMachineCodeAndDistinguishProviderRetryFromRevocation() async throws {
        for (status, code, retry, revoked) in [
            (403, "ice_restart_retry", true, false),
            (403, "forbidden", false, true),
            (404, "track_gone", false, false),
            (404, "channel_missing", false, true),
            (502, "ice_restart_invalid", false, false),
            (503, "unavailable", true, false),
            (409, "ice_restart_pending", true, false),
            (409, "conflict", false, false),
        ] {
            MockURLProtocol.handler = { request in
                XCTAssertEqual(request.url?.path, "/api/channels/Abcdef123456/media/restart-ice")
                XCTAssertEqual(request.value(forHTTPHeaderField: "authorization"), "Bearer account-secret")
                XCTAssertEqual(request.value(forHTTPHeaderField: "x-caper-media-token"), "media-secret")
                XCTAssertFalse(request.url!.absoluteString.contains("secret"))
                return (status, Data("{\"error\":\"controlled failure\",\"code\":\"\(code)\"}".utf8))
            }
            do {
                try await client().media(channelID: "Abcdef123456", operation: "restart-ice", token: "media-secret", body: [String: String]())
                XCTFail("Expected a structured media error")
            } catch let failure as APIError {
                XCTAssertEqual(failure.code, code)
                XCTAssertEqual(failure.retryableVoiceControl, retry)
                XCTAssertEqual(failure.endsVoiceAccess, revoked)
            }
        }
    }

    @MainActor
    func testLateVoiceJoinIsLeftWithoutPublishingOrReplacingNewCall() async throws {
        let voice = VoiceClient(api: client(), requestMicrophonePermission: { true })
        let old = VoiceContext(channelID: "Aaaaaaaaaaaa", channelName: "old", spaceID: "Space1234567", spaceName: "Space")
        let replacement = VoiceContext(channelID: "Bbbbbbbbbbbb", channelName: "new", spaceID: "Space1234567", spaceName: "Space")
        let oldStarted = expectation(description: "old join waiting for response")
        let newStarted = expectation(description: "new join waiting for response")
        var oldRequest: MockURLProtocol?
        var newRequest: MockURLProtocol?
        var leftTokens: [String] = []
        MockURLProtocol.deferred = { request, urlRequest in
            if urlRequest.url?.path == "/api/channels/\(old.channelID)/media/join" {
                oldRequest = request; oldStarted.fulfill(); return true
            }
            if urlRequest.url?.path == "/api/channels/\(replacement.channelID)/media/join" {
                newRequest = request; newStarted.fulfill(); return true
            }
            return false
        }
        MockURLProtocol.handler = { request in
            // Any publish/state request is a failure: neither delayed join
            // may acquire or enable local media after its explicit stop.
            XCTAssertTrue(request.url!.path.hasSuffix("/media/leave"))
            let expected = request.url!.path.contains(old.channelID) ? "old-media" : "new-media"
            XCTAssertEqual(request.value(forHTTPHeaderField: "x-caper-media-token"), expected)
            leftTokens.append(expected)
            return (204, Data())
        }
        let oldJoin = Task { await voice.join(channelID: old.channelID, context: old, name: "Old") }
        await fulfillment(of: [oldStarted], timeout: 2)
        voice.leaveImmediately()
        XCTAssertEqual(voice.phase, .idle)
        let newJoin = Task { await voice.join(channelID: replacement.channelID, context: replacement, name: "New") }
        await fulfillment(of: [newStarted], timeout: 2)
        oldRequest?.respond(status: 200, data: Data(#"{"token":"old-media","id":"old","iceServers":[]}"#.utf8))
        await oldJoin.value
        XCTAssertEqual(voice.phase, .joining)
        XCTAssertEqual(voice.context, replacement)
        XCTAssertNil(voice.error)
        voice.leaveImmediately()
        newRequest?.respond(status: 200, data: Data(#"{"token":"new-media","id":"new","iceServers":[]}"#.utf8))
        await newJoin.value
        XCTAssertEqual(voice.phase, .idle)
        XCTAssertNil(voice.context)
        XCTAssertEqual(leftTokens, ["old-media", "new-media"], "Both late capabilities must be explicitly cleaned up")
    }

    func testAccountAndChatCapabilitiesUseSeparateHeadersAndNeverURLs() async throws {
        var requests: [URLRequest] = []
        MockURLProtocol.handler = { request in
            requests.append(request)
            if request.url!.path == "/api/chat/session" {
                return (200, Data(#"{"token":"chat-secret","author":{"id":"me","name":"Me","isGuest":false}}"#.utf8))
            }
            return (200, Data(#"{"id":"m1","channelId":"Abcdef123456","seq":"1","author":{"id":"me","name":"Me","isGuest":false},"content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-01-01T00:00:00Z","clientMessageId":"client-id"}"#.utf8))
        }
        let api = client()
        let session = try await api.chatSession(name: "Me")
        _ = try await api.send(channelID: "Abcdef123456", sessionToken: session.token, clientMessageID: "client-id", text: "hi")
        XCTAssertEqual(requests.count, 2)
        XCTAssertTrue(requests.allSatisfy { $0.value(forHTTPHeaderField: "authorization") == "Bearer account-secret" })
        XCTAssertNil(requests[0].value(forHTTPHeaderField: "x-caper-chat-token"))
        XCTAssertEqual(requests[1].value(forHTTPHeaderField: "x-caper-chat-token"), "chat-secret")
        XCTAssertTrue(requests.allSatisfy { !($0.url?.absoluteString.contains("secret") ?? true) })
    }

    func testHistoryRejectsWrongChannelAndFutureContent() async {
        MockURLProtocol.handler = { _ in
            (200, Data(#"{"space":{"id":"Space1234567","name":"S"},"channel":{"id":"Other1234567","name":"general"},"messages":[{"id":"m","channelId":"Other1234567","seq":"1","author":{"id":"u","name":"U","isGuest":false},"content":{"version":2,"type":"rich","text":"unsafe"},"createdAt":"now","clientMessageId":"c"}],"cursor":"1","hasMore":false}"#.utf8))
        }
        do {
            _ = try await client().history(channelID: "Abcdef123456")
            XCTFail("wrong-channel/future content must not render")
        } catch let error as APIError { XCTAssertEqual(error.status, 502) }
        catch { XCTFail("unexpected error: \(error)") }
    }

    func testLogoutFencesLateVerificationAndClearsVaultBeforeRemoteFailure() async throws {
        let store = MemoryTokenStore("old-token")
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        let api = APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration), tokenStore: store)
        let verifyStarted = expectation(description: "verification started")
        var delayedVerification: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path == "/api/auth/email/verify" else { return false }
            delayedVerification = request
            verifyStarted.fulfill()
            return true
        }
        MockURLProtocol.handler = { request in
            XCTAssertEqual(request.url?.path, "/api/auth/logout")
            XCTAssertEqual(request.value(forHTTPHeaderField: "authorization"), "Bearer old-token")
            return (503, Data(#"{"error":"unavailable"}"#.utf8))
        }

        let verification = Task { try await api.verify(challengeId: "challenge", code: "123456") }
        await fulfillment(of: [verifyStarted], timeout: 1)
        do { try await api.logout(); XCTFail("remote revoke should report failure") }
        catch let error as APIError { XCTAssertEqual(error.status, 503) }
        XCTAssertNil(store.token, "logout clears the vault before awaiting remote revoke")
        let tokenAfterLogout = await api.authorizationToken()
        XCTAssertNil(tokenAfterLogout)

        delayedVerification?.respond(status: 200, data: Data(#"{"account":{"id":"u","username":"user","displayName":"User"},"token":"late-token"}"#.utf8))
        do { _ = try await verification.value; XCTFail("late verification must be fenced") }
        catch is CancellationError {}
        catch { XCTFail("unexpected error: \(error)") }
        XCTAssertNil(store.token)
        let tokenAfterVerification = await api.authorizationToken()
        XCTAssertNil(tokenAfterVerification)
    }

    @MainActor
    func testAppLogoutClearsVisibleAccountSelectionBeforeRemoteRevokeCompletes() async {
        let store = MemoryTokenStore("old-token")
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        let api = APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration), tokenStore: store)
        let model = AppModel(api: api)
        model.account = Account(id: "user", username: "user", displayName: "User")
        model.phase = .ready
        model.selectedSpaceID = "Space1234567"
        model.selectedChannelID = "Chan12345678"
        model.challengeID = "challenge"
        let revokeStarted = expectation(description: "remote revoke started")
        var delayedRevoke: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path == "/api/auth/logout" else { return false }
            delayedRevoke = request
            revokeStarted.fulfill()
            return true
        }
        MockURLProtocol.handler = { _ in (500, Data()) }

        let logout = Task { await model.logout() }
        await fulfillment(of: [revokeStarted], timeout: 1)
        XCTAssertEqual(model.phase, .signedOut)
        XCTAssertNil(model.account)
        XCTAssertNil(model.selectedSpaceID)
        XCTAssertNil(model.selectedChannelID)
        XCTAssertNil(model.challengeID)
        XCTAssertNil(store.token)
        delayedRevoke?.respond(status: 204)
        await logout.value
    }

    @MainActor
    func testLateChannelOpenCannotReplaceNewerChannelAndStopClearsImmediately() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        let api = APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration), tokenStore: MemoryTokenStore())
        let chat = ChatModel(api: api)
        let oldHistoryStarted = expectation(description: "old history started")
        var delayedOldHistory: MockURLProtocol?
        let oldChannel = "Aaaaaaaaaaaa"
        let newChannel = "Bbbbbbbbbbbb"
        func history(channel: String, text: String) -> Data {
            Data("""
            {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channel)","name":"\(text)"},"messages":[{"id":"\(text)","channelId":"\(channel)","seq":"1","author":{"id":"u","name":"User","isGuest":false},"content":{"version":1,"type":"text","text":"\(text)"},"createdAt":"now","clientMessageId":"c-\(text)"}],"cursor":"1","hasMore":false}
            """.utf8)
        }
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path == "/api/chat/channels/\(oldChannel)/messages" else { return false }
            delayedOldHistory = request
            oldHistoryStarted.fulfill()
            return true
        }
        MockURLProtocol.handler = { request in
            switch request.url?.path {
            case "/api/chat/session":
                return (200, Data(#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#.utf8))
            case "/api/chat/channels/\(newChannel)/messages": return (200, history(channel: newChannel, text: "new"))
            case "/api/chat/events": return (503, Data())
            default: throw URLError(.badURL)
            }
        }

        let oldOpen = Task { await chat.open(channelID: oldChannel, displayName: "User") }
        await fulfillment(of: [oldHistoryStarted], timeout: 1)
        await chat.open(channelID: newChannel, displayName: "User")
        XCTAssertEqual(chat.channelName, "new")
        XCTAssertEqual(chat.messages.map(\.content.text), ["new"])

        delayedOldHistory?.respond(status: 200, data: history(channel: oldChannel, text: "old"))
        await oldOpen.value
        XCTAssertEqual(chat.channelName, "new")
        XCTAssertEqual(chat.messages.map(\.content.text), ["new"])

        await chat.stop()
        XCTAssertTrue(chat.messages.isEmpty)
        XCTAssertTrue(chat.draft.isEmpty)
        XCTAssertEqual(chat.liveState, .disconnected)
    }

    @MainActor
    func testOlderHistorySerializesRetriesAndRevocationClearsPrivateData() async throws {
        let chat = ChatModel(api: client())
        let channel = "Aaaaaaaaaaaa"
        func history(_ sequence: Int, more: Bool) -> Data {
            Data("""
            {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channel)","name":"general"},"messages":[{"id":"m\(sequence)","channelId":"\(channel)","seq":"\(sequence)","author":{"id":"u","name":"User","isGuest":false},"content":{"version":1,"type":"text","text":"Message \(sequence)"},"createdAt":"now","clientMessageId":"c\(sequence)"}],"cursor":"\(sequence)","hasMore":\(more)}
            """.utf8)
        }
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/chat/session" {
                return (200, Data(#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#.utf8))
            }
            if request.url?.path.hasSuffix("/messages") == true { return (200, history(20, more: true)) }
            throw URLError(.badURL)
        }
        await chat.open(channelID: channel, displayName: "User")
        chat.draft = "Keep my draft"
        let started = expectation(description: "one older request")
        started.assertForOverFulfill = true
        var delayed: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.query?.contains("before=20") == true else { return false }
            delayed = request; started.fulfill(); return true
        }
        let older = Task { await chat.loadOlder() }
        await fulfillment(of: [started], timeout: 2)
        XCTAssertTrue(chat.loadingOlder)
        await chat.loadOlder()
        XCTAssertTrue(chat.loadingOlder, "duplicate call cannot release the original request's gate")
        delayed?.respond(status: 503, data: Data(#"{"error":"History unavailable"}"#.utf8))
        await older.value
        XCTAssertFalse(chat.loadingOlder)
        XCTAssertNotNil(chat.olderError)
        XCTAssertEqual(chat.messages.map(\.seq), ["20"])
        XCTAssertEqual(chat.draft, "Keep my draft")

        MockURLProtocol.deferred = nil
        MockURLProtocol.handler = { _ in (200, history(7, more: true)) }
        await chat.loadOlder()
        XCTAssertNil(chat.olderError)
        XCTAssertEqual(chat.messages.map(\.seq), ["7", "20"])
        XCTAssertEqual(chat.messages.last?.id, "m20", "prepending must not change the bottom-scroll identity")

        var revoked: String?
        chat.onAccessRevoked = { revoked = $0 }
        MockURLProtocol.handler = { _ in (403, Data(#"{"error":"Access ended"}"#.utf8)) }
        await chat.loadOlder()
        XCTAssertEqual(revoked, channel)
        XCTAssertTrue(chat.messages.isEmpty)
        XCTAssertTrue(chat.draft.isEmpty)
        XCTAssertNil(chat.currentAuthor)
        XCTAssertFalse(chat.loadingOlder)
        XCTAssertNotNil(chat.error)
    }

    @MainActor
    func testLateOwnerMutationCannotRepopulateWorkspaceAfterLogout() async {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        let api = APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration), tokenStore: MemoryTokenStore("account-token"))
        let model = AppModel(api: api)
        model.account = Account(id: "owner", username: "owner", displayName: "Owner")
        model.phase = .ready
        let createStarted = expectation(description: "create started")
        var delayedCreate: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path == "/api/spaces", urlRequest.httpMethod == "POST" else { return false }
            delayedCreate = request
            createStarted.fulfill()
            return true
        }
        MockURLProtocol.handler = { request in
            switch (request.httpMethod, request.url?.path) {
            case ("POST", "/api/auth/logout"): return (204, Data())
            case ("GET", "/api/chat/general"):
                return (200, Data(#"{"space":{"id":"demo00000001","name":"Caper"},"channel":{"id":"demo00000002","name":"general"},"messages":[],"cursor":"0","hasMore":false}"#.utf8))
            case ("POST", "/api/chat/session"):
                return (200, Data(#"{"token":"guest-chat","author":{"id":"guest0000001","name":"Guest","isGuest":true}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }

        let mutation = Task { try await model.createSpace(name: "Late Space") }
        await fulfillment(of: [createStarted], timeout: 1)
        await model.logout()
        delayedCreate?.respond(status: 200, data: Data(#"{"id":"space0000009","name":"Late Space","ownerId":"owner"}"#.utf8))
        do { try await mutation.value; XCTFail("superseded mutation must be cancelled") }
        catch is CancellationError {}
        catch { XCTFail("unexpected error: \(error)") }
        XCTAssertNil(model.account)
        XCTAssertFalse(model.spaces.contains { $0.id == "space0000009" })
    }

    @MainActor
    func testBrowsingAnotherSpacePreservesActiveVoiceContext() async {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        let api = APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration), tokenStore: MemoryTokenStore("account-token"))
        let model = AppModel(api: api)
        model.voice.phase = .connected
        model.voice.context = VoiceContext(channelID: "chan00000001", channelName: "general", spaceID: "space0000001", spaceName: "First")
        model.selectedSpaceID = "space0000001"
        model.selectedChannelID = "chan00000001"
        let detailStarted = expectation(description: "new space detail started")
        var delayedDetail: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path == "/api/spaces/space0000002" else { return false }
            delayedDetail = request
            detailStarted.fulfill()
            return true
        }
        MockURLProtocol.handler = { _ in throw URLError(.badURL) }

        let selection = Task {
            await model.select(space: Space(id: "space0000002", name: "Next", ownerId: "owner0000001", demo: nil))
        }
        await fulfillment(of: [detailStarted], timeout: 1)
        XCTAssertEqual(model.voice.phase, .connected)
        XCTAssertEqual(model.voice.context, VoiceContext(channelID: "chan00000001", channelName: "general", spaceID: "space0000001", spaceName: "First"))
        XCTAssertEqual(model.selectedSpaceID, "space0000001", "keep the current selection while the target loads")
        XCTAssertEqual(model.openingSpaceID, "space0000002")
        delayedDetail?.respond(status: 200, data: Data(#"{"space":{"id":"space0000002","name":"Next","ownerId":"owner0000001"},"channels":[],"members":[]}"#.utf8))
        await selection.value
        XCTAssertEqual(model.selectedSpaceID, "space0000002")
        XCTAssertNil(model.openingSpaceID)
        XCTAssertEqual(model.voice.phase, .connected)
        XCTAssertEqual(model.voice.context?.channelID, "chan00000001")
    }

    @MainActor
    func testFailedNavigationKeepsDraftAndRetryIgnoresSupersededTarget() async {
        let model = AppModel(api: client())
        model.selectedSpaceID = "space0000001"
        model.selectedChannelID = "chan00000001"
        model.chat.draft = "Unsent draft"
        let target = Space(id: "space0000002", name: "Next", ownerId: "owner0000001", demo: nil)
        MockURLProtocol.handler = { _ in (503, Data(#"{"error":"Space temporarily unavailable"}"#.utf8)) }
        await model.select(space: target)
        XCTAssertEqual(model.selectedSpaceID, "space0000001")
        XCTAssertEqual(model.selectedChannelID, "chan00000001")
        XCTAssertEqual(model.chat.draft, "Unsent draft")
        XCTAssertNotNil(model.navigationError)
        XCTAssertNil(model.openingSpaceID)

        let retryStarted = expectation(description: "retry requested the failed target")
        var delayedRetry: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path == "/api/spaces/space0000002" else { return false }
            delayedRetry = request; retryStarted.fulfill(); return true
        }
        let retry = Task { await model.retryNavigation() }
        await fulfillment(of: [retryStarted], timeout: 2)
        MockURLProtocol.handler = { _ in (200, Data(#"{"space":{"id":"space0000003","name":"Latest","ownerId":"owner0000001"},"channels":[],"members":[]}"#.utf8)) }
        await model.select(space: Space(id: "space0000003", name: "Latest", ownerId: "owner0000001", demo: nil))
        delayedRetry?.respond(status: 503, data: Data(#"{"error":"Stale failure"}"#.utf8))
        await retry.value
        XCTAssertEqual(model.selectedSpaceID, "space0000003")
        XCTAssertNil(model.navigationError)
        XCTAssertNil(model.openingSpaceID)
    }

    @MainActor
    func testSpaceDenialWhenOpeningAnotherChannelClearsPreviouslyVisibleSpace() async {
        let model = AppModel(api: client())
        let space = Space(id: "space0000001", name: "Fixture", ownerId: "owner0000001", demo: nil)
        let current = Channel(id: "chan00000001", spaceId: space.id, name: "general", private: false)
        let target = Channel(id: "chan00000002", spaceId: space.id, name: "private", private: true)
        model.detail = SpaceDetail(space: space, channels: [current, target], members: [])
        model.selectedSpaceID = space.id; model.selectedChannelID = current.id
        model.chat.draft = "Private draft"
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/spaces/space0000001" {
                return (200, Data(#"{"space":{"id":"space0000001","name":"Fixture","ownerId":"owner0000001"},"channels":[],"members":[]}"#.utf8))
            }
            throw URLError(.badURL)
        }
        await model.select(channel: target)
        XCTAssertEqual(model.selectedChannelID, current.id, "denial of another private channel must preserve the accessible conversation")
        XCTAssertEqual(model.chat.draft, "Private draft")
        MockURLProtocol.handler = { _ in (403, Data(#"{"error":"Space membership ended"}"#.utf8)) }
        await model.select(channel: target)
        XCTAssertNil(model.selectedSpaceID)
        XCTAssertNil(model.selectedChannelID)
        XCTAssertNil(model.detail)
        XCTAssertTrue(model.chat.draft.isEmpty)
        XCTAssertTrue(model.chat.messages.isEmpty)
    }

    @MainActor
    func testOnlyDeletingOrRevokingActiveVoiceContextStopsCall() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        let api = APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration), tokenStore: MemoryTokenStore("account-token"))
        let model = AppModel(api: api)
        let active = Channel(id: "chan00000001", spaceId: "space0000001", name: "general", private: false)
        let other = Channel(id: "chan00000002", spaceId: "space0000001", name: "design", private: false)
        let space = Space(id: "space0000001", name: "Fixture", ownerId: "owner0000001", demo: nil)
        model.detail = SpaceDetail(space: space, channels: [active, other], members: [])
        model.selectedSpaceID = space.id
        model.selectedChannelID = active.id
        model.voice.phase = .connected
        model.voice.context = VoiceContext(channelID: active.id, channelName: active.name, spaceID: space.id, spaceName: space.name)
        MockURLProtocol.handler = { request in
            guard request.httpMethod == "DELETE" else { throw URLError(.badURL) }
            return (204, Data())
        }

        try await model.deleteChannel(other)
        XCTAssertEqual(model.voice.phase, .connected, "deleting a browsed non-active channel must preserve the active call")
        model.chat.onAccessRevoked?(other.id)
        XCTAssertEqual(model.voice.phase, .connected, "revocation from another channel must not tear down voice")

        model.chat.onAccessRevoked?(active.id)
        XCTAssertEqual(model.voice.phase, .idle, "revoking the active voice channel must tear down synchronously")
        XCTAssertNil(model.voice.context)
    }

    @MainActor
    func testNavigationPrefetchIsReadOnlyAndBounded() async throws {
        let model = AppModel(api: client())
        var paths: [String] = []
        var channelSpaces: [String: String] = [:]
        let retainedPrefetchesCompleted = expectation(description: "all retained prefetches completed")
        retainedPrefetchesCompleted.expectedFulfillmentCount = 4
        MockURLProtocol.handler = { request in
            let path = request.url!.path
            paths.append(path)
            if path.hasPrefix("/api/spaces/") {
                let id = String(path.split(separator: "/").last!)
                let channel = "chan0000000\(id.last!)"
                channelSpaces[channel] = id
                return (200, Data("{\"space\":{\"id\":\"\(id)\",\"name\":\"Space\",\"ownerId\":\"owner0000001\"},\"channels\":[{\"id\":\"\(channel)\",\"spaceId\":\"\(id)\",\"name\":\"general\",\"private\":false}],\"members\":[]}".utf8))
            }
            let channel = String(path.split(separator: "/")[3])
            let space = try XCTUnwrap(channelSpaces[channel])
            if channel != "chan00000000" { retainedPrefetchesCompleted.fulfill() }
            return (200, Data("{\"space\":{\"id\":\"\(space)\",\"name\":\"Space\"},\"channel\":{\"id\":\"\(channel)\",\"name\":\"general\"},\"messages\":[],\"cursor\":\"7\",\"hasMore\":false}".utf8))
        }
        for index in 0..<5 {
            let id = "space000000\(index)"
            model.prefetch(space: Space(id: id, name: "S", ownerId: "owner0000001", demo: nil))
        }
        await fulfillment(of: [retainedPrefetchesCompleted], timeout: 2)
        XCTAssertEqual(model.navigationCacheCounts.prefetches, 4)
        XCTAssertFalse(paths.contains("/api/chat/session"), "speculation must not create capabilities")
        XCTAssertTrue(paths.allSatisfy { $0.hasPrefix("/api/spaces/") || $0.contains("/messages") })
    }

    @MainActor
    func testNavigationRevocationDiscardsWarmEntry() async throws {
        let model = AppModel(api: client())
        let space = Space(id: "space0000001", name: "S", ownerId: "owner0000001", demo: nil)
        let prefetchCompleted = expectation(description: "prefetch completed")
        MockURLProtocol.handler = { request in
            if request.url!.path == "/api/spaces/space0000001" {
                return (200, Data(#"{"space":{"id":"space0000001","name":"S","ownerId":"owner0000001"},"channels":[{"id":"chan00000001","spaceId":"space0000001","name":"general","private":true}],"members":[]}"#.utf8))
            }
            prefetchCompleted.fulfill()
            return (200, Data(#"{"space":{"id":"space0000001","name":"S"},"channel":{"id":"chan00000001","name":"general"},"messages":[],"cursor":"9","hasMore":false}"#.utf8))
        }
        model.prefetch(space: space, channelID: "chan00000001")
        await fulfillment(of: [prefetchCompleted], timeout: 2)
        XCTAssertEqual(model.navigationCacheCounts.prefetches, 1)
        model.chat.onAccessRevoked?("chan00000001")
        XCTAssertEqual(model.navigationCacheCounts.prefetches, 0, "revocation must fence already completed speculative data")
        XCTAssertEqual(model.navigationCacheCounts.visited, 0)
    }

    @MainActor
    func testWarmPrefetchCannotBypassDeniedSpaceRefresh() async {
        let model = AppModel(api: client())
        let space = Space(id: "space0000001", name: "S", ownerId: "owner0000001", demo: nil)
        let prefetchCompleted = expectation(description: "warm prefetch completed")
        var denyRefresh = false
        MockURLProtocol.handler = { request in
            if request.url!.path == "/api/spaces/space0000001" {
                if denyRefresh { return (403, Data(#"{"error":"Membership ended"}"#.utf8)) }
                return (200, Data(#"{"space":{"id":"space0000001","name":"S","ownerId":"owner0000001"},"channels":[{"id":"chan00000001","spaceId":"space0000001","name":"general","private":true}],"members":[]}"#.utf8))
            }
            prefetchCompleted.fulfill()
            return (200, Data(#"{"space":{"id":"space0000001","name":"S"},"channel":{"id":"chan00000001","name":"general"},"messages":[],"cursor":"9","hasMore":false}"#.utf8))
        }
        model.prefetch(space: space)
        await fulfillment(of: [prefetchCompleted], timeout: 2)
        denyRefresh = true

        await model.select(space: space)

        XCTAssertNil(model.selectedSpaceID)
        XCTAssertNil(model.selectedChannelID)
        XCTAssertNil(model.detail)
        XCTAssertNil(model.chat.currentAuthor, "denied navigation must not create a chat session")
        XCTAssertEqual(model.navigationCacheCounts.prefetches, 0)
        XCTAssertEqual(model.navigationCacheCounts.visited, 0)
    }

    @MainActor
    func testInvalidationFencesBlockedNavigationCompletion() async {
        let model = AppModel(api: client())
        let space = Space(id: "space0000001", name: "S", ownerId: "owner0000001", demo: nil)
        let detailStarted = expectation(description: "navigation detail started")
        var delayedDetail: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path == "/api/spaces/space0000001" else { return false }
            delayedDetail = request
            detailStarted.fulfill()
            return true
        }
        MockURLProtocol.handler = { _ in throw URLError(.badURL) }
        let navigation = Task { await model.select(space: space) }
        await fulfillment(of: [detailStarted], timeout: 2)

        model.chat.onAccessRevoked?("chan00000001")
        delayedDetail?.respond(status: 200, data: Data(#"{"space":{"id":"space0000001","name":"S","ownerId":"owner0000001"},"channels":[],"members":[]}"#.utf8))
        await navigation.value

        XCTAssertNil(model.selectedSpaceID)
        XCTAssertNil(model.detail)
        XCTAssertNil(model.openingSpaceID, "defer must clear opening state")
        XCTAssertNil(model.chat.currentAuthor)
    }

    @MainActor
    func testNavigationReturnReusesRetainedCursorInsteadOfFirstPage() async throws {
        let model = AppModel(api: client())
        let first = Channel(id: "chan00000001", spaceId: "space0000001", name: "one", private: false)
        let second = Channel(id: "chan00000002", spaceId: "space0000001", name: "two", private: false)
        let space = Space(id: "space0000001", name: "S", ownerId: "owner0000001", demo: nil)
        let detail = SpaceDetail(space: space, channels: [first, second], members: [])
        var firstHistoryReads = 0
        MockURLProtocol.handler = { request in
            switch request.url!.path {
            case "/api/spaces/space0000001":
                return (200, try JSONEncoder().encode(detail))
            case "/api/chat/channels/chan00000001/messages":
                firstHistoryReads += 1
                return (200, Data(#"{"space":{"id":"space0000001","name":"S"},"channel":{"id":"chan00000001","name":"one"},"messages":[],"cursor":"41","hasMore":false}"#.utf8))
            case "/api/chat/channels/chan00000002/messages":
                return (200, Data(#"{"space":{"id":"space0000001","name":"S"},"channel":{"id":"chan00000002","name":"two"},"messages":[],"cursor":"52","hasMore":false}"#.utf8))
            case "/api/chat/session":
                return (200, Data(#"{"token":"chat-secret","author":{"id":"me","name":"Me","isGuest":false}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        model.detail = detail
        await model.select(channel: first)
        await model.select(channel: second)
        await model.select(channel: first)
        XCTAssertEqual(firstHistoryReads, 1, "returning should resume from the retained cursor, not refetch page one")
        XCTAssertEqual(model.chat.currentSnapshot()?.cursor, "41")
    }
}
