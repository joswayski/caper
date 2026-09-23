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
