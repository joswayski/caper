import Foundation
import Observation

@MainActor @Observable
public final class AppModel {
    public enum Phase { case loading, signedOut, onboarding, ready }
    public var phase: Phase = .loading
    public var account: Account?
    public var spaces: [Space] = []
    public var detail: SpaceDetail?
    public var selectedSpaceID: String?
    public var selectedChannelID: String?
    public var error: String?
    public var busy = false
    public var challengeID: String?
    public let api: APIClient
    public let chat: ChatModel
    public let voice: VoiceClient
    private var generation = 0

    public init(api: APIClient = APIClient()) {
        self.api = api
        chat = ChatModel(api: api)
        voice = VoiceClient(api: api)
    }

    public func start() async {
        generation += 1
        let attempt = generation
        do {
            let account = try await api.account()
            guard generation == attempt else { return }
            self.account = account
            phase = account == nil ? .signedOut : needsProfile ? .onboarding : .ready
            if phase == .ready { await loadSpaces() }
        } catch {
            guard generation == attempt else { return }
            self.error = error.localizedDescription; phase = .signedOut
        }
    }

    public func requestCode(email: String) async {
        let attempt = generation
        await work(generation: attempt) {
            let challengeID = try await self.api.requestCode(email: email)
            guard self.generation == attempt else { return }
            self.challengeID = challengeID
        }
    }

    public func verify(code: String) async {
        guard let challengeID else { return }
        generation += 1
        let attempt = generation
        await work(generation: attempt) {
            let account = try await self.api.verify(challengeId: challengeID, code: code)
            guard self.generation == attempt else { return }
            self.account = account
            self.phase = self.needsProfile ? .onboarding : .ready
            if self.phase == .ready { await self.loadSpaces() }
        }
    }

    public func saveProfile(username: String, displayName: String) async {
        generation += 1
        let attempt = generation
        await work(generation: attempt) {
            let account = try await self.api.updateProfile(username: username, displayName: displayName)
            guard self.generation == attempt else { return }
            self.account = account
            self.phase = .ready
            await self.loadSpaces()
        }
    }

    public func logout() async {
        generation += 1
        account = nil; spaces = []; detail = nil
        selectedSpaceID = nil; selectedChannelID = nil; challengeID = nil
        busy = false; phase = .signedOut
        async let revoke: Void = api.logout()
        await voice.leave()
        await chat.stop()
        do { try await revoke } catch { self.error = error.localizedDescription }
    }

    public func loadSpaces() async {
        let attempt = generation
        await work(generation: attempt) {
            let spaces = try await self.api.spaces().spaces
            guard self.generation == attempt else { return }
            self.spaces = spaces
            if let first = self.spaces.first { await self.select(space: first) }
            else { await self.chat.open(channelID: nil, displayName: self.account?.displayName ?? "Guest") }
        }
    }

    public func select(space: Space) async {
        let attempt = generation
        selectedSpaceID = space.id
        await work(generation: attempt) {
            let detail = try await self.api.space(space.id)
            guard self.selectedSpaceID == space.id, self.generation == attempt else { return }
            self.detail = detail
            if let channel = detail.channels.first { await self.select(channel: channel) }
        }
    }

    public func select(channel: Channel) async {
        selectedChannelID = channel.id
        await chat.open(channelID: channel.id, displayName: account?.displayName ?? "Guest")
    }

    private var needsProfile: Bool { account?.username == nil || account?.displayName == nil }
    private func work(generation expectedGeneration: Int? = nil, _ operation: () async throws -> Void) async {
        busy = true; error = nil
        do { try await operation() }
        catch {
            guard expectedGeneration == nil || generation == expectedGeneration else { return }
            self.error = error.localizedDescription
        }
        if expectedGeneration == nil || generation == expectedGeneration { busy = false }
    }
}

@MainActor @Observable
public final class ChatModel {
    public var messages: [ChatMessage] = []
    public var channelName = "general"
    public var spaceName = "Caper"
    public var draft = ""
    public var loading = false
    public var sending = false
    public var liveState: GatewayState = .disconnected
    public var error: String?
    public var hasMore = false
    private let api: APIClient
    private var channelID: String?
    private var session: ChatSession?
    private var subscriptionID: String?
    private var generation = 0
    private var delivery = ChatDeliveryState()
    @ObservationIgnored private lazy var gateway: Gateway = Gateway(baseURL: api.baseURL, token: { [api] in await api.authorizationToken() }) { [weak self] state, error in
        self?.liveState = state
        if let error { self?.error = error }
    }

    public init(api: APIClient) { self.api = api }

    public func open(channelID: String?, displayName: String) async {
        await open(channelID: channelID, displayName: displayName, preservingPending: false)
    }

    private func open(channelID: String?, displayName: String, preservingPending: Bool) async {
        generation += 1
        let requestGeneration = generation
        let oldSubscription = subscriptionID
        subscriptionID = nil
        let preservedDraft = draft
        clearLocal(preservingPending: preservingPending)
        if preservingPending { draft = preservedDraft }
        if let oldSubscription { await gateway.unsubscribe(oldSubscription) }
        guard generation == requestGeneration else { return }
        self.channelID = channelID
        loading = true; error = nil
        do {
            async let historyRequest = api.history(channelID: channelID)
            async let sessionRequest = api.chatSession(name: displayName)
            let (history, chatSession) = try await (historyRequest, sessionRequest)
            guard self.channelID == channelID, generation == requestGeneration else { return }
            let resolvedChannelID = history.channel?.id
            self.channelID = resolvedChannelID
            messages = history.messages
            delivery.reset(cursor: history.cursor, preservingPending: preservingPending)
            hasMore = history.hasMore
            channelName = history.channel?.name ?? "general"
            spaceName = history.space?.name ?? "Caper"
            session = chatSession
            guard let actualChannel = resolvedChannelID else { throw APIError(status: 502, message: "Channel metadata is missing.") }
            let newSubscription = await gateway.subscribeChat(channelID: actualChannel, after: delivery.cursor) { [weak self] event in
                self?.receive(event, generation: requestGeneration, channelID: actualChannel)
            }
            guard generation == requestGeneration, self.channelID == actualChannel else {
                await gateway.unsubscribe(newSubscription)
                return
            }
            subscriptionID = newSubscription
        } catch {
            guard generation == requestGeneration else { return }
            if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) {
                messages = []; delivery.reset(); session = nil
            }
            self.error = error.localizedDescription
        }
        if generation == requestGeneration { loading = false }
    }

    public func loadOlder() async {
        guard hasMore, let channelID, let before = messages.first?.seq else { return }
        let requestGeneration = generation
        do {
            let page = try await api.history(channelID: channelID, before: before)
            guard generation == requestGeneration, self.channelID == channelID else { return }
            merge(page.messages)
            hasMore = page.hasMore
        } catch {
            guard generation == requestGeneration, self.channelID == channelID else { return }
            self.error = error.localizedDescription
        }
    }

    public func send() async {
        guard let channelID, let session else { error = "Messaging session is unavailable."; return }
        if delivery.pending == nil, let validation = MessageValidation.error(for: draft) { error = validation; return }
        let command = delivery.begin(text: draft)
        let requestGeneration = generation
        await gateway.reportActivity()
        sending = true; error = nil
        do {
            let message = try await api.send(channelID: channelID, sessionToken: session.token, clientMessageID: command.id, text: command.text)
            guard self.channelID == channelID,
                  generation == requestGeneration || delivery.pending?.id == command.id else { return }
            guard MessageValidation.acceptsResponse(message, channelID: channelID, command: command, authorID: session.author.id) else {
                throw APIError(status: 502, message: "The chat service returned an invalid message.")
            }
            merge([message]); delivery.confirmHTTP(id: command.id); draft = ""
        } catch {
            guard self.channelID == channelID,
                  generation == requestGeneration || delivery.pending?.id == command.id else { return }
            if delivery.pending?.id != command.id {
                draft = ""
                self.error = nil
                sending = false
                return
            }
            if let apiError = error as? APIError, [400, 404, 409, 413, 422].contains(apiError.status) {
                delivery.reject(id: command.id)
                self.error = apiError.localizedDescription
            } else {
                self.error = "Send outcome is unknown. Retry to safely resend the same message. \(error.localizedDescription)"
            }
        }
        sending = false
    }

    public func stop() async {
        generation += 1
        let oldSubscription = subscriptionID
        subscriptionID = nil
        clearLocal()
        if let oldSubscription { await gateway.unsubscribe(oldSubscription) }
    }

    private func receive(_ event: [String: Any], generation eventGeneration: Int, channelID eventChannelID: String) {
        guard generation == eventGeneration, channelID == eventChannelID else { return }
        guard let type = event["type"] as? String else { return }
        if type == "subscription.error", let status = event["status"] as? Int, [401, 403, 404].contains(status) {
            messages = []; session = nil; delivery.reset(); error = event["error"] as? String ?? "Channel access ended."
            return
        }
        if type == "message.created", let raw = event["message"], let data = try? JSONSerialization.data(withJSONObject: raw), let message = try? JSONDecoder().decode(ChatMessage.self, from: data) {
            guard message.channelId == eventChannelID,
                  event["seq"] as? String == message.seq,
                  message.content.version == 1, message.content.type == "text" else {
                requestResync(generation: eventGeneration, channelID: eventChannelID)
                return
            }
            if delivery.receive(seq: message.seq) {
                merge([message])
                if delivery.confirmGateway(clientMessageID: message.clientMessageId, authorID: message.author.id, ownAuthorID: session?.author.id) {
                    draft = ""
                    error = nil
                }
                if let subscriptionID {
                    let cursor = delivery.cursor
                    Task { await gateway.updateCursor(subscription: subscriptionID, after: cursor) }
                }
            } else { requestResync(generation: eventGeneration, channelID: eventChannelID) }
        } else if type == "ready", let next = event["cursor"] as? String, next != delivery.cursor { requestResync(generation: eventGeneration, channelID: eventChannelID) }
        else if type == "resync_required" { requestResync(generation: eventGeneration, channelID: eventChannelID) }
    }

    private func requestResync(generation expectedGeneration: Int, channelID expectedChannelID: String) {
        let name = session?.author.name ?? "Guest"
        error = "Messages changed while reconnecting. Refreshing…"
        Task { [weak self] in
            guard let self, self.generation == expectedGeneration, self.channelID == expectedChannelID else { return }
            await self.open(channelID: expectedChannelID, displayName: name, preservingPending: true)
        }
    }

    private func clearLocal(preservingPending: Bool = false) {
        delivery.reset(preservingPending: preservingPending)
        session = nil; channelID = nil; messages = []; draft = ""; hasMore = false
        channelName = "general"; spaceName = "Caper"; error = nil
        loading = false; sending = false; liveState = .disconnected
    }

    private func merge(_ incoming: [ChatMessage]) {
        var byID = Dictionary(uniqueKeysWithValues: messages.map { ($0.id, $0) })
        incoming.forEach { byID[$0.id] = $0 }
        messages = byID.values.sorted { (try? Sequence.compare($0.seq, $1.seq)) == .orderedAscending }
    }
}
