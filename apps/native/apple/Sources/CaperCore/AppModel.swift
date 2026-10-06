import Foundation
import Observation

@MainActor @Observable
public final class AppModel {
    public enum Phase { case loading, signedOut, onboarding, ready }
    public var phase: Phase = .loading
    public var account: Account?
    public var spaces: [Space] = []
    public var invitations: [Space] = []
    public var pendingMembers: [Member] = []
    public var detail: SpaceDetail?
    public var selectedSpaceID: String?
    public var selectedChannelID: String?
    public var directMessages: [DirectMessageConversation] = []
    public var selectedDirectMessageID: String?
    /// `GET /api/people` for `@` suggestions in DMs; nil until the first load
    /// succeeds. A refresh keeps the previous list until it completes.
    public private(set) var people: [Person]?
    public var pushAvailable = false
    public var pushEnabled = false
    @ObservationIgnored public var setPushEnabled: ((Bool) async -> Void)?
    @ObservationIgnored public var disablePushLocally: (() -> Void)?
    public var previewingChannel: Bool { selectedChannel?.joined == false }
    public var selectedChannel: Channel? { detail?.channels.first { $0.id == selectedChannelID } }
    public var error: String?
    public var busy = false
    public var challengeID: String? { didSet { if challengeID != oldValue { loginAttemptsRemaining = nil } } }
    /// Remaining code attempts reported by the last rejected verification, as on web.
    public var loginAttemptsRemaining: Int?
    public var limits: SpaceLimits?
    /// The account's space list has loaded at least once. With no spaces,
    /// the workspace shows web's "Name your space" first-space form.
    public var spacesLoaded = false
    public var spacesError: String?
    public var navigationOpen = false
    public var openingSpaceID: String?
    public var openingChannelID: String?
    /// Voice availability by media root ("general" or a channel id); nil while unchecked.
    public private(set) var voiceAvailability: [String: Bool] = [:]
    public private(set) var pendingVoiceChannelID: String?
    public private(set) var pendingVoiceStartedAt: Int64 = 0
    public var navigationError: String?
    public let api: APIClient
    public let chat: ChatModel
    public let voice: VoiceClient
    public let presence: PresenceModel
    public let voicePresence: VoicePresenceModel
    private let preferredInitialSpaceID: String?
    private var generation = 0
    private var navigationGeneration = 0
    private var voiceJoinGeneration = 0
    private var peopleRequest = 0
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
    private var directMessageRefreshTask: Task<Void, Never>?
    private var foreground = false

    public init(api: APIClient = APIClient(), preferredInitialSpaceID: String? = nil) {
        self.api = api
        self.preferredInitialSpaceID = preferredInitialSpaceID
        let chatModel = ChatModel(api: api)
        let voiceClient = VoiceClient(api: api)
        chat = chatModel
        voice = voiceClient
        presence = PresenceModel(api: api)
        voicePresence = VoicePresenceModel(api: api)
        chatModel.onReadCursor = { [weak self] in self?.markSelectedDirectRead() }
        chatModel.onAccessRevoked = { [weak self, weak voiceClient] channelID in
            if let channelID, voiceClient?.isActive(channelID: channelID) == true { voiceClient?.leaveImmediately() }
            if let channelID { self?.voicePresence.revoke(channelID: channelID) }
            if let channelID { self?.invalidateNavigation(spaceID: nil, channelID: channelID) }
            else { self?.clearNavigationCache() }
        }
    }

    public func start() async {
        generation += 1
        voiceJoinGeneration += 1
        pendingVoiceChannelID = nil
        voiceAvailability.removeAll()
        await voicePresence.stop()
        clearNavigationCache()
        let attempt = generation
        do {
            let account = try await api.account()
            guard generation == attempt else { return }
            self.account = account
            phase = account == nil ? .signedOut : needsProfile ? .onboarding : .ready
            if phase == .ready { await loadSpaces(); startDirectMessageRefresh() }
        } catch {
            guard generation == attempt else { return }
            self.error = error.localizedDescription; phase = .signedOut
        }
    }

    public func requestCode(email: String) async {
        let attempt = generation
        await work(generation: attempt) {
            let challengeID: String
            do { challengeID = try await self.api.requestCode(email: email) }
            catch { throw Self.loginFailure(error) }
            guard self.generation == attempt else { return }
            self.challengeID = challengeID
            self.loginAttemptsRemaining = nil
        }
    }

    public func verify(code: String) async {
        guard let challengeID else { return }
        generation += 1
        voiceJoinGeneration += 1
        pendingVoiceChannelID = nil
        voiceAvailability.removeAll()
        await voicePresence.stop()
        clearNavigationCache()
        let attempt = generation
        await work(generation: attempt) {
            let account: Account
            do { account = try await self.api.verify(challengeId: challengeID, code: code) }
            catch {
                if self.generation == attempt, let api = error as? APIError, api.status == 401 {
                    self.loginAttemptsRemaining = api.attemptsRemaining
                }
                throw Self.loginFailure(error)
            }
            guard self.generation == attempt else { return }
            self.account = account
            self.phase = self.needsProfile ? .onboarding : .ready
            if self.phase == .ready { await self.loadSpaces(); self.startDirectMessageRefresh() }
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
            let account: Account
            do { account = try await self.api.updateProfile(username: username, displayName: displayName) }
            catch {
                let status = (error as? APIError)?.status
                throw UserFacingError(message: status == 409 ? "That username is already taken."
                    : status == 400 ? "Check the username and display name requirements."
                    : "Your profile could not be saved. Please try again.")
            }
            guard self.generation == attempt else { return }
            self.account = account
            self.phase = .ready
            if onboarding { await self.loadSpaces(); self.startDirectMessageRefresh() }
            else { self.chat.updateAuthor(account: account) }
        }
    }

    public func logout() async {
        // Opt out synchronously. Session revocation handles server cleanup;
        // optional provider/network work must never hold the signed-in UI open.
        disablePushLocally?()
        pushEnabled = false; pushAvailable = false
        generation += 1
        voiceJoinGeneration += 1
        pendingVoiceChannelID = nil
        voiceAvailability.removeAll()
        clearNavigationCache()
        voice.leaveImmediately()
        directMessageRefreshTask?.cancel(); directMessageRefreshTask = nil
        account = nil; spaces = []; invitations = []; pendingMembers = []; detail = nil
        directMessages = []; selectedDirectMessageID = nil; people = nil
        spacesLoaded = false; spacesError = nil
        selectedSpaceID = nil; selectedChannelID = nil; challengeID = nil
        navigationGeneration += 1
        navigationTarget = nil; navigationError = nil
        openingSpaceID = nil; openingChannelID = nil
        limits = nil; navigationOpen = false
        busy = false; phase = .signedOut
        async let revoke: Void = api.logout()
        await chat.stop()
        await presence.stop()
        await voicePresence.stop()
        do { try await revoke } catch { self.error = error.localizedDescription }
    }

    public func loadSpaces() async {
        guard account != nil else { return }
        let attempt = generation
        spacesError = nil
        await work(generation: attempt) {
            let response: SpacesResponse
            do { response = try await self.api.spaces() }
            catch {
                if self.generation == attempt { self.spacesError = error.localizedDescription }
                throw error
            }
            guard self.generation == attempt else { return }
            self.limits = response.limits
            self.spaces = response.spaces
            self.invitations = response.invitations
            self.spacesLoaded = true
            if let selected = self.spaces.first(where: { $0.id == self.selectedSpaceID })
                ?? self.spaces.first(where: { $0.id == self.preferredInitialSpaceID })
                ?? self.spaces.first {
                await self.select(space: selected)
            }
        }
    }

    public func select(space: Space) async {
        await navigate(space: space, channelID: nil)
    }

    public func select(channel: Channel) async {
        guard let space = detail?.space else { return }
        if selectedSpaceID == space.id, selectedChannelID == channel.id,
           selectedDirectMessageID == nil, chat.isPreview == !channel.joined {
            // The displayed conversation remains usable while another target
            // opens. Clicking it cancels that transition, not the live chat.
            // Notes creation keeps the current chat visible while `busy`.
            if navigationTarget != nil || busy {
                navigationGeneration += 1
                navigationTarget = nil
                openingSpaceID = nil; openingChannelID = nil
                navigationError = nil
            }
            navigationOpen = false
            return
        }
        if openingSpaceID == space.id, openingChannelID == channel.id { return }
        await navigate(space: space, channelID: channel.id)
    }

    public func refreshDirectMessages() async {
        guard account != nil else { return }
        let attempt = generation
        do {
            let conversations = try await api.directMessages()
            guard generation == attempt, account != nil else { return }
            directMessages = conversations
        } catch is CancellationError {} catch {
            guard generation == attempt else { return }
            self.error = error.localizedDescription
        }
    }

    /// Refreshes DM `@` candidates. Failure is silent: suggestions keep the
    /// last list, or fall back to the DM peer when none has loaded.
    public func refreshPeople() async {
        guard account != nil else { return }
        let attempt = generation
        peopleRequest += 1
        let request = peopleRequest
        do {
            let loaded = try await api.people()
            guard generation == attempt, peopleRequest == request, account != nil else { return }
            people = loaded
        } catch {}
    }

    public func createDirectMessage(username: String) async -> Bool {
        let exact = username.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !exact.isEmpty, account != nil, !busy else { return false }
        let attempt = generation
        navigationGeneration += 1
        let navigation = navigationGeneration
        busy = true; error = nil
        defer { if generation == attempt { busy = false } }
        do {
            let conversation = try await api.createDirectMessage(username: exact)
            guard generation == attempt else { return false }
            if let index = directMessages.firstIndex(where: { $0.id == conversation.id }) { directMessages[index] = conversation }
            else { directMessages.append(conversation) }
            if navigationGeneration == navigation { await select(directMessage: conversation) }
            return generation == attempt
        } catch { if generation == attempt { self.error = error.localizedDescription }; return false }
    }

    /// Opens the account's notes conversation, creating it through the normal DM
    /// endpoint only when the server has not returned one yet.
    public func openSelfDirectMessage() async {
        guard let account, let username = account.username else { return }
        if let conversation = directMessages.first(where: { $0.peer.id == account.id }) {
            await select(directMessage: conversation)
        } else {
            _ = await createDirectMessage(username: username)
        }
    }

    public func select(directMessage conversation: DirectMessageConversation) async {
        let attempt = generation
        navigationGeneration += 1
        let navigation = navigationGeneration
        navigationTarget = nil; navigationError = nil
        openingSpaceID = nil; openingChannelID = nil
        do {
            let history = try await api.history(channelID: conversation.id)
            guard generation == attempt, navigationGeneration == navigation,
                  history.space?.id == "", history.channel?.id == conversation.id,
                  history.channel?.direct == true else {
                throw APIError(status: 502, message: "Caper returned another conversation.")
            }
            selectedDirectMessageID = conversation.id; selectedChannelID = nil
            navigationOpen = false
            Task { [weak self] in await self?.refreshPeople() }
            await presence.stop(); await voicePresence.stop()
            await chat.open(history: history, displayName: account?.displayName ?? "")
            guard generation == attempt, navigationGeneration == navigation else { return }
            markSelectedDirectRead()
        } catch { if generation == attempt, navigationGeneration == navigation { navigationError = error.localizedDescription } }
    }

    public func openDirectMessage(id: String) async {
        await refreshDirectMessages()
        guard let conversation = directMessages.first(where: { $0.id == id }) else { return }
        await select(directMessage: conversation)
    }

    public func applicationActivityChanged(active: Bool) {
        foreground = active
        if active, account != nil {
            markSelectedDirectRead()
            Task { await refreshDirectMessages() }
        }
    }

    private func markSelectedDirectRead() {
        guard foreground, account != nil, let id = selectedDirectMessageID,
              let history = chat.currentSnapshot(), history.channel?.id == id,
              let index = directMessages.firstIndex(where: { $0.id == id }),
              (try? Sequence.compare(history.cursor, directMessages[index].readSeq)) == .orderedDescending else { return }
        let attempt = generation
        Task {
            do {
                try await api.markDirectMessageRead(id: id, seq: history.cursor)
                guard generation == attempt, let current = directMessages.firstIndex(where: { $0.id == id }) else { return }
                let old = directMessages[current]
                let seq = (try? Sequence.compare(history.cursor, old.readSeq)) == .orderedDescending ? history.cursor : old.readSeq
                directMessages[current] = DirectMessageConversation(id: old.id, peer: old.peer, lastSeq: old.lastSeq, readSeq: seq)
            } catch { /* A later foreground refresh retries the read cursor. */ }
        }
    }

    public func configurePush(available: Bool, enabled: Bool) {
        pushAvailable = available; pushEnabled = available && enabled
    }

    public func changePushEnabled(_ enabled: Bool) async {
        guard pushAvailable else { return }
        await setPushEnabled?(enabled)
    }

    private func startDirectMessageRefresh() {
        directMessageRefreshTask?.cancel()
        let attempt = generation
        directMessageRefreshTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshDirectMessages()
                try? await Task.sleep(for: .seconds(15))
                guard self?.generation == attempt else { return }
            }
        }
    }

    /// Performs read-only speculative work. It never opens chat, starts a socket, or creates a chat session.
    public func prefetch(space: Space, channelID: String? = nil) {
        if selectedSpaceID == space.id, selectedDirectMessageID == nil,
           channelID == nil || channelID == selectedChannelID { return }
        let key = navigationKey(spaceID: space.id, channelID: channelID)
        if let entry = prefetches[key], entry.expires > Date(), entry.generation == generation { return }
        prefetches[key]?.task.cancel()
        let cacheGeneration = generation
        let previous = visitedNavigation(spaceID: space.id, channelID: channelID)
        let api = self.api
        let task = Task<PreparedNavigation, Error> {
            let detail = try await api.space(space.id)
            guard detail.space.id == space.id else { throw APIError(status: 502, message: "The service returned another space.") }
            if let previousID = previous?.channelID, !detail.channels.contains(where: { $0.id == previousID }) {
                throw APIError(status: 404, message: "This channel is no longer accessible.")
            }
            let wanted = channelID ?? previous?.channelID
            let channel = wanted.flatMap { id in detail.channels.first { $0.id == id } } ?? detail.channels.first
            if channelID != nil, channel?.id != channelID { throw APIError(status: 404, message: "This channel is no longer accessible.") }
            let history: ChatHistory?
            if let retained = previous?.history { history = retained }
            else if let channel { history = try await api.history(channelID: channel.id) }
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
        await navigate(space: target.space, channelID: target.channelID)
    }

    private func navigate(space: Space, channelID: String?) async {
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
            detail = try await self.api.space(space.id)
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
            guard detail.space.id == space.id else { throw APIError(status: 502, message: "The service returned another space.") }
            spaceVerified = true
            let restoredChannelID = channelID ?? prepared?.channelID ?? lastChannelBySpace[space.id]
            let restoredChannel = restoredChannelID.flatMap { id in detail.channels.first { $0.id == id } }
            let channel = channelID == nil ? (restoredChannel?.joined == true ? restoredChannel : detail.channels.first { $0.joined }) : restoredChannel
            if channelID != nil && channel == nil { throw APIError(status: 404, message: "This channel is no longer accessible.") }
            let history: ChatHistory?
            if let channel {
                if let retained = visitedNavigation(spaceID: space.id, channelID: channel.id)?.history { history = retained }
                else if prepared?.channelID == channel.id { history = prepared?.history }
                else { history = try await api.history(channelID: channel.id) }
                guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
                guard history?.space?.id == space.id, history?.channel?.id == channel.id else {
                    throw APIError(status: 502, message: "The service returned another conversation.")
                }
            } else { history = nil }
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
            // Keep the current conversation, draft and selection until the target is ready.
            if selectedSpaceID != space.id {
                await voicePresence.stop()
                guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
            }
            selectedSpaceID = space.id
            selectedDirectMessageID = nil
            selectedChannelID = channel?.id
            self.detail = detail
            lastChannelBySpace[space.id] = channel?.id
            navigationOpen = false
            navigationTarget = nil
            if let history {
                remember(PreparedNavigation(detail: detail, channelID: channel?.id, history: history))
                if channel?.joined == true { await chat.open(history: history, displayName: account?.displayName ?? "Guest") }
                else { await chat.preview(history: history) }
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
            await voicePresence.watch(spaceID: detail.space.id, channels: detail.channels.filter(\.joined), demo: detail.space.demo == true)
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
        } catch {
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
            let missingSpace = (error as? APIError)?.status == 404 && !spaceVerified
            navigationError = missingSpace ? "This space is no longer available." : error.localizedDescription
            if missingSpace {
                spaces.removeAll { $0.id == space.id }
                invitations.removeAll { $0.id == space.id }
                invalidateNavigation(spaceID: space.id)
            }
            if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status),
               selectedSpaceID == space.id, !spaceVerified || channelID == nil || selectedChannelID == channelID {
                if !spaceVerified { detail = nil; selectedSpaceID = nil }
                selectedChannelID = nil
                if voice.isActive(spaceID: space.id), !spaceVerified || channelID.map({ voice.isActive(channelID: $0) }) ?? true { voice.leaveImmediately() }
                await chat.stop()
                if !spaceVerified { await presence.stop(); await voicePresence.stop() }
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
        if let channelID { voicePresence.revoke(channelID: channelID) }
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
        replace(detail: SpaceDetail(space: updated, channels: detail.channels, members: detail.members, channelInvitations: detail.channelInvitations))
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
        guard let detail else { return }
        if let error = WorkspaceValidation.usernameError(username) { throw APIError(status: 400, message: error) }
        let attempt = generation
        let member = try await api.addSpaceMember(spaceID: detail.space.id, username: username)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        pendingMembers.removeAll { $0.id == member.id }
        pendingMembers.append(member)
    }

    public func loadSpaceInvitations() async throws {
        guard let id = detail?.space.id, isOwner else { pendingMembers = []; return }
        let attempt = generation
        let members = try await api.spaceInvitations(spaceID: id)
        guard generation == attempt, detail?.space.id == id else { throw CancellationError() }
        pendingMembers = members
    }

    public func cancelSpaceInvitation(_ member: Member) async throws {
        guard let id = detail?.space.id else { return }
        let attempt = generation
        try await api.cancelSpaceInvitation(spaceID: id, userID: member.id)
        guard generation == attempt, detail?.space.id == id else { throw CancellationError() }
        pendingMembers.removeAll { $0.id == member.id }
    }

    public func acceptInvitation(_ invitation: Space) async throws {
        let attempt = generation
        let accepted = try await api.acceptSpaceInvitation(spaceID: invitation.id)
        guard generation == attempt else { throw CancellationError() }
        invitations.removeAll { $0.id == invitation.id }
        spaces.removeAll { $0.id == accepted.id }; spaces.append(accepted)
        await select(space: accepted)
    }

    public func declineInvitation(_ invitation: Space) async throws {
        let attempt = generation
        try await api.declineSpaceInvitation(spaceID: invitation.id)
        guard generation == attempt else { throw CancellationError() }
        invitations.removeAll { $0.id == invitation.id }
    }

    public func removeSpaceMember(_ member: Member) async throws {
        guard var detail else { return }
        clearNavigationCache()
        let attempt = generation
        try await api.removeSpaceMember(spaceID: detail.space.id, memberID: member.id)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        detail = SpaceDetail(space: detail.space, channels: detail.channels, members: detail.members.filter { $0.id != member.id }, channelInvitations: detail.channelInvitations)
        replace(detail: detail)
    }

    @discardableResult
    public func createChannel(name: String, privateChannel: Bool) async throws -> Channel? {
        guard var detail else { return nil }
        clearNavigationCache()
        let attempt = generation
        let clean = name.hasSuffix("-") ? String(name.dropLast()) : name
        if let error = WorkspaceValidation.channelNameError(clean) { throw APIError(status: 400, message: error) }
        let channel = try await api.createChannel(spaceID: detail.space.id, name: clean, privateChannel: privateChannel)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        detail = SpaceDetail(space: detail.space, channels: detail.channels + [channel], members: detail.members, channelInvitations: detail.channelInvitations)
        replace(detail: detail)
        await select(channel: channel)
        return channel
    }

    public func updateChannel(_ channel: Channel, name: String, privateChannel: Bool) async throws -> Channel {
        guard var detail else { return channel }
        clearNavigationCache()
        let attempt = generation
        let clean = name.hasSuffix("-") ? String(name.dropLast()) : name
        if let error = WorkspaceValidation.channelNameError(clean) { throw APIError(status: 400, message: error) }
        let updated = try await api.updateChannel(spaceID: detail.space.id, channelID: channel.id, name: clean, privateChannel: privateChannel)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        detail = SpaceDetail(space: detail.space, channels: detail.channels.map { $0.id == updated.id ? updated : $0 }, members: detail.members, channelInvitations: detail.channelInvitations)
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
        voicePresence.revoke(channelID: channel.id)
        detail = SpaceDetail(space: detail.space, channels: detail.channels.filter { $0.id != channel.id }, members: detail.members, channelInvitations: detail.channelInvitations)
        replace(detail: detail)
        if selectedChannelID == channel.id {
            if let first = detail.channels.first(where: \.joined) { await select(channel: first) }
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

    public func channelInvitations(_ channel: Channel) async throws -> [Member] {
        guard let spaceID = detail?.space.id else { return [] }
        let attempt = generation
        let invitations = try await api.channelInvitations(spaceID: spaceID, channelID: channel.id)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        return invitations
    }

    public func addChannelMember(_ channel: Channel, username: String) async throws -> Member {
        guard let spaceID = detail?.space.id else { throw APIError(status: 400, message: "No space is selected.") }
        if let error = WorkspaceValidation.usernameError(username) { throw APIError(status: 400, message: error) }
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

    public func joinChannel(_ channel: Channel) async throws {
        guard let spaceID = detail?.space.id else { return }
        let attempt = generation
        clearNavigationCache()
        _ = try await api.joinChannel(spaceID: spaceID, channelID: channel.id)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        try await refreshDetail(spaceID: spaceID, selecting: channel.id)
    }

    public func leaveChannel(_ channel: Channel) async throws {
        guard let spaceID = detail?.space.id else { return }
        let attempt = generation
        let navigation = navigationGeneration
        clearNavigationCache()
        try await api.leaveChannel(spaceID: spaceID, channelID: channel.id)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        if voice.isActive(channelID: channel.id) { voice.leaveImmediately() }
        voicePresence.revoke(channelID: channel.id)
        let selecting = selectedChannelID == channel.id ? nil : selectedChannelID
        if let current = detail {
            let channels = current.channels.compactMap { item -> Channel? in
                guard item.id == channel.id else { return item }
                if item.private && !isOwner { return nil }
                return Channel(id: item.id, spaceId: item.spaceId, name: item.name, private: item.private, joined: false)
            }
            replace(detail: SpaceDetail(space: current.space, channels: channels, members: current.members, channelInvitations: current.channelInvitations))
        }
        if selectedChannelID == channel.id { selectedChannelID = nil; await chat.stop() }
        guard generation == attempt, navigationGeneration == navigation, detail?.space.id == spaceID else { throw CancellationError() }
        try await refreshDetail(spaceID: spaceID, selecting: selecting)
    }

    public func acceptChannelInvitation(_ invitation: ChannelInvitation) async throws {
        guard let spaceID = detail?.space.id else { return }
        let attempt = generation
        clearNavigationCache()
        _ = try await api.acceptChannelInvitation(spaceID: spaceID, channelID: invitation.channel.id)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        try await refreshDetail(spaceID: spaceID, selecting: invitation.channel.id)
    }

    public func declineChannelInvitation(_ invitation: ChannelInvitation) async throws {
        guard let spaceID = detail?.space.id else { return }
        let attempt = generation
        clearNavigationCache()
        try await api.declineChannelInvitation(spaceID: spaceID, channelID: invitation.channel.id)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        try await refreshDetail(spaceID: spaceID, selecting: selectedChannelID)
    }

    private func refreshDetail(spaceID: String, selecting channelID: String?) async throws {
        let attempt = generation
        let navigation = navigationGeneration
        let refreshed = try await api.space(spaceID)
        guard generation == attempt, navigationGeneration == navigation, detail?.space.id == spaceID else { throw CancellationError() }
        replace(detail: refreshed)
        let retained = channelID.flatMap { id in refreshed.channels.first { $0.id == id } }
        if let retained { await select(channel: retained) }
        else if let first = refreshed.channels.first(where: \.joined) { await select(channel: first) }
        else { selectedChannelID = nil; await chat.stop() }
    }

    private func replace(detail: SpaceDetail) {
        self.detail = detail
        spaces = spaces.map { $0.id == detail.space.id ? detail.space : $0 }
        let attempt = generation
        Task { [weak self] in
            guard let self, self.generation == attempt, self.detail?.space.id == detail.space.id else { return }
            await self.voicePresence.watch(spaceID: detail.space.id, channels: detail.channels.filter(\.joined), demo: detail.space.demo == true)
        }
    }

    private func removeCurrentSpace(id: String) async {
        let directMessageOpen = selectedDirectMessageID != nil
        generation += 1
        voiceJoinGeneration += 1
        pendingVoiceChannelID = nil
        navigationGeneration += 1
        navigationTarget = nil; navigationError = nil
        openingSpaceID = nil; openingChannelID = nil
        spaces.removeAll { $0.id == id }
        detail = nil; selectedSpaceID = nil; selectedChannelID = nil
        if !directMessageOpen { await chat.stop() }
        await presence.stop()
        await voicePresence.stop()
        startDirectMessageRefresh()
        if !directMessageOpen, let first = spaces.first { await select(space: first) }
    }

    public func openVoiceContext() async {
        guard let context = voice.context,
              let space = spaces.first(where: { $0.id == context.spaceID }) else { return }
        if selectedSpaceID != space.id { await select(space: space) }
        guard let channel = detail?.channels.first(where: { $0.id == context.channelID }) else { return }
        if selectedChannelID != channel.id { await select(channel: channel) }
    }

    public func leaveVoice() {
        voiceJoinGeneration += 1
        pendingVoiceChannelID = nil
        voice.leaveImmediately()
    }

    /// The media root voice uses for the viewed channel, as web keys its availability.
    public var viewedVoiceRoot: String? {
        guard let detail, let channelID = selectedChannelID, selectedChannel?.joined == true else { return nil }
        return detail.space.demo == true ? "general" : channelID
    }

    public var voiceAvailable: Bool? { viewedVoiceRoot.flatMap { voiceAvailability[$0] } }

    public func voiceAvailable(in channel: Channel) -> Bool? {
        guard let detail, detail.channels.contains(where: { $0.id == channel.id }) else { return nil }
        if voicePresence.unavailableChannels.contains(channel.id) { return false }
        return voiceAvailability[detail.space.demo == true ? "general" : channel.id]
    }

    public func refreshVoiceAvailability(channel: Channel? = nil) async {
        guard let detail,
              let channel = channel ?? detail.channels.first(where: { $0.id == selectedChannelID }) else { return }
        let root = detail.space.demo == true ? "general" : channel.id
        let accountGeneration = generation
        let enabled = (try? await api.mediaStatus(channelID: root == "general" ? nil : root)) ?? false
        guard !Task.isCancelled, generation == accountGeneration, self.detail?.space.id == detail.space.id,
              self.detail?.channels.contains(where: { $0.id == channel.id }) == true else { return }
        voiceAvailability[root] = enabled
    }

    private var preparedVoice: [String: ContinuousClock.Instant] = [:]

    /// Web's `media.prepare` when a signed-in member approaches Join: the API
    /// keeps the session 8 s, so reissue at most every 4 s. The public demo
    /// creates on join; failures only mean an ordinary join.
    public func prepareVoiceJoin(channel: Channel) {
        guard account != nil, let detail, detail.space.demo != true, channel.joined, voiceAvailable(in: channel) == true,
              detail.channels.contains(where: { $0.id == channel.id && $0.joined }),
              voice.context?.channelID != channel.id || voice.phase == .idle || voice.phase == .failed else { return }
        let now = ContinuousClock.now
        if let sent = preparedVoice[channel.id], sent.duration(to: now) < .seconds(4) { return }
        preparedVoice[channel.id] = now
        Task { try? await api.media(channelID: channel.id, operation: "prepare", body: [String: String]()) }
    }

    /// Verify at click time; a denied switch must not evict a healthy call.
    public func joinVoice(channel: Channel) async {
        guard pendingVoiceChannelID == nil, voice.phase != .joining, voice.phase != .reconnecting,
              voice.phase != .leaving, let detail, channel.joined, detail.channels.contains(where: { $0.id == channel.id && $0.joined }),
              voice.context?.channelID != channel.id || voice.phase == .idle || voice.phase == .failed else { return }
        voiceJoinGeneration += 1
        let joinAttempt = voiceJoinGeneration
        let clicked = Int64(Date().timeIntervalSince1970 * 1_000)
        pendingVoiceStartedAt = clicked
        pendingVoiceChannelID = channel.id
        defer { if voiceJoinGeneration == joinAttempt { pendingVoiceChannelID = nil } }
        let accountGeneration = generation
        let space = detail.space
        let ownerID = account?.id
        do {
            if space.demo != true {
                let verified = try await api.space(space.id)
                guard voiceJoinGeneration == joinAttempt, generation == accountGeneration else { return }
                guard verified.space.id == space.id, verified.channels.contains(where: { $0.id == channel.id && $0.joined }) else {
                    voicePresence.revoke(channelID: channel.id)
                    return
                }
                let history = try await api.history(channelID: channel.id)
                guard voiceJoinGeneration == joinAttempt, generation == accountGeneration else { return }
                guard history.space?.id == space.id, history.channel?.id == channel.id else {
                    voicePresence.revoke(channelID: channel.id)
                    return
                }
            }
            guard voiceJoinGeneration == joinAttempt, generation == accountGeneration,
                  account?.id == ownerID, self.detail?.space.id == space.id,
                  self.detail?.channels.contains(where: { $0.id == channel.id && $0.joined }) == true else { return }
            if voice.phase != .idle && voice.phase != .failed { voice.leaveImmediately() }
            await voice.join(channelID: space.demo == true ? nil : channel.id,
                             context: VoiceContext(channelID: channel.id, channelName: channel.name,
                                                   spaceID: space.id, spaceName: space.name),
                             name: account?.displayName ?? "Guest", joinStartedAt: clicked,
                             sessionStartedAt: voicePresence.sessionStartedAt(for: channel.id))
        } catch {
            guard voiceJoinGeneration == joinAttempt, generation == accountGeneration else { return }
            voicePresence.revoke(channelID: channel.id)
            navigationError = error.localizedDescription
        }
    }

    /// Web's sign-in copy; raw server messages are not shown.
    nonisolated static func loginFailure(_ error: Error) -> UserFacingError {
        guard let api = error as? APIError else { return UserFacingError(message: "Something went wrong. Please try again.") }
        switch api.status {
        case 401 where api.attemptsRemaining == 0: return UserFacingError(message: "That code can no longer be used. Request a new one.")
        case 401: return UserFacingError(message: "That code is incorrect or expired. Request a new one if needed.")
        case 400: return UserFacingError(message: "Enter a valid email address.")
        case 503: return UserFacingError(message: "Sign-in is temporarily unavailable. Please try again later.")
        default: return UserFacingError(message: "Something went wrong. Please try again.")
        }
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

struct UserFacingError: LocalizedError, Equatable {
    let message: String
    var errorDescription: String? { message }
}

/// Who reacted with one emoji: names once loaded, otherwise the chip's
/// count-only summary (with Retry in the sheet after a failure).
public enum ReactorsState: Equatable, Sendable {
    case loading
    case loaded([ReactorPerson])
    case failed
}

@MainActor @Observable
public final class ChatModel {
    public var messages: [ChatMessage] = []
    public var pinnedMessages: [ChatMessage] = []
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
    public var reactionErrors: [String: String] = [:]
    public var pinErrors: [String: String] = [:]
    public var pendingPins: Set<String> = []
    public var threadRootID: String?
    public var threadLoading = false
    public var threadHasMore = false
    public var threadError: String?
    private var threadBefore: String?
    private var threadRequest = 0
    private var threadOnlyRows: Set<String> = []
    private var threadDrafts: [String: String] = [:]
    private var threadBroadcasts: [String: Bool] = [:]
    public var channelMessages: [ChatMessage] { messages.filter { $0.isChannelMessage && !threadOnlyRows.contains($0.id) } }
    public var threadDraft: String {
        get { threadRootID.flatMap { threadDrafts[$0] } ?? "" }
        set { if let threadRootID { threadDrafts[threadRootID] = newValue } }
    }
    public var threadBroadcast: Bool {
        get { threadRootID.flatMap { threadBroadcasts[$0] } ?? false }
        set { if let threadRootID { threadBroadcasts[threadRootID] = newValue } }
    }
    public var currentAuthor: ChatAuthor? { session?.author }
    public private(set) var isPreview = false
    /// Changes when account/channel access or its generation changes.
    var editingContext: String { "\(generation):\(channelID ?? ""):\(currentAuthor?.id ?? ""):\(isPreview)" }
    /// Web's failed first load: no conversation to show, only the error.
    public private(set) var loadFailed = false
    /// Web's session error: history loaded but sending needs a new chat session.
    public private(set) var sessionError: String?
    public var pendingMessage: PendingMessage? { delivery.pending }
    public var sendRejected: Bool { delivery.rejected }
    @ObservationIgnored public var onAccessRevoked: ((String?) -> Void)?
    @ObservationIgnored public var onReadCursor: (() -> Void)?
    private let api: APIClient
    private var channelID: String?
    private var spaceID: String?
    private var session: ChatSession?
    private var subscriptionID: String?
    private var generation = 0
    private var delivery = ChatDeliveryState()
    private var reactionSnapshots = ReactionSnapshots()
    private var pinSnapshots = PinSnapshots()
    private var editSnapshots = EditSnapshots()
    private var failedPinActions: [String: Bool] = [:]
    private struct PendingReaction: Equatable {
        let active: Bool
        let intent: Int
    }
    private var pendingReactions: [String: [String: PendingReaction]] = [:]
    private var reactionWorkers: Set<String> = []
    private var nextReactionIntent = 0
    private var failedReactions: [String: (emoji: String, active: Bool)] = [:]
    /// Who reacted, per message: the last list with the reaction sequence it
    /// was requested at, the request in flight, and failed loads.
    private var reactorCache: [String: (seq: String, list: ReactorList)] = [:]
    private var reactorRequests: [String: (seq: String, token: Int)] = [:]
    private var reactorFailures: Set<String> = []
    private var nextReactorRequest = 0
    private var typers: [String: (author: ChatAuthor, typing: Bool, revision: String, expires: Date)] = [:]
    private var typingActive = false
    private var typingSent = false
    private var typingSentAt = Date.distantPast
    private var typingTask: Task<Void, Never>?
    private var typingIdleTask: Task<Void, Never>?
    private var typingExpiryTask: Task<Void, Never>?
    @ObservationIgnored private lazy var gateway: Gateway = Gateway(baseURL: api.baseURL, token: { [api] in await api.authorizationToken() }) { [weak self] state, error in
        self?.receiveGatewayState(state, error: error)
    }

    public init(api: APIClient) { self.api = api }

    func canEdit(_ message: ChatMessage) -> Bool {
        !isPreview && channelID == message.channelId && currentAuthor?.isGuest == false && currentAuthor?.id == message.author.id
    }

    func editMessage(_ message: ChatMessage, text: String) async throws {
        guard canEdit(message), let session else { throw UserFacingError(message: "Only the author can edit while participating.") }
        let request = generation
        let result = try await api.editMessage(channelID: message.channelId, messageID: message.id, sessionToken: session.token, text: text, expectedRevision: message.revision ?? 1)
        guard request == generation, canEdit(message) else { throw CancellationError() }
        guard result.author.id == session.author.id else { throw UserFacingError(message: "Message author mismatch.") }
        if !applyEditSnapshot(result) { requestResync(generation: request, channelID: message.channelId) }
    }

    func reloadMessage(_ message: ChatMessage) async throws -> ChatMessage {
        let request = generation
        let result = try await api.loadMessage(channelID: message.channelId, messageID: message.id)
        guard request == generation, channelID == message.channelId else { throw CancellationError() }
        if !applyEditSnapshot(result) { requestResync(generation: request, channelID: message.channelId) }
        return result
    }

    func messageVersions(_ message: ChatMessage, before: Int? = nil) async throws -> MessageVersions {
        let request = generation
        let result = try await api.messageVersions(channelID: message.channelId, messageID: message.id, before: before)
        guard request == generation, channelID == message.channelId else { throw CancellationError() }
        return result
    }

    /// HTTP snapshots never advance delivery/read cursors, insert rows or chime.
    private func applyEditSnapshot(_ message: ChatMessage) -> Bool {
        editSnapshots.seed(messages + pinnedMessages)
        editSnapshots.apply(message)
        guard !editSnapshots.unseenOverflowed else { return false }
        messages = messages.map { editSnapshots.overlay($0) }
        pinnedMessages = pinnedMessages.map { editSnapshots.overlay($0) }
        return true
    }

    func receiveGatewayState(_ state: GatewayState, error: String?) {
        // Actor callbacks queued before unsubscribe must not revive stopped chat.
        guard channelID != nil else { return }
        liveState = state
        // Transport interruptions are represented by liveState and the view's
        // delayed connection indicator. `error` is reserved for durable
        // history, session, subscription and send failures. Gateway reports a
        // rejected subscription with connected state, not reconnecting.
        if state == .connected, let error { self.error = error }
    }

    func updateAuthor(account: Account) {
        guard let session, !session.author.isGuest, session.author.id == account.id,
              let name = account.displayName else { return }
        // Account-backed sends resolve the current name server-side. Updating
        // this presentation snapshot must not reopen chat or discard its draft.
        self.session = ChatSession(token: session.token, author: ChatAuthor(id: account.id, name: name, isGuest: false, avatarId: account.avatarId))
    }

    /// A token-free copy of only the timeline currently retained by this model.
    public func currentSnapshot() -> ChatHistory? {
        guard session != nil, let channelID, let spaceID else { return nil }
        return ChatHistory(
            space: HistoryIdentity(id: spaceID, name: spaceName),
            channel: HistoryIdentity(id: channelID, name: channelName),
            messages: messages.map { pinSnapshots.overlay(reactionSnapshots.overlay($0)) },
            pinnedMessages: pinnedMessages.map { pinSnapshots.overlay($0) },
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

    public func preview(history: ChatHistory) async {
        lastOpen = nil
        generation += 1
        let requestGeneration = generation
        let oldSubscription = subscriptionID
        subscriptionID = nil
        clearLocal(preservingPending: false)
        isPreview = true
        if let oldSubscription { await gateway.unsubscribe(oldSubscription) }
        guard generation == requestGeneration else { return }
        channelID = history.channel?.id; spaceID = history.space?.id
        reactionSnapshots.seed(history.messages)
        pinSnapshots.replace(history.messages + history.pinnedMessages, cursor: history.cursor)
        editSnapshots.seed(history.messages + history.pinnedMessages)
        messages = history.messages.map { pinSnapshots.overlay(reactionSnapshots.overlay($0)) }
        pinnedMessages = history.pinnedMessages.map { pinSnapshots.overlay($0) }
        delivery.reset(cursor: history.cursor); hasMore = history.hasMore
        channelName = history.channel?.name ?? "general"; spaceName = history.space?.name ?? "Caper"
        session = nil; loading = false; loadFailed = false; sessionError = nil
        guard let actualChannel = channelID else { return }
        let subscription = await gateway.subscribeChat(channelID: actualChannel, after: delivery.cursor) { [weak self] event in
            self?.receive(event, generation: requestGeneration, channelID: actualChannel)
        }
        guard generation == requestGeneration, channelID == actualChannel else {
            await gateway.unsubscribe(subscription)
            return
        }
        subscriptionID = subscription
    }

    private var lastOpen: (channelID: String?, displayName: String)?

    /// Web's "Try again" after a failed first load.
    public func retryLoad() async {
        guard let lastOpen else { return }
        await open(channelID: lastOpen.channelID, displayName: lastOpen.displayName, preservingPending: channelID == lastOpen.channelID, prepared: nil)
    }

    private func open(channelID: String?, displayName: String, preservingPending: Bool, prepared: ChatHistory?) async {
        lastOpen = (channelID, displayName)
        isPreview = false
        let preservingTimeline = preservingPending && self.channelID == channelID
        generation += 1
        let requestGeneration = generation
        let oldSubscription = subscriptionID
        subscriptionID = nil
        let preservedDraft = draft
        let preservedMessages = channelMessages
        let preservedCursor = delivery.cursor
        let preservedHasMore = hasMore
        if preservingTimeline {
            pendingReactions = [:]; reactionWorkers = []
            renderReactions()
            typingTask?.cancel(); typingIdleTask?.cancel(); typingExpiryTask?.cancel()
            typingTask = nil; typingIdleTask = nil; typingExpiryTask = nil
            typers = [:]; typingNames = []; typingActive = false; typingSent = false
            loadingOlder = false; olderError = nil; liveState = .disconnected
        } else {
            clearLocal(preservingPending: preservingPending)
        }
        if preservingPending { draft = preservedDraft }
        // Prepared history is already loaded and authorized by navigation.
        // Show it immediately; obtaining a sending capability is not a new
        // history load (and must not flash the previous channel's title).
        if let prepared {
            self.channelID = prepared.channel?.id; spaceID = prepared.space?.id
            reactionSnapshots.seed(prepared.messages)
            pinSnapshots.replace(prepared.messages + prepared.pinnedMessages, cursor: prepared.cursor)
            editSnapshots.seed(messages + pinnedMessages + prepared.messages + prepared.pinnedMessages)
            messages = prepared.messages.map { editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0))) }
            pinnedMessages = prepared.pinnedMessages.map { editSnapshots.overlay(pinSnapshots.overlay($0)) }
            delivery.reset(cursor: prepared.cursor); hasMore = prepared.hasMore
            channelName = prepared.channel?.name ?? "general"
            spaceName = prepared.space?.name ?? "Caper"
        }
        loading = prepared == nil
        if let oldSubscription { await gateway.unsubscribe(oldSubscription) }
        guard generation == requestGeneration else { return }
        self.channelID = channelID
        loading = prepared == nil; error = nil; loadFailed = false; sessionError = nil
        do {
            async let sessionRequest = api.chatSession(name: displayName)
            let history: ChatHistory
            if let prepared { history = prepared }
            else { history = try await api.history(channelID: channelID) }
            // Web keeps the conversation when only the session fails, with Retry session.
            let chatSession: ChatSession?
            do { chatSession = try await sessionRequest } catch {
                if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) { throw error }
                chatSession = nil
                sessionError = error.localizedDescription
            }
            guard self.channelID == channelID, generation == requestGeneration else { return }
            let resolvedChannelID = history.channel?.id
            spaceID = history.space?.id
            self.channelID = resolvedChannelID
            let firstRefreshed = history.messages.first?.seq
            let canRetain = preservingTimeline && Self.refreshAccountsForMissingEvents(
                messages: history.messages,
                after: preservedCursor,
                through: history.cursor
            )
            if prepared == nil {
                // The latest history response is authoritative for the channel-wide
                // pin list. Older pagination responses are deliberately ignored.
                pinSnapshots.replace(history.messages + history.pinnedMessages, cursor: history.cursor)
                // Overflow requires fresh pages; never keep the resync flag latched.
                if editSnapshots.unseenOverflowed { editSnapshots.reset() }
                editSnapshots.seed(messages + pinnedMessages + history.messages + history.pinnedMessages)
                let candidates = Dictionary((pinnedMessages + history.pinnedMessages).map { ($0.id, $0) }, uniquingKeysWith: { _, next in next })
                pinnedMessages = candidates.values.map { editSnapshots.overlay(pinSnapshots.overlay($0)) }.filter { $0.pin != nil }
                    .sorted { (try? Sequence.compare($0.pinSeq ?? "0", $1.pinSeq ?? "0")) == .orderedDescending }
                messages = channelMessages
                threadOnlyRows = []
                if canRetain {
                    // Include HTTP confirmations received while refresh was pending.
                    merge(history.messages) // The refreshed representation wins overlapping IDs.
                } else {
                    // Preserve a newer reaction revision on overlapping rows, but
                    // discard snapshots for rows no longer in the fresh window.
                    reactionSnapshots.seed(history.messages)
                    messages = history.messages.map { editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0))) }
                    reactionSnapshots.reset()
                    reactionSnapshots.seed(messages)
                }
                delivery.reset(cursor: history.cursor, preservingPending: preservingPending)
                let retainedOlderPrefix = canRetain && firstRefreshed.map { first in
                    preservedMessages.contains { (try? Sequence.compare($0.seq, first)) == .orderedAscending }
                } == true
                hasMore = retainedOlderPrefix ? preservedHasMore : history.hasMore
            }
            channelName = history.channel?.name ?? "general"
            spaceName = history.space?.name ?? "Caper"
            session = chatSession
            if CaperRuntime.isChatPreview("reaction-chips"), let ownID = chatSession?.author.id, !messages.isEmpty {
                let fixtureEmoji = ["👍", "❤️", "😂", "🎉", "🚀", "👀", "🔥", "✅", "👏", "🤔", "💯", "🙌", "😄", "🥳", "🤝", "✨", "💚", "😮", "🤯", "👩‍💻"]
                messages[0].reactions = fixtureEmoji.enumerated().map {
                    MessageReaction(emoji: $0.element, authorIds: $0.offset == 0 ? [ownID, "fixture-other"] : ["fixture-other"])
                }
                messages[0].reactionSeq = history.cursor
                if messages.count > 1 {
                    messages[1].reactions = []
                } else {
                    let source = messages[0]
                    messages.append(ChatMessage(id: "reaction-empty-fixture", channelId: source.channelId, seq: source.seq,
                                                author: source.author, content: ChatContent(version: 1, type: "text", text: "No reactions yet"),
                                                createdAt: source.createdAt, clientMessageId: "reaction-empty-fixture", reactions: [],
                                                reactionSeq: history.cursor))
                }
                reactionSnapshots.seed(messages)
            }
            guard let actualChannel = resolvedChannelID else { throw APIError(status: 502, message: "Channel metadata is missing.") }
            let newSubscription = await gateway.subscribeChat(channelID: actualChannel, after: delivery.cursor) { [weak self] event in
                self?.receive(event, generation: requestGeneration, channelID: actualChannel)
            }
            guard generation == requestGeneration, self.channelID == actualChannel else {
                await gateway.unsubscribe(newSubscription)
                return
            }
            subscriptionID = newSubscription
            onReadCursor?()
            if threadRootID != nil { await loadThread() }
            if CaperRuntime.isChatPreview("chat-rejected") {
                let preview = delivery.begin(text: "Fixture message that was rejected")
                delivery.reject(id: preview.id)
                error = "Fixture rejection; no message was sent."
            }
        } catch {
            guard generation == requestGeneration else { return }
            if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) {
                clearLocal()
                onAccessRevoked?(channelID)
            }
            self.error = error.localizedDescription
            loadFailed = messages.isEmpty
        }
        if generation == requestGeneration { loading = false }
    }

    private static func refreshAccountsForMissingEvents(messages: [ChatMessage], after oldCursor: String, through newCursor: String) -> Bool {
        guard !messages.isEmpty, let cursorOrder = try? Sequence.compare(newCursor, oldCursor) else { return false }
        if cursorOrder == .orderedSame { return true }
        guard cursorOrder == .orderedDescending else { return false }

        let freshSequences = messages.map(\.seq).filter {
            (try? Sequence.compare($0, oldCursor)) == .orderedDescending &&
                (try? Sequence.compare($0, newCursor)) != .orderedDescending
        }.sorted { (try? Sequence.compare($0, $1)) == .orderedAscending }
        var accountedCursor = oldCursor
        for sequence in freshSequences {
            guard Sequence.isSuccessor(sequence, of: accountedCursor) else { return false }
            accountedCursor = sequence
        }
        return accountedCursor == newCursor
    }

    /// Web's Retry session.
    public func retrySession() async {
        guard let lastOpen, session == nil else { return }
        let requestGeneration = generation
        do {
            let chatSession = try await api.chatSession(name: lastOpen.displayName)
            guard generation == requestGeneration else { return }
            session = chatSession; sessionError = nil
        } catch {
            guard generation == requestGeneration else { return }
            sessionError = error.localizedDescription
        }
    }

    public func loadOlder() async {
        guard !loading, !loadingOlder, hasMore, let channelID, let before = channelMessages.first?.seq else { return }
        let requestGeneration = generation
        loadingOlder = true; olderError = nil
        defer { if generation == requestGeneration { loadingOlder = false } }
        do {
            let page = try await api.history(channelID: channelID, before: before)
            guard generation == requestGeneration, self.channelID == channelID else { return }
            threadOnlyRows.subtract(page.messages.map(\.id))
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

    public func closeThread() { threadRequest += 1; threadRootID = nil; threadLoading = false }

    public func openThread(_ rootID: String) async {
        threadRootID = rootID; threadBefore = nil; threadHasMore = false
        await loadThread()
    }

    public func loadThread(older: Bool = false) async {
        guard let rootID = threadRootID, let channelID else { return }
        threadRequest += 1
        let request = threadRequest; let channelGeneration = generation
        threadLoading = true; threadError = nil
        do {
            let page = try await api.thread(channelID: channelID, rootID: rootID, before: older ? threadBefore : nil)
            guard request == threadRequest, generation == channelGeneration, threadRootID == rootID else { return }
            let loaded = Set(messages.map(\.id))
            let rows = [page.root] + page.messages
            threadOnlyRows.formUnion(rows.filter { $0.isChannelMessage && !loaded.contains($0.id) }.map(\.id))
            merge(rows)
            threadHasMore = page.hasMore; threadBefore = page.messages.first?.seq ?? threadBefore
        } catch {
            guard request == threadRequest, generation == channelGeneration else { return }
            if let denied = error as? APIError, [401, 403, 404].contains(denied.status) {
                closeThread(); await retryLoad(); return
            }
            threadError = error.localizedDescription
        }
        if request == threadRequest { threadLoading = false }
    }

    public func send(inThread: Bool = false) async {
        guard !sending, !delivery.rejected else { return }
        guard let channelID, let session else { error = "Messaging session is unavailable."; return }
        let rootID = inThread ? threadRootID : nil
        guard !inThread || rootID != nil else { return }
        if let pending = delivery.pending, pending.threadRootId != rootID { return }
        let text = inThread ? threadDraft : draft
        if delivery.pending == nil, let validation = MessageValidation.error(for: text) { error = validation; return }
        let newSubmission = delivery.pending == nil
        let command = delivery.begin(text: text, threadRootId: rootID, broadcast: inThread && threadBroadcast)
        if newSubmission { if inThread { threadDraft = "" } else { draft = "" } }
        let requestGeneration = generation
        sending = true; error = nil
        await gateway.reportActivity()
        do {
            let message = try await api.send(channelID: channelID, sessionToken: session.token, clientMessageID: command.id, text: command.text, threadRootId: command.threadRootId, broadcast: command.broadcast)
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

    public func setReaction(messageID: String, emoji: String, active: Bool) async {
        guard !isPreview, let channelID, session != nil else { return }
        let requestGeneration = generation
        nextReactionIntent += 1
        pendingReactions[messageID, default: [:]][emoji] = PendingReaction(active: active, intent: nextReactionIntent)
        reactionErrors[messageID] = nil
        failedReactions[messageID] = nil
        renderReactions()
        guard !reactionWorkers.contains(messageID) else { return }
        reactionWorkers.insert(messageID)
        await savePendingReactions(messageID: messageID, channelID: channelID, generation: requestGeneration)
    }

    public func retryReaction(messageID: String) async {
        guard let failed = failedReactions[messageID] else { return }
        await setReaction(messageID: messageID, emoji: failed.emoji, active: failed.active)
    }

    /// What a chip's tooltip or the who-reacted sheet can show for `emoji`
    /// on `message` as currently displayed (including pending own changes).
    public func reactorsState(for message: ChatMessage, emoji: String, viewerID: String?) -> ReactorsState {
        let cached = reactorCache[message.id]
        if let list = cached?.list {
            guard let reaction = message.reactions?.first(where: { $0.emoji == emoji }) else { return .loaded([]) }
            if let people = list.people(for: reaction, viewer: reactorViewer(viewerID)) { return .loaded(people) }
        }
        if reactorRequests[message.id] != nil { return .loading }
        if reactorFailures.contains(message.id) { return .failed }
        // Loaded at this revision yet someone is still unnamed: offer Retry
        // instead of waiting. A stale list is about to be refetched.
        if let cached, Self.reactorListIsCurrent(cached, for: message) { return .failed }
        return .loading
    }

    private static func reactorListIsCurrent(_ cached: (seq: String, list: ReactorList), for message: ChatMessage) -> Bool {
        let seq = message.reactionSeq ?? "0"
        return cached.seq == seq || cached.list.reactionSeq == seq
    }

    /// Loads who reacted unless the cached list was requested at the message's
    /// current reaction sequence and still names everyone on it. Uses account
    /// authorization only, so read-only previews work too.
    @discardableResult
    public func requestReactors(messageID: String, force: Bool = false) -> Task<Void, Never>? {
        guard let channelID, let message = messages.first(where: { $0.id == messageID }) else { return nil }
        let seq = message.reactionSeq ?? "0"
        if !force, let cached = reactorCache[messageID], Self.reactorListIsCurrent(cached, for: message),
           (message.reactions ?? []).allSatisfy({ cached.list.people(for: $0, viewer: reactorViewer(session?.author.id)) != nil }) {
            return nil
        }
        if reactorRequests[messageID]?.seq == seq { return nil }
        nextReactorRequest += 1
        let token = nextReactorRequest
        reactorRequests[messageID] = (seq, token)
        reactorFailures.remove(messageID)
        let requestGeneration = generation
        let api = self.api
        // Parity fixture chips carry local-only authors the fixture server
        // cannot name, so the fixture answers locally without a request.
        let fixture = CaperRuntime.isChatPreview("reaction-chips") ? fixtureReactors(for: message) : nil
        return Task { [weak self] in
            let result: Result<ReactorList, Error>
            if let fixture { result = .success(fixture) }
            else {
                do { result = .success(try await api.reactors(channelID: channelID, messageID: messageID)) }
                catch { result = .failure(error) }
            }
            self?.finishReactors(messageID: messageID, channelID: channelID, seq: seq, token: token,
                                 generation: requestGeneration, result: result)
        }
    }

    private func finishReactors(messageID: String, channelID: String, seq: String, token: Int,
                                generation requestGeneration: Int, result: Result<ReactorList, Error>) {
        guard reactorRequests[messageID]?.token == token else { return }
        reactorRequests[messageID] = nil
        guard generation == requestGeneration, self.channelID == channelID else { return }
        switch result {
        case .success(let list):
            // A slower response for an older revision must not replace a newer list.
            if let cached = reactorCache[messageID]?.list,
               (try? Sequence.compare(cached.reactionSeq, list.reactionSeq)) == .orderedDescending { return }
            reactorCache[messageID] = (seq, list)
        case .failure:
            reactorFailures.insert(messageID)
        }
    }

    /// The viewer, named from the chat session when it is theirs, so an own
    /// reaction not yet in the fetched list still reads as "You".
    private func reactorViewer(_ id: String?) -> ReactorPerson? {
        guard let id else { return nil }
        guard let author = session?.author, author.id == id else { return ReactorPerson(id: id) }
        return ReactorPerson(id: id, displayName: author.name, avatarId: author.avatarId)
    }

    private func fixtureReactors(for message: ChatMessage) -> ReactorList {
        let own = session?.author
        let groups: [ReactorGroup] = (message.reactions ?? []).map { reaction -> ReactorGroup in
            ReactorGroup(emoji: reaction.emoji, authors: reaction.authorIds.map { id -> ReactorPerson in
                if let own, id == own.id { return ReactorPerson(id: id, displayName: own.name, avatarId: own.avatarId) }
                if id == "fixture-other" {
                    return ReactorPerson(id: id, username: "fixture_other", displayName: "TEST FIXTURE Other", avatarId: 31)
                }
                return ReactorPerson(id: id)
            })
        }
        return ReactorList(messageId: message.id, reactionSeq: message.reactionSeq ?? "0", reactions: groups)
    }

    public func setPin(messageID: String, active: Bool) async {
        guard !isPreview, let channelID, let session, !pendingPins.contains(messageID) else { return }
        pendingPins.insert(messageID); pinErrors[messageID] = nil
        failedPinActions[messageID] = nil
        let requestGeneration = generation
        defer { if generation == requestGeneration { pendingPins.remove(messageID) } }
        do {
            let event = try await api.setPin(channelID: channelID, messageID: messageID, sessionToken: session.token, active: active)
            guard generation == requestGeneration, self.channelID == channelID else { return }
            applyPin(event.message)
        } catch {
            guard generation == requestGeneration, self.channelID == channelID else { return }
            if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) {
                onAccessRevoked?(channelID); await stop(); return
            }
            failedPinActions[messageID] = active
            pinErrors[messageID] = "Couldn’t \(active ? "pin" : "unpin") message. Try again."
        }
    }

    public func retryPin(messageID: String) async {
        guard let active = failedPinActions[messageID] else { return }
        await setPin(messageID: messageID, active: active)
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
            clearLocal(); error = event["error"] as? String ?? "Channel access ended."
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
        if type == "message.reactions" {
            guard let data = try? JSONSerialization.data(withJSONObject: event),
                  let reactionEvent = try? JSONDecoder().decode(MessageReactionsEvent.self, from: data),
                  reactionEvent.isValid,
                  let seq = ReactionEvent.sequence(event, channelID: eventChannelID),
                  delivery.receive(seq: seq) else {
                requestResync(generation: eventGeneration, channelID: eventChannelID)
                return
            }
            applyReactions(reactionEvent)
            onReadCursor?()
            if reactionSnapshots.unseenOverflowed {
                requestResync(generation: eventGeneration, channelID: eventChannelID)
                return
            }
            if let subscriptionID {
                let cursor = delivery.cursor
                Task { await gateway.updateCursor(subscription: subscriptionID, after: cursor) }
            }
            return
        }
        if type == "message.edited" {
            guard let message = EditEvent.message(event, channelID: eventChannelID),
                  let seq = message.editSeq, delivery.receive(seq: seq) else {
                requestResync(generation: eventGeneration, channelID: eventChannelID); return
            }
            if !applyEditSnapshot(message) {
                requestResync(generation: eventGeneration, channelID: eventChannelID); return
            }
            onReadCursor?()
            if let subscriptionID {
                let cursor = delivery.cursor
                Task { await gateway.updateCursor(subscription: subscriptionID, after: cursor) }
            }
            return
        }
        if type == "message.pin" {
            guard let data = try? JSONSerialization.data(withJSONObject: event),
                  let pinEvent = try? JSONDecoder().decode(MessagePinEvent.self, from: data), pinEvent.isValid,
                  let seq = PinEvent.sequence(event, channelID: eventChannelID), delivery.receive(seq: seq) else {
                requestResync(generation: eventGeneration, channelID: eventChannelID); return
            }
            applyPin(pinEvent.message)
            onReadCursor?()
            if let subscriptionID {
                let cursor = delivery.cursor
                Task { await gateway.updateCursor(subscription: subscriptionID, after: cursor) }
            }
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
                onReadCursor?()
                if newMessage, let author = session?.author, author.id != message.author.id { CaperEffects.shared.play(.message) }
                if message.threadRootId == delivery.pending?.threadRootId,
                   (message.broadcast ?? false) == (delivery.pending?.broadcast ?? false),
                   delivery.confirmGateway(clientMessageID: message.clientMessageId, authorID: message.author.id, ownAuthorID: session?.author.id) {
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
        let preview = isPreview
        let name = session?.author.name ?? "Guest"
        error = "Messages changed while reconnecting. Refreshing…"
        Task { [weak self] in
            guard let self, self.generation == expectedGeneration, self.channelID == expectedChannelID else { return }
            if preview {
                do {
                    let history = try await self.api.history(channelID: expectedChannelID)
                    guard self.generation == expectedGeneration, self.channelID == expectedChannelID, self.isPreview else { return }
                    await self.preview(history: history)
                } catch {
                    guard self.generation == expectedGeneration, self.channelID == expectedChannelID, self.isPreview else { return }
                    self.error = error.localizedDescription
                }
            } else {
                await self.open(channelID: expectedChannelID, displayName: name, preservingPending: true, prepared: nil)
            }
        }
    }

    private func clearLocal(preservingPending: Bool = false) {
        typingTask?.cancel(); typingIdleTask?.cancel(); typingExpiryTask?.cancel()
        typingTask = nil; typingIdleTask = nil; typingExpiryTask = nil
        typers = [:]; typingNames = []; typingActive = false; typingSent = false
        delivery.reset(preservingPending: preservingPending)
        reactionSnapshots.reset(); pendingReactions = [:]; reactionWorkers = []; reactionErrors = [:]; failedReactions = [:]
        reactorCache = [:]; reactorRequests = [:]; reactorFailures = []
        pinSnapshots.reset(); pinnedMessages = []; pendingPins = []; pinErrors = [:]
        editSnapshots.reset()
        failedPinActions = [:]
        closeThread(); threadOnlyRows = []; threadDrafts = [:]; threadBroadcasts = [:]
        isPreview = false
        session = nil; channelID = nil; spaceID = nil; messages = []; draft = ""; hasMore = false
        channelName = "general"; spaceName = "Caper"; error = nil
        loadingOlder = false; olderError = nil
        loading = false; sending = false; liveState = .disconnected
    }

    private func merge(_ incoming: [ChatMessage]) {
        reactionSnapshots.seed(incoming)
        pinSnapshots.seed(incoming)
        editSnapshots.seed(messages + pinnedMessages + incoming)
        var byID = Dictionary(uniqueKeysWithValues: messages.map { ($0.id, $0) })
        var summaries: [String: ThreadSummary] = [:]
        for message in messages + incoming {
            guard let summary = message.thread else { continue }
            let root = message.threadRootId ?? message.id
            if summaries[root] == nil || (try? Sequence.compare(summary.seq, summaries[root]!.seq)) == .orderedDescending { summaries[root] = summary }
        }
        incoming.forEach { byID[$0.id] = editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0))) }
        byID = byID.mapValues { message in
            var updated = editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay(message)))
            updated.thread = summaries[message.threadRootId ?? message.id] ?? message.thread
            return updated
        }
        messages = byID.values.sorted { (try? Sequence.compare($0.seq, $1.seq)) == .orderedAscending }
        renderReactions()
    }

    private func applyPin(_ message: ChatMessage) {
        editSnapshots.seed(messages + pinnedMessages + [message])
        guard pinSnapshots.apply(message) else { return }
        messages = messages.map { editSnapshots.overlay(pinSnapshots.overlay($0)) }
        pinnedMessages.removeAll { $0.id == message.id }
        let updated = editSnapshots.overlay(pinSnapshots.overlay(message))
        if updated.pin != nil {
            pinnedMessages.append(updated)
            pinnedMessages.sort { (try? Sequence.compare($0.pinSeq ?? "0", $1.pinSeq ?? "0")) == .orderedDescending }
        }
    }

    private func applyReactions(_ event: MessageReactionsEvent) {
        guard reactionSnapshots.apply(messageID: event.messageId, seq: event.seq, reactions: event.reactions) else { return }
        renderReactions()
    }

    private func renderReactions() {
        let authorID = session?.author.id
        messages = messages.map { message in
            var result = reactionSnapshots.overlay(message)
            guard let authorID, let pending = pendingReactions[message.id], !pending.isEmpty else { return result }
            var reactions = result.reactions ?? []
            for (emoji, intent) in pending {
                if let index = reactions.firstIndex(where: { $0.emoji == emoji }) {
                    var authors = reactions[index].authorIds.filter { $0 != authorID }
                    if intent.active { authors.append(authorID) }
                    if authors.isEmpty { reactions.remove(at: index) }
                    else { reactions[index] = MessageReaction(emoji: emoji, authorIds: authors) }
                } else if intent.active {
                    reactions.append(MessageReaction(emoji: emoji, authorIds: [authorID]))
                }
            }
            result.reactions = reactions
            return result
        }
    }

    private func savePendingReactions(messageID: String, channelID: String, generation requestGeneration: Int) async {
        defer { if generation == requestGeneration { reactionWorkers.remove(messageID) } }
        while generation == requestGeneration, self.channelID == channelID,
              let session, let pending = pendingReactions[messageID],
              let (emoji, desired) = pending.min(by: { $0.value.intent < $1.value.intent }) {
            do {
                let event = try await api.setReaction(channelID: channelID, messageID: messageID,
                                                      sessionToken: session.token, emoji: emoji, active: desired.active)
                guard generation == requestGeneration, self.channelID == channelID else { return }
                _ = reactionSnapshots.apply(messageID: event.messageId, seq: event.seq, reactions: event.reactions)
                if pendingReactions[messageID]?[emoji] == desired {
                    pendingReactions[messageID]?[emoji] = nil
                }
                if pendingReactions[messageID]?.isEmpty == true { pendingReactions[messageID] = nil }
                renderReactions()
            } catch {
                guard generation == requestGeneration, self.channelID == channelID else { return }
                if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) {
                    onAccessRevoked?(channelID)
                    await stop()
                    return
                }
                if pendingReactions[messageID]?[emoji] == desired {
                    pendingReactions[messageID]?[emoji] = nil
                    if pendingReactions[messageID]?.isEmpty == true { pendingReactions[messageID] = nil }
                    reactionErrors[messageID] = "Couldn’t save reaction. Retry."
                    failedReactions[messageID] = (emoji, desired.active)
                    renderReactions()
                }
            }
        }
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
