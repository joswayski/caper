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
    public var limits: SpaceLimits?
    public var navigationOpen = false
    public var openingSpaceID: String?
    public var openingChannelID: String?
    public var navigationError: String?
    public let api: APIClient
    public let chat: ChatModel
    public let voice: VoiceClient
    public let presence: PresenceModel
    private let preferredInitialSpaceID: String?
    private var generation = 0
    private var demoDetail: SpaceDetail?
    private var navigationGeneration = 0
    private var navigationCacheEpoch = 0
    private var navigationTarget: (space: Space, channelID: String?)?
    private struct PreparedNavigation {
        let detail: SpaceDetail
        let channelID: String?
        let history: ChatHistory?
    }
    private struct PrefetchEntry {
        let id: UUID
        let expires: Date
        let generation: Int
        let task: Task<PreparedNavigation, Error>
    }
    private var prefetches: [String: PrefetchEntry] = [:]
    private var prefetchOrder: [String] = []
    private var visited: [String: PreparedNavigation] = [:]
    private var visitedOrder: [String] = []
    private var lastChannelBySpace: [String: String] = [:]

    public init(api: APIClient = APIClient(), preferredInitialSpaceID: String? = nil) {
        self.api = api
        self.preferredInitialSpaceID = preferredInitialSpaceID
        let chatModel = ChatModel(api: api)
        let voiceClient = VoiceClient(api: api)
        chat = chatModel
        voice = voiceClient
        presence = PresenceModel(api: api)
        chatModel.onAccessRevoked = { [weak self, weak voiceClient] channelID in
            if let channelID, voiceClient?.isActive(channelID: channelID) == true { voiceClient?.leaveImmediately() }
            if let channelID { self?.invalidateNavigation(spaceID: nil, channelID: channelID) }
            else { self?.clearNavigationCache() }
        }
    }

    public func start() async {
        generation += 1
        clearNavigationCache()
        let attempt = generation
        do {
            let account = try await api.account()
            guard generation == attempt else { return }
            self.account = account
            phase = account == nil ? .signedOut : needsProfile ? .onboarding : .ready
            if phase != .onboarding { await loadSpaces() }
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
        clearNavigationCache()
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
        guard !busy else { return }
        if let validation = ProfileValidation.error(username: username, displayName: displayName) {
            error = validation
            return
        }
        let onboarding = phase != .ready
        if onboarding {
            generation += 1
            clearNavigationCache()
        }
        let attempt = generation
        await work(generation: attempt) {
            let account = try await self.api.updateProfile(username: username, displayName: displayName)
            guard self.generation == attempt else { return }
            self.account = account
            self.phase = .ready
            if onboarding { await self.loadSpaces() }
            else { self.chat.updateAuthor(account: account) }
        }
    }

    public func logout() async {
        generation += 1
        clearNavigationCache()
        voice.leaveImmediately()
        account = nil; spaces = []; detail = nil
        selectedSpaceID = nil; selectedChannelID = nil; challengeID = nil
        navigationGeneration += 1
        navigationTarget = nil; navigationError = nil
        openingSpaceID = nil; openingChannelID = nil
        limits = nil; navigationOpen = false
        busy = false; phase = .signedOut
        async let revoke: Void = api.logout()
        await chat.stop()
        await presence.stop()
        do { try await revoke } catch { self.error = error.localizedDescription }
        await loadSpaces()
    }

    public func loadSpaces() async {
        let attempt = generation
        await work(generation: attempt) {
            async let demoRequest = self.api.history()
            let response: SpacesResponse?
            if self.account == nil { response = nil }
            else { response = try await self.api.spaces() }
            let history = try await demoRequest
            guard self.generation == attempt else { return }
            guard let spaceIdentity = history.space, let channelIdentity = history.channel else {
                throw APIError(status: 502, message: "The public conversation is unavailable.")
            }
            let demo = Space(id: spaceIdentity.id, name: spaceIdentity.name, ownerId: "", demo: true)
            let demoChannel = Channel(id: channelIdentity.id, spaceId: demo.id, name: channelIdentity.name, private: false)
            self.demoDetail = SpaceDetail(space: demo, channels: [demoChannel], members: [])
            self.limits = response?.limits
            self.spaces = [demo] + (response?.spaces ?? []).filter { $0.id != demo.id }
            if let selected = self.spaces.first(where: { $0.id == self.selectedSpaceID })
                ?? self.spaces.first(where: { $0.id == self.preferredInitialSpaceID })
                ?? self.spaces.first {
                await self.select(space: selected, preparedDemo: history)
            }
        }
    }

    public func select(space: Space) async {
        await select(space: space, preparedDemo: nil)
    }

    private func select(space: Space, preparedDemo: ChatHistory?) async {
        await navigate(space: space, channelID: nil, preparedDemo: preparedDemo)
    }

    public func select(channel: Channel) async {
        guard let space = detail?.space else { return }
        await navigate(space: space, channelID: channel.id, preparedDemo: nil)
    }

    /// Performs read-only speculative work. It never opens chat, starts a socket, or creates a chat session.
    public func prefetch(space: Space, channelID: String? = nil) {
        let key = navigationKey(spaceID: space.id, channelID: channelID)
        if let entry = prefetches[key], entry.expires > Date(), entry.generation == generation { return }
        prefetches[key]?.task.cancel()
        let cacheGeneration = generation
        let previous = visitedNavigation(spaceID: space.id, channelID: channelID)
        let api = self.api
        let demo = space.demo == true ? demoDetail : nil
        let task = Task<PreparedNavigation, Error> {
            let detail: SpaceDetail
            if let demo { detail = demo }
            else { detail = try await api.space(space.id) }
            guard detail.space.id == space.id else { throw APIError(status: 502, message: "The service returned another space.") }
            if let previousID = previous?.channelID, !detail.channels.contains(where: { $0.id == previousID }) {
                throw APIError(status: 404, message: "This channel is no longer accessible.")
            }
            let wanted = channelID ?? previous?.channelID
            let channel = wanted.flatMap { id in detail.channels.first { $0.id == id } } ?? detail.channels.first
            if channelID != nil, channel?.id != channelID { throw APIError(status: 404, message: "This channel is no longer accessible.") }
            let history: ChatHistory?
            if let retained = previous?.history { history = retained }
            else if let channel { history = try await api.history(channelID: space.demo == true ? nil : channel.id) }
            else { history = nil }
            if let history, history.space?.id != space.id || history.channel?.id != channel?.id {
                throw APIError(status: 502, message: "The service returned another conversation.")
            }
            return PreparedNavigation(detail: detail, channelID: channel?.id, history: history)
        }
        let entryID = UUID()
        prefetches[key] = PrefetchEntry(id: entryID, expires: Date().addingTimeInterval(5), generation: cacheGeneration, task: task)
        prefetchOrder.removeAll { $0 == key }; prefetchOrder.append(key)
        while prefetchOrder.count > 4 {
            let removed = prefetchOrder.removeFirst()
            prefetches.removeValue(forKey: removed)?.task.cancel()
        }
        Task { [weak self] in
            do { _ = try await task.value }
            catch {
                guard let self, self.generation == cacheGeneration, self.prefetches[key]?.id == entryID else { return }
                self.removePrefetch(key)
                if let failure = error as? APIError, [401, 403, 404].contains(failure.status) { self.invalidateNavigation(spaceID: space.id) }
            }
        }
    }

    public func retryNavigation() async {
        guard let target = navigationTarget else { return }
        await navigate(space: target.space, channelID: target.channelID, preparedDemo: nil)
    }

    private func navigate(space: Space, channelID: String?, preparedDemo: ChatHistory?) async {
        rememberCurrentNavigation()
        navigationGeneration += 1
        let navigation = navigationGeneration
        let attempt = generation
        let cacheEpoch = navigationCacheEpoch
        navigationTarget = (space, channelID)
        navigationError = nil
        openingSpaceID = space.id; openingChannelID = channelID
        var spaceVerified = false
        defer {
            if navigationGeneration == navigation {
                openingSpaceID = nil; openingChannelID = nil
            }
        }
        do {
            let key = navigationKey(spaceID: space.id, channelID: channelID)
            let prepared: PreparedNavigation?
            if let entry = prefetches[key], entry.expires > Date(), entry.generation == generation {
                removePrefetch(key)
                prepared = try await entry.task.value
                guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
            } else {
                removePrefetch(key)
                prepared = nil
            }
            let detail: SpaceDetail
            // A hover is not authorization for a later click. Recheck membership
            // even when speculative history has already completed.
            if space.demo == true, let demo = self.demoDetail { detail = demo }
            else { detail = try await self.api.space(space.id) }
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
            guard detail.space.id == space.id else { throw APIError(status: 502, message: "The service returned another space.") }
            spaceVerified = true
            let restoredChannelID = channelID ?? prepared?.channelID ?? lastChannelBySpace[space.id]
            let restoredChannel = restoredChannelID.flatMap { id in detail.channels.first { $0.id == id } }
            let channel = channelID == nil ? (restoredChannel ?? detail.channels.first) : restoredChannel
            if channelID != nil && channel == nil { throw APIError(status: 404, message: "This channel is no longer accessible.") }
            let history: ChatHistory?
            if let channel {
                if let preparedDemo, space.demo == true { history = preparedDemo }
                else if let retained = visitedNavigation(spaceID: space.id, channelID: channel.id)?.history { history = retained }
                else if prepared?.channelID == channel.id { history = prepared?.history }
                else { history = try await api.history(channelID: space.demo == true ? nil : channel.id) }
                guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
                guard history?.space?.id == space.id, history?.channel?.id == channel.id else {
                    throw APIError(status: 502, message: "The service returned another conversation.")
                }
            } else { history = nil }
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
            // Keep the current conversation, draft and selection until the target is ready.
            selectedSpaceID = space.id
            selectedChannelID = channel?.id
            self.detail = detail
            lastChannelBySpace[space.id] = channel?.id
            navigationOpen = false
            navigationTarget = nil
            if let history {
                remember(PreparedNavigation(detail: detail, channelID: channel?.id, history: history))
                await chat.open(history: history, displayName: account?.displayName ?? "Guest")
            }
            else { await chat.stop() }
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else {
                if navigationGeneration == navigation {
                    selectedSpaceID = nil; selectedChannelID = nil; self.detail = nil
                    await chat.stop()
                }
                return
            }
            if detail.space.demo == true { await self.presence.stop() }
            else { await self.presence.watch(spaceID: detail.space.id, members: detail.members) }
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
        } catch {
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
            navigationError = error.localizedDescription
            if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status),
               selectedSpaceID == space.id, !spaceVerified || channelID == nil || selectedChannelID == channelID {
                if !spaceVerified { detail = nil; selectedSpaceID = nil }
                selectedChannelID = nil
                if voice.isActive(spaceID: space.id), !spaceVerified || channelID.map({ voice.isActive(channelID: $0) }) ?? true { voice.leaveImmediately() }
                await chat.stop()
                if !spaceVerified { await presence.stop() }
            }
            if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) {
                invalidateNavigation(spaceID: space.id)
            }
        }
    }

    private func navigationKey(spaceID: String, channelID: String?) -> String { "\(spaceID):\(channelID ?? "")" }
    private func removePrefetch(_ key: String) {
        prefetches.removeValue(forKey: key)
        prefetchOrder.removeAll { $0 == key }
    }
    private func visitedNavigation(spaceID: String, channelID: String?) -> PreparedNavigation? {
        if let channelID { return visited[navigationKey(spaceID: spaceID, channelID: channelID)] }
        if let last = lastChannelBySpace[spaceID] { return visited[navigationKey(spaceID: spaceID, channelID: last)] }
        return visitedOrder.reversed().compactMap { visited[$0] }.first { $0.detail.space.id == spaceID }
    }
    private func remember(_ value: PreparedNavigation) {
        let key = navigationKey(spaceID: value.detail.space.id, channelID: value.channelID)
        visited[key] = value; visitedOrder.removeAll { $0 == key }; visitedOrder.append(key)
        while visitedOrder.count > 20 { visited.removeValue(forKey: visitedOrder.removeFirst()) }
        lastChannelBySpace = lastChannelBySpace.filter { visited[navigationKey(spaceID: $0.key, channelID: $0.value)] != nil }
    }
    private func rememberCurrentNavigation() {
        guard let detail, let snapshot = chat.currentSnapshot(), snapshot.space?.id == detail.space.id else { return }
        lastChannelBySpace[detail.space.id] = snapshot.channel?.id
        remember(PreparedNavigation(detail: detail, channelID: snapshot.channel?.id, history: snapshot))
    }
    private func clearNavigationCache() {
        navigationCacheEpoch += 1
        prefetches.values.forEach { $0.task.cancel() }
        prefetches.removeAll(); prefetchOrder.removeAll(); visited.removeAll(); visitedOrder.removeAll(); lastChannelBySpace.removeAll()
    }
    private func invalidateNavigation(spaceID: String? = nil, channelID: String? = nil) {
        navigationCacheEpoch += 1
        let keys = Set(prefetches.keys).union(visited.keys).filter { key in
            if let spaceID, !key.hasPrefix("\(spaceID):") { return false }
            // A space-only prefetch may have selected this channel; drop these
            // speculative aliases too, rather than reintroducing revoked data.
            if let channelID, !key.hasSuffix(":\(channelID)"), !key.hasSuffix(":") { return false }
            return true
        }
        for key in keys { prefetches.removeValue(forKey: key)?.task.cancel(); visited.removeValue(forKey: key) }
        prefetchOrder.removeAll { keys.contains($0) }; visitedOrder.removeAll { keys.contains($0) }
        if let spaceID, channelID == nil { lastChannelBySpace.removeValue(forKey: spaceID) }
        if let channelID { lastChannelBySpace = lastChannelBySpace.filter { $0.value != channelID } }
    }
    var navigationCacheCounts: (prefetches: Int, visited: Int) { (prefetches.count, visited.count) }

    public var isOwner: Bool { account?.id == detail?.space.ownerId }
    public var canCreateSpace: Bool {
        guard account != nil, let limits else { return false }
        return spaces.filter { $0.ownerId == account?.id }.count < limits.ownedSpaces
            && spaces.filter { $0.demo != true }.count < limits.totalSpaces
    }
    public var canCreateChannel: Bool {
        guard isOwner, let limits, let detail else { return false }
        return detail.channels.count < limits.channelsPerSpace
    }

    public func createSpace(name: String) async throws {
        clearNavigationCache()
        let attempt = generation
        if let error = WorkspaceValidation.spaceNameError(name) { throw APIError(status: 400, message: error) }
        let created = try await api.createSpace(name: name)
        guard generation == attempt, account != nil else { throw CancellationError() }
        spaces.append(created)
        await select(space: created)
    }

    public func renameSpace(_ name: String) async throws {
        guard let detail else { return }
        clearNavigationCache()
        let attempt = generation
        if let error = WorkspaceValidation.spaceNameError(name) { throw APIError(status: 400, message: error) }
        let updated = try await api.updateSpace(id: detail.space.id, name: name)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        replace(detail: SpaceDetail(space: updated, channels: detail.channels, members: detail.members))
    }

    public func deleteCurrentSpace() async throws {
        guard let id = detail?.space.id else { return }
        clearNavigationCache()
        let attempt = generation
        try await api.deleteSpace(id: id)
        guard generation == attempt, detail?.space.id == id else { throw CancellationError() }
        if voice.isActive(spaceID: id) { voice.leaveImmediately() }
        await removeCurrentSpace(id: id)
    }

    public func leaveCurrentSpace() async throws {
        guard let account, let id = detail?.space.id else { return }
        clearNavigationCache()
        let attempt = generation
        try await api.removeSpaceMember(spaceID: id, memberID: account.id)
        guard generation == attempt, self.account?.id == account.id, detail?.space.id == id else { throw CancellationError() }
        if voice.isActive(spaceID: id) { voice.leaveImmediately() }
        await removeCurrentSpace(id: id)
    }

    public func addSpaceMember(username: String) async throws {
        guard var detail else { return }
        clearNavigationCache()
        let attempt = generation
        let member = try await api.addSpaceMember(spaceID: detail.space.id, username: username)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        detail = SpaceDetail(space: detail.space, channels: detail.channels, members: detail.members.filter { $0.id != member.id } + [member])
        replace(detail: detail)
    }

    public func removeSpaceMember(_ member: Member) async throws {
        guard var detail else { return }
        clearNavigationCache()
        let attempt = generation
        try await api.removeSpaceMember(spaceID: detail.space.id, memberID: member.id)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        detail = SpaceDetail(space: detail.space, channels: detail.channels, members: detail.members.filter { $0.id != member.id })
        replace(detail: detail)
    }

    public func createChannel(name: String, privateChannel: Bool) async throws {
        guard var detail else { return }
        clearNavigationCache()
        let attempt = generation
        let clean = name.hasSuffix("-") ? String(name.dropLast()) : name
        if let error = WorkspaceValidation.channelNameError(clean) { throw APIError(status: 400, message: error) }
        let channel = try await api.createChannel(spaceID: detail.space.id, name: clean, privateChannel: privateChannel)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        detail = SpaceDetail(space: detail.space, channels: detail.channels + [channel], members: detail.members)
        replace(detail: detail)
        await select(channel: channel)
    }

    public func updateChannel(_ channel: Channel, name: String, privateChannel: Bool) async throws -> Channel {
        guard var detail else { return channel }
        clearNavigationCache()
        let attempt = generation
        let clean = name.hasSuffix("-") ? String(name.dropLast()) : name
        if let error = WorkspaceValidation.channelNameError(clean) { throw APIError(status: 400, message: error) }
        let updated = try await api.updateChannel(spaceID: detail.space.id, channelID: channel.id, name: clean, privateChannel: privateChannel)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        detail = SpaceDetail(space: detail.space, channels: detail.channels.map { $0.id == updated.id ? updated : $0 }, members: detail.members)
        replace(detail: detail)
        return updated
    }

    public func deleteChannel(_ channel: Channel) async throws {
        guard var detail else { return }
        clearNavigationCache()
        let attempt = generation
        try await api.deleteChannel(spaceID: detail.space.id, channelID: channel.id)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        if voice.isActive(channelID: channel.id) { voice.leaveImmediately() }
        detail = SpaceDetail(space: detail.space, channels: detail.channels.filter { $0.id != channel.id }, members: detail.members)
        replace(detail: detail)
        if selectedChannelID == channel.id {
            if let first = detail.channels.first { await select(channel: first) }
            else { selectedChannelID = nil; await chat.stop() }
        }
    }

    public func channelMembers(_ channel: Channel) async throws -> [Member] {
        guard let spaceID = detail?.space.id else { return [] }
        let attempt = generation
        let members = try await api.channelMembers(spaceID: spaceID, channelID: channel.id)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        return members
    }

    public func addChannelMember(_ channel: Channel, username: String) async throws -> Member {
        guard let spaceID = detail?.space.id else { throw APIError(status: 400, message: "No space is selected.") }
        clearNavigationCache()
        let attempt = generation
        let member = try await api.addChannelMember(spaceID: spaceID, channelID: channel.id, username: username)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        return member
    }

    public func removeChannelMember(_ channel: Channel, member: Member) async throws {
        guard let spaceID = detail?.space.id else { return }
        clearNavigationCache()
        let attempt = generation
        try await api.removeChannelMember(spaceID: spaceID, channelID: channel.id, memberID: member.id)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
    }

    private func replace(detail: SpaceDetail) {
        self.detail = detail
        spaces = spaces.map { $0.id == detail.space.id ? detail.space : $0 }
    }

    private func removeCurrentSpace(id: String) async {
        generation += 1
        navigationGeneration += 1
        navigationTarget = nil; navigationError = nil
        openingSpaceID = nil; openingChannelID = nil
        spaces.removeAll { $0.id == id }
        detail = nil; selectedSpaceID = nil; selectedChannelID = nil
        await chat.stop()
        await presence.stop()
        if let first = spaces.first { await select(space: first) }
    }

    public func openVoiceContext() async {
        guard let context = voice.context,
              let space = spaces.first(where: { $0.id == context.spaceID }) else { return }
        if selectedSpaceID != space.id { await select(space: space) }
        guard let channel = detail?.channels.first(where: { $0.id == context.channelID }) else { return }
        if selectedChannelID != channel.id { await select(channel: channel) }
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
    public var loadingOlder = false
    public var olderError: String?
    public var sending = false
    public var liveState: GatewayState = .disconnected
    public var error: String?
    public var hasMore = false
    public var typingNames: [String] = []
    public var currentAuthor: ChatAuthor? { session?.author }
    public var pendingMessage: PendingMessage? { delivery.pending }
    public var sendRejected: Bool { delivery.rejected }
    @ObservationIgnored public var onAccessRevoked: ((String?) -> Void)?
    private let api: APIClient
    private var channelID: String?
    private var spaceID: String?
    private var session: ChatSession?
    private var subscriptionID: String?
    private var generation = 0
    private var delivery = ChatDeliveryState()
    private var typers: [String: (author: ChatAuthor, typing: Bool, revision: String, expires: Date)] = [:]
    private var typingActive = false
    private var typingSent = false
    private var typingSentAt = Date.distantPast
    private var typingTask: Task<Void, Never>?
    private var typingIdleTask: Task<Void, Never>?
    private var typingExpiryTask: Task<Void, Never>?
    @ObservationIgnored private lazy var gateway: Gateway = Gateway(baseURL: api.baseURL, token: { [api] in await api.authorizationToken() }) { [weak self] state, error in
        self?.liveState = state
        if let error { self?.error = error }
    }

    public init(api: APIClient) { self.api = api }

    func updateAuthor(account: Account) {
        guard let session, !session.author.isGuest, session.author.id == account.id,
              let name = account.displayName else { return }
        // Account-backed sends resolve the current name server-side. Updating
        // this presentation snapshot must not reopen chat or discard its draft.
        self.session = ChatSession(token: session.token, author: ChatAuthor(id: account.id, name: name, isGuest: false))
    }

    /// A token-free copy of only the timeline currently retained by this model.
    public func currentSnapshot() -> ChatHistory? {
        guard session != nil, let channelID, let spaceID else { return nil }
        return ChatHistory(
            space: HistoryIdentity(id: spaceID, name: spaceName),
            channel: HistoryIdentity(id: channelID, name: channelName),
            messages: messages,
            cursor: delivery.cursor,
            hasMore: hasMore
        )
    }

    public func open(channelID: String?, displayName: String) async {
        await open(channelID: channelID, displayName: displayName, preservingPending: false, prepared: nil)
    }

    public func open(history: ChatHistory, displayName: String) async {
        await open(channelID: history.channel?.id, displayName: displayName, preservingPending: false, prepared: history)
    }

    private func open(channelID: String?, displayName: String, preservingPending: Bool, prepared: ChatHistory?) async {
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
            async let sessionRequest = api.chatSession(name: displayName)
            let history: ChatHistory
            if let prepared { history = prepared }
            else { history = try await api.history(channelID: channelID) }
            let chatSession = try await sessionRequest
            guard self.channelID == channelID, generation == requestGeneration else { return }
            let resolvedChannelID = history.channel?.id
            spaceID = history.space?.id
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
            if CaperRuntime.isChatPreview("chat-rejected") {
                let preview = delivery.begin(text: "Fixture message that was rejected")
                delivery.reject(id: preview.id)
                error = "Fixture rejection; no message was sent."
            }
        } catch {
            guard generation == requestGeneration else { return }
            if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) {
                messages = []; delivery.reset(); session = nil
                onAccessRevoked?(channelID)
            }
            self.error = error.localizedDescription
        }
        if generation == requestGeneration { loading = false }
    }

    public func loadOlder() async {
        guard !loadingOlder, hasMore, let channelID, let before = messages.first?.seq else { return }
        let requestGeneration = generation
        loadingOlder = true; olderError = nil
        defer { if generation == requestGeneration { loadingOlder = false } }
        do {
            let page = try await api.history(channelID: channelID, before: before)
            guard generation == requestGeneration, self.channelID == channelID else { return }
            merge(page.messages)
            hasMore = page.hasMore
        } catch {
            guard generation == requestGeneration, self.channelID == channelID else { return }
            if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) {
                onAccessRevoked?(channelID)
                await stop()
                guard generation == requestGeneration + 1 else { return }
                self.error = error.localizedDescription
            } else { olderError = error.localizedDescription }
        }
    }

    public func send() async {
        guard !sending, !delivery.rejected else { return }
        guard let channelID, let session else { error = "Messaging session is unavailable."; return }
        if delivery.pending == nil, let validation = MessageValidation.error(for: draft) { error = validation; return }
        let newSubmission = delivery.pending == nil
        let command = delivery.begin(text: draft)
        if newSubmission { draft = "" }
        let requestGeneration = generation
        sending = true; error = nil
        await gateway.reportActivity()
        do {
            let message = try await api.send(channelID: channelID, sessionToken: session.token, clientMessageID: command.id, text: command.text)
            guard self.channelID == channelID,
                  generation == requestGeneration || delivery.pending?.id == command.id else { return }
            guard MessageValidation.acceptsResponse(message, channelID: channelID, command: command, authorID: session.author.id) else {
                throw APIError(status: 502, message: "The chat service returned an invalid message.")
            }
            merge([message]); delivery.confirmHTTP(id: command.id)
        } catch {
            guard self.channelID == channelID,
                  generation == requestGeneration || delivery.pending?.id == command.id else { return }
            if delivery.pending?.id != command.id {
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

    @discardableResult public func discardRejected(edit: Bool = false) -> Bool {
        guard !sending, delivery.rejected, !edit || draft.isEmpty else { return false }
        guard let text = delivery.discardRejected() else { return false }
        if edit { draft = text }
        error = nil
        return true
    }

    public func setTyping(_ active: Bool) {
        typingIdleTask?.cancel()
        typingActive = active
        if active {
            typingIdleTask = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(500))
                guard !Task.isCancelled else { return }
                await MainActor.run { self?.setTyping(false) }
            }
        }
        flushTyping()
    }

    public func stop() async {
        generation += 1
        let oldSubscription = subscriptionID
        subscriptionID = nil
        clearLocal()
        if let oldSubscription { await gateway.unsubscribe(oldSubscription) }
    }

    func receive(_ event: [String: Any], generation eventGeneration: Int, channelID eventChannelID: String) {
        guard generation == eventGeneration, channelID == eventChannelID else { return }
        guard let type = event["type"] as? String else { return }
        if type == "subscription.error", let status = event["status"] as? Int, [401, 403, 404].contains(status) {
            messages = []; session = nil; delivery.reset(); error = event["error"] as? String ?? "Channel access ended."
            onAccessRevoked?(eventChannelID)
            return
        }
        if type == "typing.updated",
           event["channelId"] as? String == eventChannelID,
           let rawAuthor = event["author"],
           let data = try? JSONSerialization.data(withJSONObject: rawAuthor),
           let author = try? JSONDecoder().decode(ChatAuthor.self, from: data),
           let typing = event["typing"] as? Bool,
           let revision = event["revision"] as? String {
            receiveTyping(author: author, typing: typing, revision: revision)
            return
        }
        if type == "message.created", let raw = event["message"], let data = try? JSONSerialization.data(withJSONObject: raw), let message = try? JSONDecoder().decode(ChatMessage.self, from: data) {
            guard message.channelId == eventChannelID,
                  event["seq"] as? String == message.seq,
                  message.content.version == 1, message.content.type == "text" else {
                requestResync(generation: eventGeneration, channelID: eventChannelID)
                return
            }
            let newMessage = (try? Sequence.compare(message.seq, delivery.cursor)) == .orderedDescending
            if delivery.receive(seq: message.seq) {
                merge([message])
                if newMessage, let author = session?.author, author.id != message.author.id { CaperEffects.shared.play(.message) }
                if delivery.confirmGateway(clientMessageID: message.clientMessageId, authorID: message.author.id, ownAuthorID: session?.author.id) {
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
            await self.open(channelID: expectedChannelID, displayName: name, preservingPending: true, prepared: nil)
        }
    }

    private func clearLocal(preservingPending: Bool = false) {
        typingTask?.cancel(); typingIdleTask?.cancel(); typingExpiryTask?.cancel()
        typingTask = nil; typingIdleTask = nil; typingExpiryTask = nil
        typers = [:]; typingNames = []; typingActive = false; typingSent = false
        delivery.reset(preservingPending: preservingPending)
        session = nil; channelID = nil; spaceID = nil; messages = []; draft = ""; hasMore = false
        channelName = "general"; spaceName = "Caper"; error = nil
        loadingOlder = false; olderError = nil
        loading = false; sending = false; liveState = .disconnected
    }

    private func merge(_ incoming: [ChatMessage]) {
        var byID = Dictionary(uniqueKeysWithValues: messages.map { ($0.id, $0) })
        incoming.forEach { byID[$0.id] = $0 }
        messages = byID.values.sorted { (try? Sequence.compare($0.seq, $1.seq)) == .orderedAscending }
    }

    private func flushTyping() {
        guard let channelID, let session else { return }
        let active = typingActive
        if !active && !typingSent { return }
        if active && typingSent && Date().timeIntervalSince(typingSentAt) < 0.5 { return }
        typingSent = active; typingSentAt = Date()
        let requestGeneration = generation
        let previous = typingTask
        typingTask = Task { [weak self] in
            await previous?.value
            guard let self, self.generation == requestGeneration, self.channelID == channelID else { return }
            _ = try? await self.gateway.command(
                method: "typing", channelID: channelID, chatToken: session.token,
                body: ["typing": active], timeout: .seconds(2)
            )
            guard self.generation == requestGeneration else { return }
            if self.typingActive != active { self.flushTyping() }
        }
    }

    private func receiveTyping(author: ChatAuthor, typing: Bool, revision: String) {
        guard author.id != session?.author.id, (try? Sequence.compare(revision, "0")) != nil else { return }
        if let previous = typers[author.id], (try? Sequence.compare(revision, previous.revision)) != .orderedDescending { return }
        guard typers[author.id] != nil || typers.count < 64 else { return }
        typers[author.id] = (author, typing, revision, Date().addingTimeInterval(6))
        refreshTypers()
    }

    private func refreshTypers() {
        let now = Date()
        typers = typers.filter { $0.value.expires > now }
        typingNames = typers.values.compactMap { entry in
            // Stop tombstones remain to reject delayed frames but are not shown.
            entry.typing && entry.expires > now && entry.author.id != session?.author.id ? entry.author.name : nil
        }
        typingExpiryTask?.cancel()
        guard let expiry = typers.values.map(\.expires).min() else { return }
        typingExpiryTask = Task { [weak self] in
            let delay = max(0, expiry.timeIntervalSinceNow)
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled else { return }
            await MainActor.run { self?.refreshTypers() }
        }
    }
}
