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

private func requestBodyData(_ request: URLRequest) throws -> Data? {
    if let body = request.httpBody { return body }
    guard let stream = request.httpBodyStream else { return nil }
    stream.open()
    defer { stream.close() }
    var body = Data()
    var buffer = [UInt8](repeating: 0, count: 4096)
    while true {
        let count = stream.read(&buffer, maxLength: buffer.count)
        if count < 0 { throw try XCTUnwrap(stream.streamError) }
        if count == 0 { break }
        body.append(buffer, count: count)
    }
    return body
}

private func chatHistory(_ channel: String, sequences: [Int], cursor: Int, hasMore: Bool, label: String = "message") -> Data {
    let messages = sequences.map { sequence in
        """
        {"id":"m\(sequence)","channelId":"\(channel)","seq":"\(sequence)","author":{"id":"u","name":"User","isGuest":false},"content":{"version":1,"type":"text","text":"\(label) \(sequence)"},"createdAt":"now","clientMessageId":"c\(sequence)"}
        """
    }.joined(separator: ",")
    return Data("""
    {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channel)","name":"general"},"messages":[\(messages)],"cursor":"\(cursor)","hasMore":\(hasMore)}
    """.utf8)
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
    func testRepeatThreadOpenKeepsPendingLoadAndReusesPaginationUntilHistoryReset() async throws {
        let channel = "chan00000001"
        let author = ChatAuthor(id: "other", name: "Other", isGuest: false)
        let root = ChatMessage(id: "Message00000001", channelId: channel, seq: "1", author: author,
                               content: ChatContent(version: 1, type: "text", text: "Parent"), createdAt: "now", clientMessageId: "root")
        let latest = ChatMessage(id: "Message00000003", channelId: channel, seq: "3", author: author,
                                 content: ChatContent(version: 1, type: "text", text: "Latest reply"), createdAt: "now", clientMessageId: "latest", threadRootId: root.id)
        let older = ChatMessage(id: "Message00000002", channelId: channel, seq: "2", author: author,
                                content: ChatContent(version: 1, type: "text", text: "Older reply"), createdAt: "now", clientMessageId: "older", threadRootId: root.id)
        let history = ChatHistory(space: HistoryIdentity(id: "space0000001", name: "Space"),
                                  channel: HistoryIdentity(id: channel, name: "general"), messages: [root], cursor: "3", hasMore: false)
        let started = expectation(description: "initial thread request started")
        var held: MockURLProtocol?
        var threadRequests = 0
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path.hasSuffix("/thread") == true, held == nil else { return false }
            threadRequests += 1; held = request; started.fulfill(); return true
        }
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/chat/session" {
                return (200, Data(#"{"token":"chat-secret","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            }
            guard request.url?.path.hasSuffix("/thread") == true else { throw URLError(.badURL) }
            threadRequests += 1
            XCTAssertEqual(request.url?.query, "before=3")
            return (200, try JSONEncoder().encode(ThreadHistory(root: root, messages: [older], cursor: "3", hasMore: false)))
        }
        let chat = ChatModel(api: client())
        await chat.open(history: history, displayName: "Me")
        let opening = Task { await chat.openThread(root.id) }
        await fulfillment(of: [started], timeout: 2)
        await chat.openThread(root.id)
        XCTAssertEqual(threadRequests, 1)
        XCTAssertTrue(chat.threadLoading)
        held?.respond(status: 200, data: try JSONEncoder().encode(ThreadHistory(root: root, messages: [latest], cursor: "3", hasMore: true)))
        await opening.value
        chat.threadDraft = "Keep this draft"
        await chat.openThread(root.id)
        XCTAssertEqual(threadRequests, 1)
        await chat.loadThread(older: true)
        XCTAssertEqual(threadRequests, 2)
        chat.closeThread()
        await chat.openThread(root.id)
        XCTAssertEqual(threadRequests, 2)
        XCTAssertFalse(chat.threadLoading)
        XCTAssertFalse(chat.threadHasMore)
        XCTAssertEqual(chat.threadDraft, "Keep this draft")
        XCTAssertEqual(chat.messages.filter { $0.threadRootId == root.id }.map(\.id), [older.id, latest.id])
        // A new authorized history replaces reply rows and must discard their cache.
        await chat.preview(history: history)
        MockURLProtocol.handler = { _ in
            threadRequests += 1
            return (200, try JSONEncoder().encode(ThreadHistory(root: root, messages: [], cursor: "3", hasMore: false)))
        }
        await chat.openThread(root.id)
        XCTAssertEqual(threadRequests, 3)
        XCTAssertTrue(chat.messages.allSatisfy { $0.threadRootId == nil })
        await chat.stop()
    }

    func testReactionPUTUsesChatTokenBodyAndFifteenCharacterMessageID() async throws {
        let channel = "Channel12345"
        let message = "Message00000001"
        MockURLProtocol.handler = { request in
            XCTAssertEqual(request.httpMethod, "PUT")
            XCTAssertEqual(request.url?.path, "/api/chat/channels/\(channel)/messages/\(message)/reactions")
            XCTAssertEqual(request.value(forHTTPHeaderField: "x-caper-chat-token"), "chat-secret")
            XCTAssertEqual(request.value(forHTTPHeaderField: "authorization"), "Bearer account-secret")
            var bodyData = request.httpBody ?? Data()
            // URLSession may turn the body into a stream before URLProtocol sees it.
            if let stream = request.httpBodyStream {
                stream.open()
                defer { stream.close() }
                var buffer = [UInt8](repeating: 0, count: 1_024)
                while true {
                    let count = stream.read(&buffer, maxLength: buffer.count)
                    guard count >= 0 else { throw stream.streamError ?? URLError(.cannotDecodeRawData) }
                    if count == 0 { break }
                    bodyData.append(contentsOf: buffer.prefix(count))
                }
            }
            let body = try XCTUnwrap(JSONSerialization.jsonObject(with: bodyData) as? [String: Any])
            XCTAssertEqual(body["emoji"] as? String, "👍")
            XCTAssertEqual(body["active"] as? Bool, true)
            return (200, Data("""
            {"type":"message.reactions","schemaVersion":1,"channelId":"\(channel)","seq":"9","messageId":"\(message)","reactions":[{"emoji":"👍","authorIds":["self"]}]}
            """.utf8))
        }
        let event = try await client().setReaction(channelID: channel, messageID: message, sessionToken: "chat-secret", emoji: "👍", active: true)
        XCTAssertEqual(event.seq, "9")
        do {
            _ = try await client().setReaction(channelID: channel, messageID: "only-twelve1", sessionToken: "chat-secret", emoji: "👍", active: true)
            XCTFail("Expected local message ID rejection")
        } catch let error as APIError { XCTAssertEqual(error.status, 400) }
    }

    func testReactorsGETUsesAccountAuthorizationAndRejectsAnotherMessage() async throws {
        let channel = "Channel12345"
        let message = "Message00000001"
        var routes: [String] = []
        MockURLProtocol.handler = { request in
            routes.append("\(request.httpMethod ?? "") \(request.url?.path ?? "")")
            XCTAssertEqual(request.value(forHTTPHeaderField: "authorization"), "Bearer account-secret")
            XCTAssertNil(request.value(forHTTPHeaderField: "x-caper-chat-token"), "who reacted needs no chat session")
            return (200, Data("""
            {"messageId":"\(message)","reactionSeq":"12","reactions":[{"emoji":"👍","authors":[{"id":"bob","username":"bob","displayName":"Bob B","avatarId":101},{"id":"alice","username":null,"displayName":null,"avatarId":100}]}]}
            """.utf8))
        }
        let list = try await client().reactors(channelID: channel, messageID: message)
        XCTAssertEqual(routes, ["GET /api/chat/channels/\(channel)/messages/\(message)/reactions"])
        XCTAssertEqual(list.reactions.first?.authors.map(\.name), ["Bob B", "Someone"])
        do {
            _ = try await client().reactors(channelID: channel, messageID: "Message00000002")
            XCTFail("A list for another message must be rejected")
        } catch let error as APIError { XCTAssertEqual(error.status, 502) }
        do {
            _ = try await client().reactors(channelID: channel, messageID: "only-twelve1")
            XCTFail("Expected local message ID rejection")
        } catch let error as APIError { XCTAssertEqual(error.status, 400) }
    }

    @MainActor
    func testReactorListsAreCachedByReactionSequenceAndRefetchedAfterChanges() async throws {
        let channel = "chan00000001"
        let messageID = "Message00000001"
        var reactorRequests = 0
        MockURLProtocol.handler = { request in
            switch request.url?.path {
            case "/api/chat/session":
                return (200, Data(#"{"token":"chat-secret","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case "/api/chat/channels/\(channel)/messages/\(messageID)/reactions" where request.httpMethod == "GET":
                reactorRequests += 1
                if reactorRequests == 3 { return (500, Data(#"{"error":"unavailable"}"#.utf8)) }
                let third = reactorRequests == 1 ? "" : #",{"id":"third","username":"third","displayName":"Third T","avatarId":4}"#
                return (200, Data("""
                {"messageId":"\(messageID)","reactionSeq":"\(reactorRequests)","reactions":[{"emoji":"👍","authors":[{"id":"other","username":"other","displayName":"Other O","avatarId":3}\(third)]}]}
                """.utf8))
            default: throw URLError(.badURL)
            }
        }
        let message = ChatMessage(id: messageID, channelId: channel, seq: "1",
                                  author: ChatAuthor(id: "other", name: "Other", isGuest: false),
                                  content: ChatContent(version: 1, type: "text", text: "Hello"), createdAt: "now",
                                  clientMessageId: "client", reactions: [MessageReaction(emoji: "👍", authorIds: ["other"])],
                                  reactionSeq: "1")
        let history = ChatHistory(space: HistoryIdentity(id: "Space1234567", name: "Space"),
                                  channel: HistoryIdentity(id: channel, name: "general"), messages: [message], cursor: "1", hasMore: false)
        let chat = ChatModel(api: client())
        await chat.open(history: history, displayName: "Me")
        let other = ReactorPerson(id: "other", username: "other", displayName: "Other O", avatarId: 3)
        let third = ReactorPerson(id: "third", username: "third", displayName: "Third T", avatarId: 4)

        XCTAssertEqual(chat.reactorsState(for: chat.messages[0], emoji: "👍", viewerID: "self"), .loading)
        _ = await chat.requestReactors(messageID: messageID)?.value
        XCTAssertEqual(chat.reactorsState(for: chat.messages[0], emoji: "👍", viewerID: "self"), .loaded([other]))
        XCTAssertNil(chat.requestReactors(messageID: messageID), "an unchanged reaction sequence reuses the list")
        XCTAssertEqual(reactorRequests, 1)

        chat.receive(["type": "message.reactions", "schemaVersion": 1, "channelId": channel, "seq": "2",
                      "messageId": messageID, "reactions": [["emoji": "👍", "authorIds": ["other", "third"]]]],
                     generation: 1, channelID: channel)
        XCTAssertEqual(chat.messages[0].reactionSeq, "2")
        XCTAssertEqual(chat.reactorsState(for: chat.messages[0], emoji: "👍", viewerID: "self"), .loading,
                       "a stale list cannot name the new reactor and is refetched")
        _ = await chat.requestReactors(messageID: messageID)?.value
        XCTAssertEqual(reactorRequests, 2)
        XCTAssertEqual(chat.reactorsState(for: chat.messages[0], emoji: "👍", viewerID: "self"), .loaded([other, third]))

        chat.receive(["type": "message.reactions", "schemaVersion": 1, "channelId": channel, "seq": "3",
                      "messageId": messageID, "reactions": [["emoji": "👍", "authorIds": ["other", "third", "fourth"]]]],
                     generation: 1, channelID: channel)
        _ = await chat.requestReactors(messageID: messageID)?.value
        XCTAssertEqual(reactorRequests, 3)
        XCTAssertEqual(chat.reactorsState(for: chat.messages[0], emoji: "👍", viewerID: "self"), .failed)
        await chat.stop()
        XCTAssertNil(chat.requestReactors(messageID: messageID), "a stopped chat has no message to load")
    }

    func testPinPUTUsesChatTokenAndDecodesMessagePayload() async throws {
        let channel = "Channel12345", message = "Message00000001"
        MockURLProtocol.handler = { request in
            XCTAssertEqual(request.httpMethod, "PUT")
            XCTAssertEqual(request.url?.path, "/api/chat/channels/\(channel)/messages/\(message)/pin")
            XCTAssertEqual(request.value(forHTTPHeaderField: "x-caper-chat-token"), "chat-secret")
            let body = try XCTUnwrap(try requestBodyData(request))
            XCTAssertEqual((try JSONSerialization.jsonObject(with: body) as? [String: Bool])?["active"], true)
            return (200, Data("""
            {"type":"message.pin","schemaVersion":1,"channelId":"\(channel)","seq":"9","message":{"id":"\(message)","channelId":"\(channel)","seq":"2","author":{"id":"u","name":"User","isGuest":false},"content":{"version":1,"type":"text","text":"old row"},"createdAt":"now","clientMessageId":"c2","pin":{"author":{"id":"p","name":"Pinner","isGuest":false},"createdAt":"now"},"pinSeq":"9"}}
            """.utf8))
        }
        let event = try await client().setPin(channelID: channel, messageID: message, sessionToken: "chat-secret", active: true)
        XCTAssertEqual(event.message.seq, "2", "pinning retains original creation sequence")
        XCTAssertEqual(event.message.pinSeq, event.seq)
        XCTAssertEqual(event.message.pin?.author.name, "Pinner")
    }

    func testHistoryDefaultsMissingPinnedMessagesToEmpty() throws {
        let history = try JSONDecoder().decode(ChatHistory.self, from: chatHistory("Channel12345", sequences: [1], cursor: 1, hasMore: false))
        XCTAssertEqual(history.pinnedMessages.count, 0)
    }

    @MainActor
    private func waitUntil(_ predicate: @escaping @MainActor () -> Bool) async {
        for _ in 0..<100 where !predicate() { try? await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(predicate(), "timed out waiting for chat recovery")
    }

    func testSpacesResponseDefaultsMissingInvitationsToEmpty() throws {
        let response = try JSONDecoder().decode(SpacesResponse.self, from: Data(#"{"spaces":[],"limits":{"ownedSpaces":2,"totalSpaces":5,"channelsPerSpace":10}}"#.utf8))
        XCTAssertEqual(response.invitations, [])
    }

    func testChannelAndDetailDecodeLegacyAndMembershipFields() throws {
        let legacy = try JSONDecoder().decode(Channel.self, from: Data(#"{"id":"Chan12345678","spaceId":"Space1234567","name":"general","private":false}"#.utf8))
        XCTAssertTrue(legacy.joined)
        let detail = try JSONDecoder().decode(SpaceDetail.self, from: Data(#"{"space":{"id":"Space1234567","name":"Studio","ownerId":"Owner1234567"},"channels":[{"id":"Chan12345678","spaceId":"Space1234567","name":"lobby","private":false,"joined":false}],"members":[],"channelInvitations":[{"channel":{"id":"Priv12345678","spaceId":"Space1234567","name":"private","private":true,"joined":false},"inviter":{"username":"host","displayName":"Host"}}]}"#.utf8))
        XCTAssertFalse(detail.channels[0].joined)
        XCTAssertEqual(detail.channelInvitations.first?.inviter.username, "host")
        let oldDetail = try JSONDecoder().decode(SpaceDetail.self, from: Data(#"{"space":{"id":"Space1234567","name":"Studio","ownerId":"Owner1234567"},"channels":[],"members":[]}"#.utf8))
        XCTAssertTrue(oldDetail.channelInvitations.isEmpty)
    }

    func testChannelMembershipAndInvitationRoutes() async throws {
        var routes: [String] = []
        MockURLProtocol.handler = { request in
            routes.append("\(request.httpMethod!) \(request.url!.path)")
            if request.httpMethod == "POST" { return (200, Data(#"{"id":"Chan12345678","spaceId":"Space1234567","name":"general","private":false,"joined":true}"#.utf8)) }
            return (204, Data())
        }
        let api = client()
        _ = try await api.joinChannel(spaceID: "Space1234567", channelID: "Chan12345678")
        try await api.leaveChannel(spaceID: "Space1234567", channelID: "Chan12345678")
        _ = try await api.acceptChannelInvitation(spaceID: "Space1234567", channelID: "Chan12345678")
        try await api.declineChannelInvitation(spaceID: "Space1234567", channelID: "Chan12345678")
        XCTAssertEqual(routes, [
            "POST /api/spaces/Space1234567/channels/Chan12345678/membership",
            "DELETE /api/spaces/Space1234567/channels/Chan12345678/membership",
            "POST /api/spaces/Space1234567/channels/Chan12345678/invitation",
            "DELETE /api/spaces/Space1234567/channels/Chan12345678/invitation",
        ])
    }

    func testInvitationDecodesInviterAndLegacyMetadata() throws {
        let legacy = try JSONDecoder().decode(Space.self, from: Data(#"{"id":"Space1234567","name":"Studio","ownerId":"Owner1234567"}"#.utf8))
        XCTAssertNil(legacy.inviter)
        let invitation = try JSONDecoder().decode(Space.self, from: Data(#"{"id":"Space1234567","name":"Studio","ownerId":"Owner1234567","inviter":{"username":"host_user","displayName":"Space Host"}}"#.utf8))
        XCTAssertEqual(invitation.inviter?.username, "host_user")
        XCTAssertEqual(invitation.inviter?.displayName, "Space Host")
    }

    func testInvitationMembershipEndpoints() async throws {
        let spaceID = "Space1234567"
        let userID = "Member123456"
        var requests: [String] = []
        MockURLProtocol.handler = { request in
            requests.append("\(request.httpMethod ?? "") \(request.url!.path)")
            switch (request.httpMethod, request.url?.path) {
            case ("GET", "/api/spaces/\(spaceID)/invitations"):
                return (200, Data(#"{"members":[]}"#.utf8))
            case ("POST", "/api/spaces/\(spaceID)/invitation"):
                return (200, Data(#"{"id":"Space1234567","name":"Invited","ownerId":"Owner1234567"}"#.utf8))
            default: return (204, Data())
            }
        }
        let api = client()
        let pending = try await api.spaceInvitations(spaceID: spaceID)
        let accepted = try await api.acceptSpaceInvitation(spaceID: spaceID)
        XCTAssertEqual(pending, [])
        XCTAssertEqual(accepted.name, "Invited")
        try await api.declineSpaceInvitation(spaceID: spaceID)
        try await api.cancelSpaceInvitation(spaceID: spaceID, userID: userID)
        XCTAssertEqual(requests, [
            "GET /api/spaces/\(spaceID)/invitations",
            "POST /api/spaces/\(spaceID)/invitation",
            "DELETE /api/spaces/\(spaceID)/invitation",
            "DELETE /api/spaces/\(spaceID)/invitations/\(userID)",
        ])
    }

    func testInviteUsernameNormalizationAndValidation() {
        XCTAssertEqual(WorkspaceValidation.normalizeUsername(" Alice-TEAM! "), "aliceteam")
        XCTAssertNil(WorkspaceValidation.usernameError("alice_123"))
        XCTAssertNotNil(WorkspaceValidation.usernameError("Alice"))
        XCTAssertNotNil(WorkspaceValidation.usernameError("ab"))
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
            "id": "message-one", "channelId": channel, "seq": "1", "clientMessageId": command.id.lowercased(),
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
        {"id":"m1","channelId":"\(channel)","seq":"1","clientMessageId":"\(command.id.lowercased())","author":{"id":"self","name":"Me","isGuest":false},"content":{"version":1,"type":"text","text":"submitted first"},"createdAt":"2026-01-01T00:00:00Z"}
        """.utf8)
        held?.respond(status: 200, data: response)
        await send.value
        XCTAssertNil(chat.pendingMessage)
        XCTAssertNil(chat.error, "a canonical UUID response is a successful send, not an invalid message")
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
        model.selectedDirectMessageID = "Dm1234567890"
        model.spacesLoaded = true
        model.spacesError = "offline"
        model.configurePush(available: true, enabled: true)
        var locallyDisabled = false
        model.disablePushLocally = { locallyDisabled = true }
        model.setPushEnabled = { _ in
            XCTFail("logout must not await optional push unregister")
            try? await Task.sleep(for: .seconds(2))
        }
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
        XCTAssertNil(model.selectedDirectMessageID)
        XCTAssertFalse(model.spacesLoaded)
        XCTAssertNil(model.spacesError)
        XCTAssertTrue(locallyDisabled)
        XCTAssertFalse(model.pushEnabled)
        XCTAssertFalse(model.pushAvailable)
        XCTAssertNil(store.token)
        delayedRevoke?.respond(status: 204)
        await logout.value
    }

    @MainActor
    func testDelayedReconnectRefreshKeepsVisibleHistoryAndDraft() async throws {
        let channel = "Aaaaaaaaaaaa"
        var historyRequests = 0
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/chat/session" { return (200, Data(#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#.utf8)) }
            historyRequests += 1
            return (200, chatHistory(channel, sequences: [1, 2, 3, 4], cursor: 4, hasMore: true))
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channel, displayName: "User")
        chat.draft = "unfinished reply"

        let refreshStarted = expectation(description: "refresh started")
        var delayedRefresh: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path.hasSuffix("/messages") == true else { return false }
            delayedRefresh = request; refreshStarted.fulfill(); return true
        }
        chat.receive(["type": "resync_required"], generation: 1, channelID: channel)
        await fulfillment(of: [refreshStarted], timeout: 2)
        XCTAssertEqual(chat.messages.map(\.seq), ["1", "2", "3", "4"])
        XCTAssertEqual(chat.draft, "unfinished reply")
        XCTAssertTrue(chat.loading)
        await chat.loadOlder()
        XCTAssertEqual(historyRequests, 1, "pagination must not start while refresh is resetting history")

        delayedRefresh?.respond(status: 200, data: chatHistory(channel, sequences: [3, 4, 5], cursor: 5, hasMore: false, label: "fresh"))
        await waitUntil { !chat.loading }
        XCTAssertEqual(chat.messages.map(\.seq), ["1", "2", "3", "4", "5"])
        XCTAssertEqual(chat.messages.first(where: { $0.seq == "3" })?.content.text, "fresh 3")
        XCTAssertTrue(chat.hasMore, "the retained older prefix keeps its pagination state")
        XCTAssertEqual(chat.draft, "unfinished reply")
        await chat.stop()
    }

    @MainActor
    func testReconnectRefreshRetainsOverlapButReplacesAcrossGap() async throws {
        let channel = "Aaaaaaaaaaaa"
        var response = chatHistory(channel, sequences: [1, 2, 3, 4], cursor: 4, hasMore: true)
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/chat/session" { return (200, Data(#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#.utf8)) }
            return (200, response)
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channel, displayName: "User")

        response = chatHistory(channel, sequences: [4, 5], cursor: 5, hasMore: false, label: "overlap")
        chat.receive(["type": "resync_required"], generation: 1, channelID: channel)
        await waitUntil { !chat.loading && chat.messages.last?.seq == "5" }
        XCTAssertEqual(chat.messages.map(\.seq), ["1", "2", "3", "4", "5"])
        XCTAssertEqual(chat.messages.first(where: { $0.seq == "4" })?.content.text, "overlap 4")

        response = chatHistory(channel, sequences: [7, 8], cursor: 8, hasMore: false, label: "gap")
        chat.receive(["type": "resync_required"], generation: 2, channelID: channel)
        await waitUntil { !chat.loading && chat.messages.last?.seq == "8" }
        XCTAssertEqual(chat.messages.map(\.seq), ["7", "8"], "a gap after durable cursor 5 must replace unpageable retained history")
        XCTAssertFalse(chat.hasMore)
        await chat.stop()
    }

    @MainActor
    func testReconnectRefreshDropsOlderPagesWhenMissingSequenceMayBeReaction() async throws {
        let channel = "Aaaaaaaaaaaa"
        var response = chatHistory(channel, sequences: [1, 2, 3, 4, 5], cursor: 5, hasMore: true)
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/chat/session" { return (200, Data(#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#.utf8)) }
            return (200, response)
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channel, displayName: "User")

        response = chatHistory(channel, sequences: [6], cursor: 7, hasMore: false, label: "fresh")
        chat.receive(["type": "resync_required"], generation: 1, channelID: channel)
        await waitUntil { !chat.loading && chat.currentSnapshot()?.cursor == "7" }

        XCTAssertEqual(chat.messages.map(\.seq), ["6"], "missing sequence 7 may be a reaction on an older row, so cached pages are unsafe")
        XCTAssertFalse(chat.hasMore)
        response = chatHistory(channel, sequences: [], cursor: 7, hasMore: false)
        chat.receive(["type": "resync_required"], generation: 2, channelID: channel)
        await waitUntil { !chat.loading && chat.messages.isEmpty }
        XCTAssertFalse(chat.hasMore, "an empty authoritative page must clear cached history even at the same cursor")
        await chat.stop()
    }

    @MainActor
    func testTransientReconnectFailureAndRetryPreserveTimeline() async throws {
        let channel = "Aaaaaaaaaaaa"
        var historyStatus = 200
        var history = chatHistory(channel, sequences: [1, 2, 3], cursor: 3, hasMore: true)
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/chat/session" { return (200, Data(#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#.utf8)) }
            return (historyStatus, historyStatus == 200 ? history : Data(#"{"error":"temporarily unavailable"}"#.utf8))
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channel, displayName: "User")
        chat.draft = "keep this"
        historyStatus = 503
        chat.receive(["type": "resync_required"], generation: 1, channelID: channel)
        await waitUntil { !chat.loading && chat.error?.contains("temporarily unavailable") == true }
        XCTAssertEqual(chat.messages.map(\.seq), ["1", "2", "3"])
        XCTAssertEqual(chat.draft, "keep this")

        historyStatus = 200
        history = chatHistory(channel, sequences: [3, 4], cursor: 4, hasMore: false, label: "retried")
        await chat.retryLoad()
        XCTAssertEqual(chat.messages.map(\.seq), ["1", "2", "3", "4"])
        XCTAssertEqual(chat.draft, "keep this")
        await chat.stop()
    }

    @MainActor
    func testDeniedReconnectRefreshClearsRetainedPrivateData() async throws {
        let channel = "Aaaaaaaaaaaa"
        var denied = false
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/chat/session" { return (200, Data(#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#.utf8)) }
            if denied { return (403, Data(#"{"error":"Access ended"}"#.utf8)) }
            return (200, chatHistory(channel, sequences: [1, 2], cursor: 2, hasMore: true))
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channel, displayName: "User")
        chat.draft = "private draft"
        let revoked = expectation(description: "access revoked")
        chat.onAccessRevoked = { revokedChannel in
            XCTAssertEqual(revokedChannel, channel)
            revoked.fulfill()
        }
        denied = true
        chat.receive(["type": "resync_required"], generation: 1, channelID: channel)
        await fulfillment(of: [revoked], timeout: 2)
        XCTAssertTrue(chat.messages.isEmpty)
        XCTAssertTrue(chat.draft.isEmpty)
        XCTAssertNil(chat.currentAuthor)
        XCTAssertFalse(chat.hasMore)
        XCTAssertNotNil(chat.error)
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

        chat.receiveGatewayState(.connected, error: nil)
        XCTAssertEqual(chat.liveState, .connected)
        chat.error = "A genuine send failure"
        chat.receiveGatewayState(.reconnecting, error: "Live updates disconnected. Reconnecting…")
        XCTAssertEqual(chat.liveState, .reconnecting)
        XCTAssertEqual(chat.error, "A genuine send failure")
        chat.receiveGatewayState(.connected, error: nil)
        XCTAssertEqual(chat.error, "A genuine send failure", "transport recovery must not erase chat failures")
        chat.receiveGatewayState(.connected, error: "Channel access was revoked")
        XCTAssertEqual(chat.error, "Channel access was revoked", "subscription failures must remain visible")
        await chat.stop()
        XCTAssertTrue(chat.messages.isEmpty)
        XCTAssertTrue(chat.draft.isEmpty)
        XCTAssertEqual(chat.liveState, .disconnected)
        for lateState in [GatewayState.connecting, .connected, .reconnecting] {
            chat.receiveGatewayState(lateState, error: "Late gateway failure")
            XCTAssertEqual(chat.liveState, .disconnected)
            XCTAssertNil(chat.error)
        }
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
    func testDeniedVoiceTargetDoesNotEvictDifferentCallOrChangeSelectedChat() async {
        let model = AppModel(api: client())
        let text = Channel(id: "chan00000001", spaceId: "space0000001", name: "general", private: false)
        let target = Channel(id: "chan00000002", spaceId: "space0000001", name: "design", private: true)
        let space = Space(id: "space0000001", name: "Fixture", ownerId: "owner0000001", demo: nil)
        model.detail = SpaceDetail(space: space, channels: [text, target], members: [])
        model.selectedSpaceID = space.id
        model.selectedChannelID = text.id
        model.account = Account(id: "member000001", username: "member", displayName: "Member")
        model.voice.phase = .connected
        model.voice.context = VoiceContext(channelID: text.id, channelName: text.name, spaceID: space.id, spaceName: space.name)
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/spaces/\(space.id)" {
                return (200, Data("""
                {"space":{"id":"\(space.id)","name":"Fixture","ownerId":"owner0000001"},"channels":[
                {"id":"\(text.id)","spaceId":"\(space.id)","name":"general","private":false},
                {"id":"\(target.id)","spaceId":"\(space.id)","name":"design","private":true}],"members":[]}
                """.utf8))
            }
            if request.url?.path.contains(target.id) == true { return (403, Data(#"{"error":"Private channel denied"}"#.utf8)) }
            throw URLError(.badURL)
        }
        await model.joinVoice(channel: target)
        XCTAssertEqual(model.selectedChannelID, text.id)
        XCTAssertEqual(model.voice.context?.channelID, text.id)
        XCTAssertEqual(model.voice.phase, .connected)
        XCTAssertTrue(model.voicePresence.roster(for: target.id).isEmpty)
    }

    @MainActor
    func testEachChannelActionUsesItsOwnAvailabilityAndRevocation() async {
        let model = AppModel(api: client())
        let text = Channel(id: "chan00000001", spaceId: "space0000001", name: "general", private: false)
        let other = Channel(id: "chan00000002", spaceId: text.spaceId, name: "design", private: true)
        let space = Space(id: text.spaceId, name: "Fixture", ownerId: "owner0000001", demo: nil)
        model.detail = SpaceDetail(space: space, channels: [text, other], members: [])
        model.selectedChannelID = text.id
        MockURLProtocol.handler = { request in
            switch request.url?.path {
            case "/api/channels/\(text.id)/media/status": return (200, Data(#"{"enabled":false}"#.utf8))
            case "/api/channels/\(other.id)/media/status": return (200, Data(#"{"enabled":true}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        await model.refreshVoiceAvailability(channel: text)
        await model.refreshVoiceAvailability(channel: other)
        XCTAssertEqual(model.voiceAvailable(in: text), false)
        XCTAssertEqual(model.voiceAvailable(in: other), true)
        XCTAssertEqual(model.selectedChannelID, text.id)
        model.voicePresence.revoke(channelID: other.id)
        XCTAssertEqual(model.voiceAvailable(in: other), false, "A revoked channel keeps its action disabled")
    }

    @MainActor
    func testLateChannelAvailabilityCannotPublishAfterLogout() async {
        let model = AppModel(api: client())
        let channel = Channel(id: "chan00000001", spaceId: "space0000001", name: "general", private: false)
        let space = Space(id: channel.spaceId, name: "Fixture", ownerId: "owner0000001", demo: nil)
        model.detail = SpaceDetail(space: space, channels: [channel], members: [])
        model.selectedChannelID = channel.id
        let requested = expectation(description: "channel status request")
        var held: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path == "/api/channels/\(channel.id)/media/status" else { return false }
            held = request; requested.fulfill(); return true
        }
        MockURLProtocol.handler = { _ in (204, Data()) }
        let checking = Task { await model.refreshVoiceAvailability(channel: channel) }
        await fulfillment(of: [requested], timeout: 2)
        await model.logout()
        held?.respond(status: 200, data: Data(#"{"enabled":true}"#.utf8))
        await checking.value
        XCTAssertTrue(model.voiceAvailability.isEmpty)
        XCTAssertNil(model.voiceAvailable(in: channel))
    }

    @MainActor
    func testFreshUnjoinedVoiceTargetPreservesExistingCall() async {
        let model = AppModel(api: client())
        let current = Channel(id: "chan00000001", spaceId: "space0000001", name: "general", private: false)
        let target = Channel(id: "chan00000002", spaceId: current.spaceId, name: "design", private: false)
        let space = Space(id: current.spaceId, name: "Fixture", ownerId: "owner0000001", demo: nil)
        model.detail = SpaceDetail(space: space, channels: [current, target], members: [])
        model.selectedSpaceID = space.id
        model.selectedChannelID = current.id
        model.voice.phase = .connected
        model.voice.context = VoiceContext(channelID: current.id, channelName: current.name, spaceID: space.id, spaceName: space.name)
        var requests: [String] = []
        MockURLProtocol.handler = { request in
            let path = request.url!.path
            requests.append(path)
            if path == "/api/spaces/\(space.id)" {
                return (200, Data("""
                {"space":{"id":"\(space.id)","name":"Fixture","ownerId":"owner0000001"},"channels":[
                {"id":"\(current.id)","spaceId":"\(space.id)","name":"general","private":false,"joined":true},
                {"id":"\(target.id)","spaceId":"\(space.id)","name":"design","private":false,"joined":false}],"members":[]}
                """.utf8))
            }
            if path == "/api/chat/channels/\(target.id)/messages" {
                return (200, Data("""
                {"space":{"id":"\(space.id)","name":"Fixture"},"channel":{"id":"\(target.id)","name":"design"},"messages":[],"cursor":"0","hasMore":false}
                """.utf8))
            }
            throw URLError(.badURL)
        }
        await model.joinVoice(channel: target)
        XCTAssertEqual(model.voice.context?.channelID, current.id)
        XCTAssertEqual(model.voice.phase, .connected)
        XCTAssertEqual(model.selectedChannelID, current.id)
        XCTAssertEqual(requests, ["/api/spaces/\(space.id)"], "Readable history must not authorize an unjoined voice target")
    }

    @MainActor
    func testAcknowledgedChannelLeaveSurvivesFailedDetailRefresh() async {
        for privateChannel in [false, true] {
            let model = AppModel(api: client())
            let channel = Channel(id: "chan00000001", spaceId: "space0000001", name: "general", private: privateChannel)
            let space = Space(id: channel.spaceId, name: "Fixture", ownerId: "owner0000001", demo: nil)
            model.account = Account(id: "member000001", username: "member", displayName: "Member")
            model.detail = SpaceDetail(space: space, channels: [channel], members: [])
            model.selectedSpaceID = space.id
            model.selectedChannelID = channel.id
            model.chat.draft = "old conversation draft"
            MockURLProtocol.handler = { request in
                if request.httpMethod == "DELETE", request.url?.path == "/api/spaces/\(space.id)/channels/\(channel.id)/membership" { return (204, Data()) }
                return (503, Data(#"{"error":"TEST FIXTURE unavailable"}"#.utf8))
            }
            do {
                try await model.leaveChannel(channel)
                XCTFail("Detail refresh should fail")
            } catch {
                XCTAssertEqual((error as? APIError)?.status, 503)
            }
            XCTAssertFalse(model.detail?.channels.contains { $0.id == channel.id && $0.joined } == true)
            XCTAssertEqual(model.detail?.channels.contains { $0.id == channel.id }, !privateChannel)
            XCTAssertNil(model.selectedChannelID)
            XCTAssertTrue(model.chat.draft.isEmpty)
            XCTAssertTrue(model.chat.messages.isEmpty)
        }
    }

    @MainActor
    func testLeaveCancelsPendingPermissionCheckBeforeVoiceSwitch() async throws {
        let model = AppModel(api: client())
        let current = Channel(id: "chan00000001", spaceId: "space0000001", name: "general", private: false)
        let target = Channel(id: "chan00000002", spaceId: current.spaceId, name: "design", private: false)
        let space = Space(id: current.spaceId, name: "Fixture", ownerId: "owner0000001", demo: nil)
        model.detail = SpaceDetail(space: space, channels: [current, target], members: [])
        model.selectedSpaceID = space.id
        model.selectedChannelID = current.id
        model.voice.phase = .connected
        model.voice.context = VoiceContext(channelID: current.id, channelName: current.name, spaceID: space.id, spaceName: space.name)
        let requested = expectation(description: "target permission request")
        var held: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard held == nil, urlRequest.url?.path == "/api/spaces/\(space.id)" else { return false }
            held = request; requested.fulfill(); return true
        }
        MockURLProtocol.handler = { _ in throw URLError(.badURL) }
        let beforeClick = Int64(Date().timeIntervalSince1970 * 1_000)
        let joining = Task { await model.joinVoice(channel: target) }
        await fulfillment(of: [requested], timeout: 2)
        XCTAssertEqual(model.pendingVoiceChannelID, target.id)
        let clicked = model.pendingVoiceStartedAt
        XCTAssertGreaterThanOrEqual(clicked, beforeClick)
        XCTAssertLessThanOrEqual(clicked, Int64(Date().timeIntervalSince1970 * 1_000))
        await model.joinVoice(channel: target)
        XCTAssertEqual(model.pendingVoiceChannelID, target.id, "Duplicate taps cannot replace the pending authorization")
        XCTAssertEqual(model.pendingVoiceStartedAt, clicked, "Duplicate taps cannot restart the pending timer")
        XCTAssertNil(model.navigationError)
        model.leaveVoice()
        XCTAssertNil(model.pendingVoiceChannelID)
        held?.respond(status: 200, data: Data("""
        {"space":{"id":"\(space.id)","name":"Fixture","ownerId":"owner0000001"},"channels":[
        {"id":"\(current.id)","spaceId":"\(space.id)","name":"general","private":false},
        {"id":"\(target.id)","spaceId":"\(space.id)","name":"design","private":false}],"members":[]}
        """.utf8))
        await joining.value
        XCTAssertEqual(model.voice.phase, .idle)
        XCTAssertNil(model.voice.context, "The late permission response must not start a new call after Leave")
        XCTAssertEqual(model.selectedChannelID, current.id)
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

    @MainActor
    func testRepeatChannelSelectionPreservesDraftAndCancelsPendingNavigation() async throws {
        let model = AppModel(api: client())
        let first = Channel(id: "chan00000001", spaceId: "space0000001", name: "one", private: false)
        let second = Channel(id: "chan00000002", spaceId: first.spaceId, name: "two", private: false)
        let space = Space(id: first.spaceId, name: "S", ownerId: "owner0000001", demo: nil)
        let detail = SpaceDetail(space: space, channels: [first, second], members: [])
        var paths: [String] = []
        MockURLProtocol.handler = { request in
            let path = request.url!.path
            paths.append(path)
            switch path {
            case "/api/spaces/space0000001": return (200, try JSONEncoder().encode(detail))
            case "/api/chat/channels/chan00000001/messages":
                return (200, Data(#"{"space":{"id":"space0000001","name":"S"},"channel":{"id":"chan00000001","name":"one"},"messages":[],"cursor":"41","hasMore":false}"#.utf8))
            case "/api/chat/session":
                return (200, Data(#"{"token":"chat-secret","author":{"id":"me","name":"Me","isGuest":false}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        model.detail = detail
        await model.select(channel: first)
        model.chat.draft = "Keep this draft"
        let count = paths.count
        await model.select(channel: first)
        model.prefetch(space: space, channelID: first.id)
        XCTAssertEqual(paths.count, count)
        XCTAssertEqual(model.navigationCacheCounts.prefetches, 0)
        XCTAssertEqual(model.chat.draft, "Keep this draft")
        XCTAssertEqual(model.chat.currentSnapshot()?.cursor, "41")

        let started = expectation(description: "second channel navigation started")
        var held: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path == "/api/spaces/space0000001" else { return false }
            XCTAssertNil(held, "duplicate selection must share the pending navigation")
            held = request; started.fulfill(); return true
        }
        let opening = Task { await model.select(channel: second) }
        await fulfillment(of: [started], timeout: 2)
        await model.select(channel: second)
        await model.select(channel: first)
        XCTAssertNil(model.openingChannelID)
        held?.respond(status: 200, data: try JSONEncoder().encode(detail))
        await opening.value
        XCTAssertEqual(model.selectedChannelID, first.id)
        XCTAssertEqual(model.chat.draft, "Keep this draft")
        XCTAssertEqual(model.chat.currentSnapshot()?.cursor, "41")
        XCTAssertFalse(paths.contains("/api/chat/channels/chan00000002/messages"))
    }

    @MainActor
    func testPreparedHistoryIsVisibleWhileSessionLoadsIncludingEmptyChannels() async throws {
        for sequences in [[], [5]] {
            let chat = ChatModel(api: client())
            let page = try JSONDecoder().decode(ChatHistory.self, from: chatHistory("Channel12345", sequences: sequences, cursor: 5, hasMore: !sequences.isEmpty))
            let history = ChatHistory(space: HistoryIdentity(id: "Space1234567", name: "Prepared space"),
                                      channel: HistoryIdentity(id: "Channel12345", name: "planning"),
                                      messages: page.messages, cursor: "5", hasMore: !sequences.isEmpty)
            let started = expectation(description: "chat session started")
            var held: MockURLProtocol?
            MockURLProtocol.deferred = { request, urlRequest in
                guard urlRequest.url?.path == "/api/chat/session" else { return false }
                held = request; started.fulfill(); return true
            }
            MockURLProtocol.handler = { request in
                XCTAssertTrue(request.url!.query?.contains("before=5") == true, "prepared history must not fetch the first page again")
                return (200, chatHistory("Channel12345", sequences: [2], cursor: 5, hasMore: false))
            }
            let opening = Task { await chat.open(history: history, displayName: "Me") }
            await fulfillment(of: [started], timeout: 2)
            XCTAssertFalse(chat.loading, "a pending sending capability is not a history load")
            XCTAssertEqual(chat.messages.map(\.seq), sequences.map(String.init))
            XCTAssertEqual(chat.spaceName, "Prepared space")
            XCTAssertEqual(chat.channelName, "planning")
            if !sequences.isEmpty {
                await chat.loadOlder()
                XCTAssertEqual(chat.messages.map(\.seq), ["2", "5"])
            }
            held?.respond(status: 200, data: Data(#"{"token":"chat-secret","author":{"id":"me","name":"Me","isGuest":false}}"#.utf8))
            await opening.value
            XCTAssertEqual(chat.messages.map(\.seq), sequences.isEmpty ? [] : ["2", "5"], "session completion must not overwrite history loaded in the meantime")
            XCTAssertFalse(chat.hasMore)
            await chat.stop()
        }
    }

    @MainActor
    func testSequencedReactionAdvancesDirectMessageReadCursorButHTTPSnapshotDoesNot() async throws {
        let channel = "dm0000000001"
        let messageID = "Message00000001"
        MockURLProtocol.handler = { request in
            switch request.url?.path {
            case "/api/chat/session":
                return (200, Data(#"{"token":"chat-secret","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case "/api/chat/channels/\(channel)/messages/\(messageID)/reactions":
                return (200, Data("""
                {"type":"message.reactions","schemaVersion":1,"channelId":"\(channel)","seq":"3","messageId":"\(messageID)","reactions":[{"emoji":"👍","authorIds":["self"]}]}
                """.utf8))
            default: throw URLError(.badURL)
            }
        }
        let message = ChatMessage(id: messageID, channelId: channel, seq: "1",
                                  author: ChatAuthor(id: "other", name: "Other", isGuest: false),
                                  content: ChatContent(version: 1, type: "text", text: "Hello"), createdAt: "now",
                                  clientMessageId: "client", reactions: [], reactionSeq: "1")
        let history = ChatHistory(space: HistoryIdentity(id: "", name: "Direct messages"),
                                  channel: HistoryIdentity(id: channel, name: "Other"), messages: [message], cursor: "1", hasMore: false)
        let chat = ChatModel(api: client())
        await chat.open(history: history, displayName: "Me")
        var readUpdates = 0
        chat.onReadCursor = { readUpdates += 1 }

        chat.receive(["type": "message.reactions", "schemaVersion": 1, "channelId": channel, "seq": "2",
                      "messageId": messageID, "reactions": [["emoji": "👍", "authorIds": ["other"]]]],
                     generation: 1, channelID: channel)
        XCTAssertEqual(chat.currentSnapshot()?.cursor, "2")
        XCTAssertEqual(readUpdates, 1, "a sequenced reaction in an open DM must update its read sequence")

        await chat.setReaction(messageID: messageID, emoji: "👍", active: true)
        XCTAssertEqual(chat.currentSnapshot()?.cursor, "2", "an HTTP reaction snapshot must not move the WebSocket replay cursor")
        XCTAssertEqual(readUpdates, 1)
        await chat.stop()
    }

    @MainActor
    func testPinnedReactionsStayCurrentOutsideHistoryAndAcrossDelayedPinSnapshots() async throws {
        let channel = "Channel12345"
        let messageID = "Message00000001"
        MockURLProtocol.handler = { request in
            guard request.url?.path == "/api/chat/session" else { throw URLError(.badURL) }
            return (200, Data(#"{"token":"chat-secret","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
        }
        for loaded in [false, true] {
            let author = ChatAuthor(id: "other", name: "Other", isGuest: false)
            var original = ChatMessage(id: messageID, channelId: channel, seq: "1", author: author,
                                       content: ChatContent(version: 1, type: "text", text: "Pinned"), createdAt: "now",
                                       clientMessageId: "client", reactions: [], reactionSeq: "1",
                                       pin: MessagePin(author: author, createdAt: "now"), pinSeq: "2")
            let chat = ChatModel(api: client())
            await chat.open(history: ChatHistory(space: HistoryIdentity(id: "Space1234567", name: "Space"),
                                                channel: HistoryIdentity(id: channel, name: "general"),
                                                messages: loaded ? [original] : [], pinnedMessages: [original],
                                                cursor: "10", hasMore: false), displayName: "Me")
            chat.receive(["type": "message.reactions", "schemaVersion": 1, "channelId": channel, "seq": "11",
                          "messageId": messageID, "reactions": [["emoji": "👍", "authorIds": ["alice", "bob"]]]],
                         generation: 1, channelID: channel)
            XCTAssertEqual(chat.pinnedMessages.first?.reactions, [MessageReaction(emoji: "👍", authorIds: ["alice", "bob"])])
            original.pinSeq = "12"
            chat.receive(["type": "message.pin", "schemaVersion": 1, "channelId": channel, "seq": "12",
                          "message": try JSONSerialization.jsonObject(with: JSONEncoder().encode(original))],
                         generation: 1, channelID: channel)
            XCTAssertEqual(chat.pinnedMessages.first?.reactionSeq, "11")
            chat.receive(["type": "message.reactions", "schemaVersion": 1, "channelId": channel, "seq": "13",
                          "messageId": messageID, "reactions": []], generation: 1, channelID: channel)
            original.pinSeq = "14"
            original.reactionSeq = "11"
            original.reactions = [MessageReaction(emoji: "👍", authorIds: ["alice", "bob"])]
            chat.receive(["type": "message.pin", "schemaVersion": 1, "channelId": channel, "seq": "14",
                          "message": try JSONSerialization.jsonObject(with: JSONEncoder().encode(original))],
                         generation: 1, channelID: channel)
            XCTAssertEqual(chat.pinnedMessages.first?.reactions, [])
            XCTAssertEqual(chat.currentSnapshot()?.pinnedMessages.first?.reactionSeq, "13")
            XCTAssertEqual(chat.messages.count, loaded ? 1 : 0, "pins must not create timeline rows")
            XCTAssertEqual(chat.currentSnapshot()?.cursor, "14")
            await chat.stop()
        }
    }

    @MainActor
    func testReactionIsOptimisticAndGatewaySnapshotsPreservePendingOwnIntent() async throws {
        let channel = "chan00000001"
        let messageID = "Message00000001"
        let requestStarted = expectation(description: "reaction request started")
        var held: MockURLProtocol?
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path.hasSuffix("/reactions") == true else { return false }
            held = request; requestStarted.fulfill(); return true
        }
        MockURLProtocol.handler = { request in
            guard request.url?.path == "/api/chat/session" else { throw URLError(.badURL) }
            return (200, Data(#"{"token":"chat-secret","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
        }
        let message = ChatMessage(id: messageID, channelId: channel, seq: "1",
                                  author: ChatAuthor(id: "other", name: "Other", isGuest: false),
                                  content: ChatContent(version: 1, type: "text", text: "Hello"), createdAt: "now",
                                  clientMessageId: "client", reactions: [], reactionSeq: "1")
        let history = ChatHistory(space: HistoryIdentity(id: "space0000001", name: "Space"),
                                  channel: HistoryIdentity(id: channel, name: "general"), messages: [message], cursor: "1", hasMore: false)
        let chat = ChatModel(api: client())
        await chat.open(history: history, displayName: "Me")

        let saving = Task { await chat.setReaction(messageID: messageID, emoji: "👍", active: true) }
        await fulfillment(of: [requestStarted], timeout: 2)
        XCTAssertEqual(chat.messages[0].reactions, [MessageReaction(emoji: "👍", authorIds: ["self"])], "own reaction must appear before acknowledgement")
        XCTAssertEqual(chat.currentSnapshot()?.messages[0].reactions, [], "cached history must not retain optimistic membership")

        chat.receive(["type": "message.reactions", "schemaVersion": 1, "channelId": channel, "seq": "2",
                      "messageId": messageID, "reactions": [["emoji": "🎉", "authorIds": ["other"]]]],
                     generation: 1, channelID: channel)
        XCTAssertEqual(chat.messages[0].reactions, [MessageReaction(emoji: "🎉", authorIds: ["other"]),
                                                    MessageReaction(emoji: "👍", authorIds: ["self"])])
        held?.respond(status: 200, data: Data("""
        {"type":"message.reactions","schemaVersion":1,"channelId":"\(channel)","seq":"3","messageId":"\(messageID)","reactions":[{"emoji":"🎉","authorIds":["other"]},{"emoji":"👍","authorIds":["self"]}]}
        """.utf8))
        await saving.value
        XCTAssertNil(chat.reactionErrors[messageID])
        await chat.stop()
    }

    @MainActor
    func testPinnedReactionsProjectAndRollbackWithoutInsertingUnloadedHistory() async throws {
        for loaded in [false, true] {
            let channel = "chan00000001", messageID = "Message00000001"
            let started = expectation(description: "pinned reaction started")
            var held: MockURLProtocol?
            MockURLProtocol.deferred = { request, urlRequest in
                guard urlRequest.url?.path.hasSuffix("/reactions") == true else { return false }
                held = request; started.fulfill(); return true
            }
            MockURLProtocol.handler = { request in
                guard request.url?.path == "/api/chat/session" else { throw URLError(.badURL) }
                return (200, Data(#"{"token":"chat-secret","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            }
            let author = ChatAuthor(id: "other", name: "Other", isGuest: false)
            var pinned = ChatMessage(id: messageID, channelId: channel, seq: "1", author: author,
                content: ChatContent(version: 1, type: "text", text: "Pinned"), createdAt: "now",
                clientMessageId: "client", reactions: [], reactionSeq: "1")
            pinned.pin = MessagePin(author: author, createdAt: "now"); pinned.pinSeq = "2"
            let history = ChatHistory(space: HistoryIdentity(id: "space0000001", name: "Space"),
                channel: HistoryIdentity(id: channel, name: "general"), messages: loaded ? [pinned] : [],
                pinnedMessages: [pinned], cursor: "2", hasMore: true)
            let chat = ChatModel(api: client())
            await chat.open(history: history, displayName: "Me")
            let saving = Task { await chat.setReaction(messageID: messageID, emoji: "👍", active: true) }
            await fulfillment(of: [started], timeout: 2)
            XCTAssertEqual(chat.pinnedMessages[0].reactions, [MessageReaction(emoji: "👍", authorIds: ["self"])])
            XCTAssertEqual(chat.currentSnapshot()?.pinnedMessages[0].reactions, [], "cached pins exclude pending intents")
            chat.receive(["type": "message.reactions", "schemaVersion": 1, "channelId": channel, "seq": "3",
                "messageId": messageID, "reactions": [["emoji": "🎉", "authorIds": ["other"]]]],
                generation: 1, channelID: channel)
            XCTAssertEqual(chat.pinnedMessages[0].reactions, [MessageReaction(emoji: "🎉", authorIds: ["other"]),
                MessageReaction(emoji: "👍", authorIds: ["self"])])
            held?.respond(status: 503)
            await saving.value
            XCTAssertEqual(chat.pinnedMessages[0].reactions, [MessageReaction(emoji: "🎉", authorIds: ["other"])])
            XCTAssertEqual(chat.messages.count, loaded ? 1 : 0)
            XCTAssertEqual(chat.currentSnapshot()?.cursor, "3")
            XCTAssertEqual(chat.pinnedMessages[0].pinSeq, "2")
            await chat.stop()
        }
    }

    @MainActor
    func testReactionFailureRollsBackAndRapidSupersedingIntentWins() async throws {
        let channel = "chan00000001"
        let messageID = "Message00000001"
        var held: [MockURLProtocol] = []
        MockURLProtocol.deferred = { request, urlRequest in
            guard urlRequest.url?.path.hasSuffix("/reactions") == true else { return false }
            held.append(request); return true
        }
        MockURLProtocol.handler = { request in
            guard request.url?.path == "/api/chat/session" else { throw URLError(.badURL) }
            return (200, Data(#"{"token":"chat-secret","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
        }
        let message = ChatMessage(id: messageID, channelId: channel, seq: "1",
                                  author: ChatAuthor(id: "other", name: "Other", isGuest: false),
                                  content: ChatContent(version: 1, type: "text", text: "Hello"), createdAt: "now",
                                  clientMessageId: "client", reactions: [], reactionSeq: "1")
        let history = ChatHistory(space: HistoryIdentity(id: "space0000001", name: "Space"),
                                  channel: HistoryIdentity(id: channel, name: "general"), messages: [message], cursor: "1", hasMore: false)
        let chat = ChatModel(api: client())
        await chat.open(history: history, displayName: "Me")

        let add = Task { await chat.setReaction(messageID: messageID, emoji: "👍", active: true) }
        await waitUntil { held.count == 1 }
        let remove = Task { await chat.setReaction(messageID: messageID, emoji: "👍", active: false) }
        await waitUntil { chat.messages[0].reactions?.isEmpty == true }
        XCTAssertTrue(chat.messages[0].reactions?.isEmpty == true, "latest remove must be visible while add is in flight")
        held[0].respond(status: 500)
        await waitUntil { held.count == 2 }
        XCTAssertNil(chat.reactionErrors[messageID], "superseded failure must not surface")
        chat.receive(["type": "message.reactions", "schemaVersion": 1, "channelId": channel, "seq": "2",
                      "messageId": messageID, "reactions": [["emoji": "👍", "authorIds": ["other", "self"]]]],
                     generation: 1, channelID: channel)
        XCTAssertEqual(chat.messages[0].reactions, [MessageReaction(emoji: "👍", authorIds: ["other"])])
        held[1].respond(status: 500)
        await add.value; await remove.value
        XCTAssertEqual(chat.messages[0].reactions, [MessageReaction(emoji: "👍", authorIds: ["other", "self"])], "failed removal restores the latest server state")
        XCTAssertEqual(chat.reactionErrors[messageID], "Couldn’t save reaction. Retry.")

        let retry = Task { await chat.retryReaction(messageID: messageID) }
        await waitUntil { held.count == 3 }
        XCTAssertNil(chat.reactionErrors[messageID], "a new attempt clears the previous error immediately")
        XCTAssertEqual(chat.messages[0].reactions, [MessageReaction(emoji: "👍", authorIds: ["other"])])
        held[2].respond(status: 200, data: Data("""
        {"type":"message.reactions","schemaVersion":1,"channelId":"\(channel)","seq":"3","messageId":"\(messageID)","reactions":[{"emoji":"👍","authorIds":["other"]}]}
        """.utf8))
        await retry.value
        await chat.stop()
    }

    @MainActor
    func testJoinedConversationBecomesReadOnlyPreviewWithoutRetainingSession() async throws {
        let channel = "chan00000001"
        var sessionRequests = 0
        var reactionRequests = 0
        MockURLProtocol.handler = { request in
            if request.url?.path == "/api/chat/session" {
                sessionRequests += 1
                return (200, Data(#"{"token":"chat-secret","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            }
            if request.url?.path.hasSuffix("/reactions") == true { reactionRequests += 1 }
            throw URLError(.badURL)
        }
        let message = ChatMessage(id: "Message00000001", channelId: channel, seq: "1",
                                  author: ChatAuthor(id: "other", name: "Other", isGuest: false),
                                  content: ChatContent(version: 1, type: "text", text: "Hello"), createdAt: "now",
                                  clientMessageId: "client", reactions: [MessageReaction(emoji: "👍", authorIds: ["other"])], reactionSeq: "1")
        let history = ChatHistory(space: HistoryIdentity(id: "space0000001", name: "Space"),
                                  channel: HistoryIdentity(id: channel, name: "general"), messages: [message], cursor: "1", hasMore: false)
        let chat = ChatModel(api: client())
        await chat.open(history: history, displayName: "Me")
        XCTAssertNotNil(chat.currentAuthor)

        await chat.preview(history: history)
        XCTAssertTrue(chat.isPreview)
        XCTAssertNil(chat.currentAuthor)
        XCTAssertEqual(chat.messages.first?.reactions?.first?.emoji, "👍")
        await chat.setReaction(messageID: message.id, emoji: "👍", active: false)
        XCTAssertEqual(sessionRequests, 1, "preview must not create or retain a chat session")
        XCTAssertEqual(reactionRequests, 0, "preview reaction attempts must not write")
        await chat.stop()
    }

    @MainActor
    func testReactionDrivenPreviewResyncReadsHistoryWithoutSessionOrWrite() async throws {
        let channel = "chan00000001"
        var paths: [String] = []
        let refreshed = expectation(description: "preview history refreshed")
        MockURLProtocol.handler = { request in
            paths.append(request.url!.path)
            guard request.url?.path == "/api/chat/channels/\(channel)/messages" else { throw URLError(.badURL) }
            refreshed.fulfill()
            return (200, Data("""
            {"space":{"id":"space0000001","name":"Space"},"channel":{"id":"\(channel)","name":"general"},"messages":[{"id":"Message00000001","channelId":"\(channel)","seq":"1","author":{"id":"other","name":"Other","isGuest":false},"content":{"version":1,"type":"text","text":"Refreshed"},"createdAt":"now","clientMessageId":"client","reactions":[{"emoji":"🎉","authorIds":["other"]}],"reactionSeq":"2"}],"cursor":"2","hasMore":false}
            """.utf8))
        }
        let initial = ChatHistory(space: HistoryIdentity(id: "space0000001", name: "Space"),
                                  channel: HistoryIdentity(id: channel, name: "general"), messages: [], cursor: "1", hasMore: false)
        let chat = ChatModel(api: client())
        await chat.preview(history: initial)
        chat.receive(["type": "message.reactions", "channelId": channel, "seq": "3"], generation: 1, channelID: channel)
        await fulfillment(of: [refreshed], timeout: 2)
        await waitUntil { !chat.messages.isEmpty }
        XCTAssertTrue(chat.isPreview)
        XCTAssertNil(chat.currentAuthor)
        XCTAssertEqual(chat.messages.first?.content.text, "Refreshed")
        XCTAssertEqual(chat.messages.first?.reactions?.first?.emoji, "🎉")
        await chat.setReaction(messageID: "Message00000001", emoji: "🎉", active: false)
        XCTAssertFalse(paths.contains("/api/chat/session"))
        XCTAssertFalse(paths.contains { $0.hasSuffix("/reactions") })
        await chat.stop()
    }

    func testDirectMessageContractsAndReadRequest() async throws {
        let api = client()
        var requests: [(String, String, Data?)] = []
        MockURLProtocol.handler = { request in
            requests.append((request.httpMethod ?? "", request.url!.path, try requestBodyData(request)))
            switch (request.httpMethod, request.url!.path) {
            case ("GET", "/api/dms"):
                return (200, Data(#"{"conversations":[{"id":"dm0000000001","peer":{"id":"peer","username":"exact_name","displayName":"Exact Name"},"lastSeq":"12","readSeq":"9"}]}"#.utf8))
            case ("POST", "/api/dms"):
                return (200, Data(#"{"id":"dm0000000001","peer":{"id":"peer","username":"exact_name","displayName":"Exact Name"},"lastSeq":"12","readSeq":"9"}"#.utf8))
            case ("POST", "/api/dms/dm0000000001/read"): return (204, Data())
            default: throw URLError(.badURL)
            }
        }
        let listed = try await api.directMessages()
        XCTAssertTrue(listed[0].unread)
        let created = try await api.createDirectMessage(username: "exact_name")
        XCTAssertEqual(created.peer.displayName, "Exact Name")
        try await api.markDirectMessageRead(id: created.id, seq: "12")
        XCTAssertEqual(requests.map { "\($0.0) \($0.1)" }, ["GET /api/dms", "POST /api/dms", "POST /api/dms/dm0000000001/read"])
        XCTAssertEqual(try JSONSerialization.jsonObject(with: XCTUnwrap(requests[1].2)) as? [String: String], ["username": "exact_name"])
        XCTAssertEqual(try JSONSerialization.jsonObject(with: XCTUnwrap(requests[2].2)) as? [String: String], ["seq": "12"])
    }

    @MainActor
    func testDirectMessageRefreshClearsRecoveredDNSErrorWithoutClearingOtherErrors() async {
        let model = AppModel(api: client())
        model.account = Account(id: "me", username: "me", displayName: "Me")
        let previous = DirectMessageConversation(id: "dm0000000001", peer: DirectMessagePeer(id: "peer", username: "peer", displayName: "Peer"), lastSeq: "1", readSeq: "0")
        model.directMessages = [previous]
        model.error = "Your profile could not be saved."
        var failure = true
        var empty = false
        MockURLProtocol.handler = { request in
            XCTAssertEqual(request.url?.path, "/api/dms")
            if failure { throw URLError(.cannotFindHost) }
            if empty { return (200, Data(#"{"conversations":[]}"#.utf8)) }
            return (200, Data(#"{"conversations":[{"id":"dm0000000001","peer":{"id":"peer","username":"peer","displayName":"Peer"},"lastSeq":"12","readSeq":"9"}]}"#.utf8))
        }

        await model.refreshDirectMessages()
        XCTAssertEqual(model.directMessages, [previous], "a failed refresh keeps the last list")
        XCTAssertEqual(model.directMessagesError, "Couldn’t reach Caper. Check your connection.", "network failures read as web's sentence")
        XCTAssertEqual(model.error, "Your profile could not be saved.", "DM failures must not replace operation errors")

        failure = false
        await model.refreshDirectMessages()
        XCTAssertNil(model.directMessagesError, "network recovery clears the stale DM error")
        XCTAssertEqual(model.directMessages.map(\.id), ["dm0000000001"])
        XCTAssertEqual(model.directMessages.first?.lastSeq, "12")
        XCTAssertEqual(model.directMessages.first?.readSeq, "9")
        XCTAssertEqual(model.error, "Your profile could not be saved.", "background success must not hide another failure")

        failure = true
        await model.refreshDirectMessages()
        XCTAssertNotNil(model.directMessagesError)
        failure = false; empty = true
        await model.refreshDirectMessages()
        XCTAssertTrue(model.directMessages.isEmpty)
        XCTAssertNil(model.directMessagesError, "an empty successful list also clears the error")
        XCTAssertEqual(model.error, "Your profile could not be saved.")
    }

    /// Web's `requestsOpen ?? viewingRequest`: the section follows the open
    /// request until the person toggles it, and their choice wins.
    @MainActor
    func testMessageRequestsFollowTheOpenRequestUntilToggled() {
        let model = AppModel(api: client())
        let request = DirectMessageConversation(id: "dm0000000003", peer: DirectMessagePeer(id: "peer", username: "jordan", displayName: "Jordan"),
                                                lastSeq: "1", readSeq: "0", status: .incoming)
        let accepted = DirectMessageConversation(id: "dm0000000001", peer: DirectMessagePeer(id: "maya", username: "maya", displayName: "Maya"),
                                                 lastSeq: "1", readSeq: "0")
        model.directMessages = [request, accepted]
        XCTAssertFalse(model.showingMessageRequests, "closed by default")
        model.selectedDirectMessageID = accepted.id
        XCTAssertFalse(model.showingMessageRequests, "an accepted conversation leaves it closed")
        model.selectedDirectMessageID = request.id
        XCTAssertTrue(model.showingMessageRequests, "viewing a request opens it")
        model.showingMessageRequests.toggle()
        XCTAssertFalse(model.showingMessageRequests, "the toggle closes it while the request stays open")
        XCTAssertEqual(model.selectedDirectMessageID, request.id)
        model.selectedDirectMessageID = nil
        model.showingMessageRequests.toggle()
        XCTAssertTrue(model.showingMessageRequests, "an explicit choice wins over the view")
    }

    @MainActor
    func testLogoutClearsDMRefreshErrorAndFencesLateRefreshResults() async {
        for status in [200, 503] {
            let model = AppModel(api: client())
            model.account = Account(id: "me", username: "me", displayName: "Me")
            MockURLProtocol.handler = { request in
                if request.url?.path == "/api/auth/logout" { return (204, Data()) }
                XCTAssertEqual(request.url?.path, "/api/dms")
                throw URLError(.cannotFindHost)
            }
            await model.refreshDirectMessages()
            XCTAssertNotNil(model.directMessagesError)
            XCTAssertNil(model.error, "a background refresh uses its own error state")

            let refreshStarted = expectation(description: "DM refresh started before logout (\(status))")
            var delayedRefresh: MockURLProtocol?
            MockURLProtocol.deferred = { request, urlRequest in
                guard urlRequest.url?.path == "/api/dms" else { return false }
                delayedRefresh = request
                refreshStarted.fulfill()
                return true
            }
            let refresh = Task { await model.refreshDirectMessages() }
            await fulfillment(of: [refreshStarted], timeout: 1)
            await model.logout()
            XCTAssertNil(model.directMessagesError, "logout clears the previous account's error immediately")
            let data = status == 200
                ? Data(#"{"conversations":[{"id":"dm0000000001","peer":{"id":"peer","username":"peer","displayName":"Peer"},"lastSeq":"12","readSeq":"9"}]}"#.utf8)
                : Data(#"{"error":"Late DM refresh failure."}"#.utf8)
            delayedRefresh?.respond(status: status, data: data)
            await refresh.value
            MockURLProtocol.deferred = nil
            XCTAssertNil(model.account)
            XCTAssertTrue(model.directMessages.isEmpty, "an old request cannot restore signed-out data")
            XCTAssertNil(model.directMessagesError, "an old failure cannot restore the banner")
            XCTAssertNil(model.error)
        }
    }

    func testPeopleGETUsesAccountAuthorizationAndDecodesNullAvatar() async throws {
        var routes: [String] = []
        MockURLProtocol.handler = { request in
            routes.append("\(request.httpMethod ?? "") \(request.url?.path ?? "")")
            XCTAssertEqual(request.value(forHTTPHeaderField: "authorization"), "Bearer account-secret")
            return (200, Data(#"{"people":[{"id":"alex00000001","username":"alex","displayName":"Alex","avatarId":799},{"id":"sam000000001","username":"sam","displayName":"Sam","avatarId":null}]}"#.utf8))
        }
        let people = try await client().people()
        XCTAssertEqual(routes, ["GET /api/people"])
        XCTAssertEqual(people.map(\.username), ["alex", "sam"])
        XCTAssertEqual(people.map(\.avatarId), [799, nil])
    }

    @MainActor
    func testPeopleRefreshKeepsLastListOnFailureAndClearsOnLogout() async throws {
        let model = AppModel(api: client())
        model.account = Account(id: "me0000000001", username: "me", displayName: "Me")
        var fail = false
        MockURLProtocol.handler = { request in
            switch request.url!.path {
            case "/api/people":
                if fail { return (500, Data(#"{"error":"unavailable"}"#.utf8)) }
                return (200, Data(#"{"people":[{"id":"alex00000001","username":"alex","displayName":"Alex","avatarId":null}]}"#.utf8))
            case "/api/auth/logout": return (204, Data())
            default: throw URLError(.badURL)
            }
        }
        XCTAssertNil(model.people, "DM suggestions fall back to the peer until people load")
        await model.refreshPeople()
        XCTAssertEqual(model.people?.map(\.username), ["alex"])
        fail = true
        await model.refreshPeople()
        XCTAssertEqual(model.people?.map(\.username), ["alex"], "a failed refresh keeps the previous list")
        XCTAssertNil(model.error, "people failures are silent")
        await model.logout()
        XCTAssertNil(model.people)
    }

    func testMessageRequestBlockAndPrivacyEndpoints() async throws {
        let api = client()
        var requests: [(String, String, Data?)] = []
        MockURLProtocol.handler = { request in
            requests.append((request.httpMethod ?? "", request.url!.path, try requestBodyData(request)))
            XCTAssertEqual(request.value(forHTTPHeaderField: "authorization"), "Bearer account-secret")
            switch (request.httpMethod, request.url!.path) {
            case ("POST", "/api/dms/dm0000000003/accept"):
                return (200, Data(#"{"id":"dm0000000003","peer":{"id":"stranger0001","username":"jordan","displayName":"Jordan","avatarId":412},"lastSeq":"1","readSeq":"0","status":"accepted","blocked":false}"#.utf8))
            case ("POST", "/api/dms/dm0000000003/decline"), ("PUT", "/api/blocks/member000001"), ("DELETE", "/api/blocks/member000001"):
                return (204, Data())
            case ("GET", "/api/blocks"):
                return (200, Data(#"{"blocks":[{"id":"member000001","username":"maya","displayName":"Maya","avatarId":31}]}"#.utf8))
            case ("GET", "/api/account/privacy"), ("PUT", "/api/account/privacy"):
                return (200, Data(#"{"directMessages":"spaces"}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let accepted = try await api.acceptDirectMessage(id: "dm0000000003")
        XCTAssertEqual(accepted.status, .accepted)
        XCTAssertEqual(accepted.peer.avatarId, 412)
        try await api.declineDirectMessage(id: "dm0000000003")
        try await api.block(accountID: "member000001")
        let blocked = try await api.blocks()
        XCTAssertEqual(blocked.map(\.username), ["maya"])
        try await api.unblock(accountID: "member000001")
        let privacy = try await api.privacy()
        XCTAssertEqual(privacy.directMessages, .spaces)
        let saved = try await api.updatePrivacy(.spaces)
        XCTAssertEqual(saved.directMessages, .spaces)
        XCTAssertEqual(requests.map { "\($0.0) \($0.1)" }, [
            "POST /api/dms/dm0000000003/accept", "POST /api/dms/dm0000000003/decline", "PUT /api/blocks/member000001",
            "GET /api/blocks", "DELETE /api/blocks/member000001", "GET /api/account/privacy", "PUT /api/account/privacy",
        ])
        XCTAssertEqual(try JSONSerialization.jsonObject(with: XCTUnwrap(requests[6].2)) as? [String: String], ["directMessages": "spaces"])
        do {
            try await api.block(accountID: "not-an-id")
            XCTFail("Expected local account ID rejection")
        } catch let error as APIError { XCTAssertEqual(error.status, 400) }
    }

    func testDirectMessageRefusalsUseClientWording() async throws {
        let api = client()
        var code = "dm_not_accepted"
        MockURLProtocol.handler = { _ in (403, Data(#"{"error":"server wording","code":"\#(code)"}"#.utf8)) }
        do {
            _ = try await api.createDirectMessage(username: "jordan")
            XCTFail("Expected a refusal")
        } catch let error as APIError {
            XCTAssertEqual(error.code, "dm_not_accepted")
            XCTAssertEqual(error.localizedDescription, "This person isn't accepting direct messages.")
        }
        code = "dm_blocked"
        do {
            _ = try await api.createDirectMessage(username: "jordan")
            XCTFail("Expected a refusal")
        } catch let error as APIError {
            XCTAssertEqual(error.localizedDescription, "You blocked this person. Unblock them to message them.")
        }
        MockURLProtocol.handler = { _ in (403, Data(#"{"error":"Forbidden."}"#.utf8)) }
        do {
            _ = try await api.createDirectMessage(username: "jordan")
            XCTFail("Expected a refusal")
        } catch let error as APIError { XCTAssertEqual(error.localizedDescription, "Forbidden.", "other errors keep the server's text") }
    }

    @MainActor
    func testRequestsDeclineAndBlocksUpdateListsAndTimelines() async throws {
        let model = AppModel(api: client())
        model.account = Account(id: "owner0000001", username: "owner", displayName: "Owner")
        let alex = DirectMessagePeer(id: "member000002", username: "alex", displayName: "Alex")
        let jordan = DirectMessagePeer(id: "stranger0001", username: "jordan", displayName: "Jordan", avatarId: 412)
        model.directMessages = [
            DirectMessageConversation(id: "dm0000000001", peer: alex, lastSeq: "2", readSeq: "1"),
            DirectMessageConversation(id: "dm0000000003", peer: jordan, lastSeq: "1", readSeq: "0", status: .incoming),
        ]
        var blocks: [String] = []
        MockURLProtocol.handler = { request in
            switch (request.httpMethod, request.url!.path) {
            case ("POST", "/api/dms/dm0000000003/decline"): return (204, Data())
            case ("PUT", "/api/blocks/member000002"): blocks = ["member000002"]; return (204, Data())
            case ("DELETE", "/api/blocks/member000002"): blocks = []; return (204, Data())
            case ("GET", "/api/blocks"):
                let rows = blocks.map { #"{"id":"\#($0)","username":"alex","displayName":"Alex"}"# }.joined(separator: ",")
                return (200, Data(#"{"blocks":[\#(rows)]}"#.utf8))
            case ("POST", "/api/auth/logout"): return (204, Data())
            default: throw URLError(.badURL)
            }
        }
        XCTAssertEqual(model.visibleDirectMessages.map(\.id), ["dm0000000001"])
        XCTAssertEqual(model.messageRequests.map(\.id), ["dm0000000003"])
        try await model.declineRequest(model.messageRequests[0])
        XCTAssertTrue(model.messageRequests.isEmpty)

        try await model.block(BlockTarget(peer: alex))
        XCTAssertEqual(model.blockedIDs, ["member000002"])
        XCTAssertEqual(model.chat.blockedAuthorIDs, ["member000002"], "timelines collapse their messages at once")
        XCTAssertEqual(model.directMessages.first?.blocked, true)
        try await model.unblock(accountID: "member000002")
        XCTAssertTrue(model.blockedIDs.isEmpty)
        XCTAssertTrue(model.chat.blockedAuthorIDs.isEmpty)
        XCTAssertEqual(model.directMessages.first?.blocked, false)
        do {
            try await model.block(BlockTarget(id: "owner0000001", username: "owner", displayName: "Owner"))
            XCTFail("You can't block yourself")
        } catch {}
        await model.logout()
        XCTAssertTrue(model.blockedAccounts.isEmpty)
        XCTAssertNil(model.directMessagePrivacy)
    }

    @MainActor
    func testSelfNotesCreateWithOwnUsernameAndReuseWithoutSpace() async throws {
        let model = AppModel(api: client())
        model.account = Account(id: "self00000001", username: "notes_owner", displayName: "Notes Owner")
        model.directMessages = [DirectMessageConversation(id: "dm0000000001", peer: DirectMessagePeer(id: "other", username: "other", displayName: "Other"), lastSeq: "0", readSeq: "0")]
        var creates = 0
        MockURLProtocol.handler = { request in
            switch (request.httpMethod, request.url!.path) {
            case ("POST", "/api/dms"):
                creates += 1
                let body = try JSONSerialization.jsonObject(with: XCTUnwrap(requestBodyData(request))) as? [String: String]
                XCTAssertEqual(body, ["username": "notes_owner"])
                return (200, Data(#"{"id":"dm0000000002","peer":{"id":"self00000001","username":"notes_owner","displayName":"Notes Owner"},"lastSeq":"0","readSeq":"0"}"#.utf8))
            case ("GET", "/api/chat/channels/dm0000000002/messages"):
                return (200, Data(#"{"space":{"id":"","name":"Direct messages"},"channel":{"id":"dm0000000002","name":"Notes Owner","direct":true},"messages":[],"cursor":"0","hasMore":false}"#.utf8))
            case ("POST", "/api/chat/session"):
                return (200, Data(#"{"token":"fixture-chat","author":{"id":"self00000001","name":"Notes Owner","isGuest":false}}"#.utf8))
            case ("POST", "/api/dms/dm0000000002/read"), ("POST", "/api/auth/logout"): return (204, Data())
            default: throw URLError(.badURL)
            }
        }
        await model.openSelfDirectMessage()
        XCTAssertEqual(creates, 1)
        XCTAssertFalse(model.busy)
        XCTAssertEqual(model.selectedDirectMessageID, "dm0000000002")
        XCTAssertEqual(model.chat.channelName, "Notes Owner")
        await model.openSelfDirectMessage()
        XCTAssertEqual(creates, 1, "reopening must not create another notes conversation")
        XCTAssertEqual(model.directMessages.count, 2, "the peer conversation stays alongside notes")
        XCTAssertNil(model.selectedSpaceID)
        await model.logout()
    }

    @MainActor
    func testDirectMessageNavigationWorksWithoutSpaceAndClearsOnLogout() async throws {
        let model = AppModel(api: client())
        model.account = Account(id: "me", username: "me", displayName: "Me")
        let dm = DirectMessageConversation(id: "dm0000000001", peer: DirectMessagePeer(id: "peer", username: "peer", displayName: "Peer"), lastSeq: "1", readSeq: "0")
        MockURLProtocol.handler = { request in
            switch request.url!.path {
            case "/api/chat/channels/dm0000000001/messages":
                return (200, Data(#"{"space":{"id":"","name":"Direct messages"},"channel":{"id":"dm0000000001","name":"Peer","direct":true},"messages":[],"cursor":"1","hasMore":false}"#.utf8))
            case "/api/chat/session": return (200, Data(#"{"token":"chat","author":{"id":"me","name":"Me","isGuest":false}}"#.utf8))
            case "/api/dms/dm0000000001/read": return (204, Data())
            case "/api/dms": return (200, Data(#"{"conversations":[{"id":"dm0000000001","peer":{"id":"peer","username":"peer","displayName":"Peer"},"lastSeq":"1","readSeq":"1"}]}"#.utf8))
            case "/api/auth/logout": return (204, Data())
            default: throw URLError(.badURL)
            }
        }
        await model.select(directMessage: dm)
        XCTAssertNil(model.selectedSpaceID)
        XCTAssertEqual(model.selectedDirectMessageID, dm.id)
        XCTAssertEqual(model.chat.spaceName, "Direct messages")
        XCTAssertEqual(model.chat.channelName, "Peer")
        await model.logout()
        XCTAssertTrue(model.directMessages.isEmpty)
        XCTAssertNil(model.selectedDirectMessageID)
    }
}
