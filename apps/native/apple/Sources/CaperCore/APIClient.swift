import Foundation

public struct APIError: LocalizedError, Equatable {
    public let status: Int
    public let message: String
    public let code: String?
    public let attemptsRemaining: Int?
    public var errorDescription: String? { message }

    public init(status: Int, message: String, code: String? = nil, attemptsRemaining: Int? = nil) {
        self.status = status; self.message = message; self.code = code; self.attemptsRemaining = attemptsRemaining
    }

    var endsVoiceAccess: Bool {
        code != "ice_restart_retry" && code != "track_gone" && [401, 403, 404].contains(status)
    }

    var retryableVoiceControl: Bool {
        if code == "ice_restart_invalid" { return false }
        if code == "ice_restart_retry" { return true }
        return status == 408 || status == 429 || status >= 500
            || (status == 409 && code == "ice_restart_pending")
    }
}

private struct ErrorBody: Decodable { let error: String?; let attemptsRemaining: Int?; let code: String? }
private struct Challenge: Decodable { let challengeId: String }
private struct Verification: Decodable { let account: Account; let token: String }
private struct SessionInput: Encodable { let name: String }
private struct EmailInput: Encodable { let email: String }
private struct VerifyInput: Encodable { let challengeId: String; let code: String; let tokenTransport = "bearer" }
private struct ProfileInput: Encodable { let username: String; let displayName: String }
private struct SendInput: Encodable { let clientMessageId: String; let text: String; let threadRootId: String?; let broadcast: Bool? }
private struct ReactionInput: Encodable { let emoji: String; let active: Bool }
private struct PinInput: Encodable { let active: Bool }
private struct SpaceInput: Encodable { let name: String }
private struct ChannelInput: Encodable {
    let name: String
    let privateChannel: Bool
    enum CodingKeys: String, CodingKey { case name; case privateChannel = "private" }
}
private struct UsernameInput: Encodable { let username: String }
private struct ReadInput: Encodable { let seq: String }
private struct PushDeviceInput: Encodable { let platform: String; let token: String }
public struct MembersResponse: Decodable, Sendable {
    public let members: [Member]
    public let invitations: [Member]
    private enum CodingKeys: String, CodingKey { case members, invitations }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        members = try values.decode([Member].self, forKey: .members)
        invitations = try values.decodeIfPresent([Member].self, forKey: .invitations) ?? []
    }
}

public actor APIClient {
    public let baseURL: URL
    private let session: URLSession
    private let tokenStore: TokenStore
    private var token: String?
    public var isSignedIn: Bool { token != nil }
    private var authGeneration: UInt64 = 0
    private let decoder = JSONDecoder()
    private let encoder = JSONEncoder()

    public init(baseURL: URL = URL(string: "https://caper.chat")!, session: URLSession? = nil, tokenStore: TokenStore? = nil) {
        self.baseURL = baseURL
        self.session = session ?? SecureSession.make()
        let resolvedStore = tokenStore ?? KeychainTokenStore(service: Self.sessionService(for: baseURL))
        self.tokenStore = resolvedStore
        token = try? resolvedStore.load()
    }

    public nonisolated static func sessionService(for baseURL: URL) -> String {
        let scheme = baseURL.scheme?.lowercased() ?? "invalid"
        let host = baseURL.host?.lowercased() ?? "invalid"
        let port = baseURL.port ?? (scheme == "https" ? 443 : scheme == "http" ? 80 : 0)
        if scheme == "https", host == "caper.chat", port == 443 { return "chat.caper.session" }
        return "chat.caper.session.\(scheme).\(host).\(port)"
    }

    public func authorizationToken() -> String? { token }

    public func account() async throws -> Account? {
        do { return try await request("api/account/me") }
        catch let error as APIError where error.status == 401 { return nil }
    }

    public func requestCode(email: String) async throws -> String {
        let result: Challenge = try await request("api/auth/email/request", method: "POST", body: EmailInput(email: email))
        return result.challengeId
    }

    public func verify(challengeId: String, code: String) async throws -> Account {
        authGeneration &+= 1
        let generation = authGeneration
        let result: Verification = try await request("api/auth/email/verify", method: "POST", body: VerifyInput(challengeId: challengeId, code: code))
        guard generation == authGeneration else { throw CancellationError() }
        try tokenStore.save(result.token)
        token = result.token
        return result.account
    }

    public func updateProfile(username: String, displayName: String) async throws -> Account {
        try await request("api/account/profile", method: "POST", body: ProfileInput(username: username, displayName: displayName))
    }

    public func logout() async throws {
        authGeneration &+= 1
        let revokedToken = token
        token = nil
        try tokenStore.clear()
        guard let revokedToken else { return }
        let _: Empty = try await request("api/auth/logout", method: "POST", extraHeaders: ["authorization": "Bearer \(revokedToken)"])
    }

    public func spaces() async throws -> SpacesResponse { try await request("api/spaces") }
    public func directMessages() async throws -> [DirectMessageConversation] {
        let response: DirectMessagesResponse = try await request("api/dms")
        guard response.conversations.allSatisfy({
            (try? Sequence.compare($0.lastSeq, "0")) != nil && (try? Sequence.compare($0.readSeq, "0")) != nil
                && (try? Sequence.compare($0.readSeq, $0.lastSeq)) != .orderedDescending
        }) else { throw APIError(status: 502, message: "Caper returned invalid direct messages.") }
        return response.conversations
    }
    public func createDirectMessage(username: String) async throws -> DirectMessageConversation {
        try await request("api/dms", method: "POST", body: UsernameInput(username: username))
    }
    public func markDirectMessageRead(id: String, seq: String) async throws {
        let _: Empty = try await request("api/dms/\(try pathID(id))/read", method: "POST", body: ReadInput(seq: seq))
    }
    public func pushConfiguration() async throws -> PushConfiguration { try await request("api/push/config") }
    public func registerPushDevice(platform: String, token: String) async throws {
        let _: Empty = try await request("api/push/devices", method: "POST", body: PushDeviceInput(platform: platform, token: token))
    }
    public func unregisterPushDevice(platform: String, token: String) async throws {
        let _: Empty = try await request("api/push/devices", method: "DELETE", body: PushDeviceInput(platform: platform, token: token))
    }
    public func space(_ id: String) async throws -> SpaceDetail { try await request("api/spaces/\(try pathID(id))") }

    public func createSpace(name: String) async throws -> Space {
        try await request("api/spaces", method: "POST", body: SpaceInput(name: name.trimmingCharacters(in: .whitespacesAndNewlines)))
    }

    public func updateSpace(id: String, name: String) async throws -> Space {
        try await request("api/spaces/\(try pathID(id))", method: "PATCH", body: SpaceInput(name: name.trimmingCharacters(in: .whitespacesAndNewlines)))
    }

    public func deleteSpace(id: String) async throws {
        let _: Empty = try await request("api/spaces/\(try pathID(id))", method: "DELETE")
    }

    public func createChannel(spaceID: String, name: String, privateChannel: Bool) async throws -> Channel {
        try await request("api/spaces/\(try pathID(spaceID))/channels", method: "POST", body: ChannelInput(name: name, privateChannel: privateChannel))
    }

    public func updateChannel(spaceID: String, channelID: String, name: String, privateChannel: Bool) async throws -> Channel {
        try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))", method: "PATCH", body: ChannelInput(name: name, privateChannel: privateChannel))
    }

    public func deleteChannel(spaceID: String, channelID: String) async throws {
        let _: Empty = try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))", method: "DELETE")
    }

    public func spaceMembers(spaceID: String) async throws -> [Member] {
        let response: MembersResponse = try await request("api/spaces/\(try pathID(spaceID))/members")
        return response.members
    }

    public func addSpaceMember(spaceID: String, username: String) async throws -> Member {
        try await request("api/spaces/\(try pathID(spaceID))/members", method: "POST", body: UsernameInput(username: username))
    }

    public func spaceInvitations(spaceID: String) async throws -> [Member] {
        let response: MembersResponse = try await request("api/spaces/\(try pathID(spaceID))/invitations")
        return response.members
    }

    public func cancelSpaceInvitation(spaceID: String, userID: String) async throws {
        let _: Empty = try await request("api/spaces/\(try pathID(spaceID))/invitations/\(try pathID(userID))", method: "DELETE")
    }

    public func acceptSpaceInvitation(spaceID: String) async throws -> Space {
        try await request("api/spaces/\(try pathID(spaceID))/invitation", method: "POST")
    }

    public func declineSpaceInvitation(spaceID: String) async throws {
        let _: Empty = try await request("api/spaces/\(try pathID(spaceID))/invitation", method: "DELETE")
    }

    public func removeSpaceMember(spaceID: String, memberID: String) async throws {
        let _: Empty = try await request("api/spaces/\(try pathID(spaceID))/members/\(try pathID(memberID))", method: "DELETE")
    }

    public func channelMembers(spaceID: String, channelID: String) async throws -> [Member] {
        let response: MembersResponse = try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))/members")
        return response.members
    }

    public func channelInvitations(spaceID: String, channelID: String) async throws -> [Member] {
        let response: MembersResponse = try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))/members")
        return response.invitations
    }

    public func addChannelMember(spaceID: String, channelID: String, username: String) async throws -> Member {
        try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))/members", method: "POST", body: UsernameInput(username: username))
    }

    public func removeChannelMember(spaceID: String, channelID: String, memberID: String) async throws {
        let _: Empty = try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))/members/\(try pathID(memberID))", method: "DELETE")
    }

    public func joinChannel(spaceID: String, channelID: String) async throws -> Channel {
        try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))/membership", method: "POST")
    }

    public func leaveChannel(spaceID: String, channelID: String) async throws {
        let _: Empty = try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))/membership", method: "DELETE")
    }

    public func acceptChannelInvitation(spaceID: String, channelID: String) async throws -> Channel {
        try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))/invitation", method: "POST")
    }

    public func declineChannelInvitation(spaceID: String, channelID: String) async throws {
        let _: Empty = try await request("api/spaces/\(try pathID(spaceID))/channels/\(try pathID(channelID))/invitation", method: "DELETE")
    }

    public func history(channelID: String? = nil, before: String? = nil) async throws -> ChatHistory {
        var path = channelID.map { "api/chat/channels/\($0)/messages" } ?? "api/chat/general"
        if let before { path += "?before=\(before.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? before)" }
        let history: ChatHistory = try await request(path)
        guard history.cursor == "0" || (try? Sequence.compare(history.cursor, "0")) != nil,
              channelID == nil || history.channel?.id == channelID,
              history.messages.allSatisfy({
                  ($0.channelId == history.channel?.id || history.channel == nil)
                      && $0.content.version == 1 && $0.content.type == "text"
                      && (try? Sequence.compare($0.seq, "0")) != nil
              }), history.pinnedMessages.count <= 100,
              history.pinnedMessages.allSatisfy({
                  ($0.channelId == history.channel?.id || history.channel == nil) && $0.pin != nil
                      && (try? Sequence.compare($0.pinSeq ?? "", "0")) != nil
              }) else {
            throw APIError(status: 502, message: "The chat service returned invalid history.")
        }
        return history
    }

    public func chatSession(name: String) async throws -> ChatSession {
        try await request("api/chat/session", method: "POST", body: SessionInput(name: name))
    }

    public func thread(channelID: String, rootID: String, before: String? = nil) async throws -> ThreadHistory {
        let path = "api/chat/channels/\(try pathID(channelID))/messages/\(try messagePathID(rootID))/thread" + (before.map { "?before=\($0)" } ?? "")
        let page: ThreadHistory = try await request(path)
        guard page.root.id == rootID, page.root.channelId == channelID, page.root.threadRootId == nil,
              page.messages.allSatisfy({ $0.channelId == channelID && $0.threadRootId == rootID && $0.content.version == 1 && $0.content.type == "text" && (try? Sequence.compare($0.seq, "0")) != nil }) else {
            throw APIError(status: 502, message: "The chat service returned an invalid thread.")
        }
        return page
    }

    public func forwardDestinations() async throws -> [ForwardDestination] {
        let result: ForwardDestinations = try await request("api/chat/forward-destinations")
        return result.destinations
    }

    public func forward(source: ChatMessage, destinationID: String, sessionToken: String, clientMessageID: String, text: String) async throws -> ChatMessage {
        struct Input: Encodable { let sourceChannelId: String; let sourceMessageId: String; let clientMessageId: String; let text: String }
        let message: ChatMessage = try await request("api/chat/channels/\(try pathID(destinationID))/forwards", method: "POST",
            body: Input(sourceChannelId: source.channelId, sourceMessageId: source.id, clientMessageId: clientMessageID, text: text), extraHeaders: ["x-caper-chat-token": sessionToken])
        guard message.channelId == destinationID, message.clientMessageId == clientMessageID.lowercased(), message.forward != nil, message.isValidForward else {
            throw APIError(status: 502, message: "Caper returned an invalid forward.")
        }
        return message
    }

    public func forwardedConversation(message: ChatMessage, before: String? = nil) async throws -> ForwardConversationHistory {
        var path = "api/chat/channels/\(try pathID(message.channelId))/forwards/\(try messagePathID(message.id))/thread"
        if let before {
            guard (try? Sequence.compare(before, "0")) != nil else { throw APIError(status: 400, message: "Invalid cursor.") }
            path += "?before=\(before)"
        }
        let history: ForwardConversationHistory = try await request(path)
        guard (try? Sequence.compare(history.cursor, "0")) != nil,
              (history.root.map { $0.content.version == 1 && $0.content.type == "text" && $0.isValidForward } ?? true),
              history.messages.allSatisfy({ $0.content.version == 1 && $0.content.type == "text" && $0.isValidForward && (try? Sequence.compare($0.seq, "0")) != nil }) else {
            throw APIError(status: 502, message: "Caper returned an invalid forwarded conversation.")
        }
        return history
    }

    public func send(channelID: String, sessionToken: String, clientMessageID: String, text: String, threadRootId: String? = nil, broadcast: Bool = false) async throws -> ChatMessage {
        try await request("api/chat/channels/\(try pathID(channelID))/messages", method: "POST", body: SendInput(clientMessageId: clientMessageID, text: text, threadRootId: threadRootId, broadcast: threadRootId == nil ? nil : broadcast), extraHeaders: ["x-caper-chat-token": sessionToken])
    }

    public func setReaction(channelID: String, messageID: String, sessionToken: String, emoji: String, active: Bool) async throws -> MessageReactionsEvent {
        let event: MessageReactionsEvent = try await request(
            "api/chat/channels/\(try pathID(channelID))/messages/\(try messagePathID(messageID))/reactions",
            method: "PUT", body: ReactionInput(emoji: emoji, active: active),
            extraHeaders: ["x-caper-chat-token": sessionToken]
        )
        guard event.isValid, event.channelId == channelID, event.messageId == messageID else {
            throw APIError(status: 502, message: "The chat service returned invalid reactions.")
        }
        return event
    }

    /// Who reacted to a message. Same account authorization as history, so
    /// read-only previews and DMs work; no chat session token is needed.
    public func reactors(channelID: String, messageID: String) async throws -> ReactorList {
        let list: ReactorList = try await request(
            "api/chat/channels/\(try pathID(channelID))/messages/\(try messagePathID(messageID))/reactions"
        )
        guard list.isValid(messageID: messageID) else {
            throw APIError(status: 502, message: "The chat service returned invalid reactions.")
        }
        return list
    }

    public func setPin(channelID: String, messageID: String, sessionToken: String, active: Bool) async throws -> MessagePinEvent {
        let event: MessagePinEvent = try await request(
            "api/chat/channels/\(try pathID(channelID))/messages/\(try messagePathID(messageID))/pin",
            method: "PUT", body: PinInput(active: active), extraHeaders: ["x-caper-chat-token": sessionToken]
        )
        guard event.isValid, event.channelId == channelID, event.message.id == messageID else {
            throw APIError(status: 502, message: "The chat service returned an invalid pin.")
        }
        return event
    }

    public func media<T: Decodable, B: Encodable>(channelID: String?, operation: String, token mediaToken: String? = nil, body: B) async throws -> T {
        let root = channelID.map { "api/channels/\($0)/media" } ?? "api/media"
        return try await request("\(root)/\(operation)", method: "POST", body: body, extraHeaders: mediaToken.map { ["x-caper-media-token": $0] } ?? [:])
    }

    /// Whether voice can be joined through this media root (web's `${mediaRoot}/status`).
    public func mediaStatus(channelID: String?) async throws -> Bool {
        struct Status: Decodable { let enabled: Bool }
        let root = try channelID.map { "api/channels/\(try pathID($0))/media" } ?? "api/media"
        let status: Status = try await request("\(root)/status")
        return status.enabled
    }

    public func media<B: Encodable>(channelID: String?, operation: String, token mediaToken: String? = nil, body: B) async throws {
        let _: Empty = try await media(channelID: channelID, operation: operation, token: mediaToken, body: body)
    }

    private func pathID(_ id: String) throws -> String {
        guard id.count == 12, id.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else { throw APIError(status: 400, message: "Invalid resource ID.") }
        return id
    }

    private func messagePathID(_ id: String) throws -> String {
        guard id.count == 15, id.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else {
            throw APIError(status: 400, message: "Invalid message ID.")
        }
        return id
    }

    private func request<T: Decodable>(_ path: String, method: String = "GET", extraHeaders: [String: String] = [:]) async throws -> T {
        try await perform(path, method: method, body: nil, extraHeaders: extraHeaders)
    }

    private func request<T: Decodable, B: Encodable>(_ path: String, method: String, body: B, extraHeaders: [String: String] = [:]) async throws -> T {
        try await perform(path, method: method, body: try encoder.encode(body), extraHeaders: extraHeaders)
    }

    private func perform<T: Decodable>(_ path: String, method: String, body: Data?, extraHeaders: [String: String]) async throws -> T {
        guard let url = URL(string: path, relativeTo: baseURL)?.absoluteURL else { throw URLError(.badURL) }
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 25)
        request.httpMethod = method
        request.httpBody = body
        if body != nil { request.setValue("application/json", forHTTPHeaderField: "content-type") }
        if let token { request.setValue("Bearer \(token)", forHTTPHeaderField: "authorization") }
        extraHeaders.forEach { request.setValue($1, forHTTPHeaderField: $0) }
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
        guard (200..<300).contains(http.statusCode) else {
            let detail = try? decoder.decode(ErrorBody.self, from: data)
            throw APIError(status: http.statusCode, message: detail?.error ?? "That request did not work.", code: detail?.code,
                           attemptsRemaining: detail?.attemptsRemaining)
        }
        if T.self == Empty.self { return Empty() as! T }
        do { return try decoder.decode(T.self, from: data) }
        catch { throw APIError(status: 502, message: "Caper returned an invalid response.") }
    }
}

public struct Empty: Codable, Sendable { public init() {} }
