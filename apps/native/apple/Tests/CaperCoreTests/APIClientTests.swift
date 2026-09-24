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
        XCTAssertEqual(model.selectedSpaceID, "space0000002")
        delayedDetail?.respond(status: 200, data: Data(#"{"space":{"id":"space0000002","name":"Next","ownerId":"owner0000001"},"channels":[],"members":[]}"#.utf8))
        await selection.value
        XCTAssertEqual(model.voice.phase, .connected)
        XCTAssertEqual(model.voice.context?.channelID, "chan00000001")
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
}
