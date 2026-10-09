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
    public var selectedChannelID: String? { didSet { if selectedChannelID != oldValue { removeShownNotifications() } } }
    public var directMessages: [DirectMessageConversation] = []
    public private(set) var directMessagesError: String?
    public var selectedDirectMessageID: String? { didSet { if selectedDirectMessageID != oldValue { removeShownNotifications() } } }
    /// `GET /api/people` for `@` suggestions in DMs; nil until the first load
    /// succeeds. A refresh keeps the previous list until it completes.
    public private(set) var people: [Person]?
    /// The main "Direct messages" list: accepted and outgoing conversations.
    public var visibleDirectMessages: [DirectMessageConversation] { MessageRequests.visible(directMessages) }
    /// Incoming message requests. They never count as unread or chime.
    public var messageRequests: [DirectMessageConversation] { MessageRequests.incoming(directMessages) }
    public var selectedDirectMessage: DirectMessageConversation? {
        selectedDirectMessageID.flatMap { id in directMessages.first { $0.id == id } }
    }
    /// The person's own open/closed choice for the requests list; nil until they
    /// toggle it. Web's `requestsOpen`.
    public var messageRequestsOpen: Bool?
    /// The requests list is open: a pushed page on narrow layouts, an expanded
    /// sidebar section otherwise. As on web, it follows the view (open while an
    /// incoming request is shown) until the person toggles it; their choice wins.
    public var showingMessageRequests: Bool {
        get { messageRequestsOpen ?? (selectedDirectMessage?.status == .incoming) }
        set { messageRequestsOpen = newValue }
    }
    /// Accounts you blocked, newest first, and their ids for every timeline.
    public private(set) var blockedAccounts: [BlockedAccount] = []
    public private(set) var blockedIDs: Set<String> = []
    public private(set) var blocksLoaded = false
    public var blocksError: String?
    /// "Who can start a DM with you"; nil until loaded.
    public private(set) var directMessagePrivacy: DirectMessagePrivacy?
    public var privacyError: String?
    public var pushAvailable = false
    public var pushEnabled = false
    @ObservationIgnored public var setPushEnabled: ((Bool) async -> Void)?
    @ObservationIgnored public var disablePushLocally: (() -> Void)?
    /// Removes a conversation's delivered pushes, by its channel or DM id (the
    /// APNs `thread-id`), once it is on screen in the foreground.
    @ObservationIgnored public var removeDeliveredNotifications: ((String) -> Void)?
    /// `GET /api/notifications/settings`; nil until the first load, so menus
    /// never show a guessed level.
    public private(set) var notificationSettings: NotificationSettings?
    /// A failed notification change, shown inline until dismissed or the next change.
    public var notificationError: String?
    /// The settings could not load; shown in Notifications settings.
    public private(set) var notificationsLoadError: String?
    /// Bumped when a timed mute ends, so muted rows redraw.
    public private(set) var muteExpiryTick = 0
    @ObservationIgnored private var notificationsLoadedAt: Date?
    @ObservationIgnored private var notificationRequest = 0
    @ObservationIgnored private var notificationChangesInFlight = 0
    @ObservationIgnored private var accountNotificationRevisions: [String: Int] = [:]
    @ObservationIgnored private var overrideRevisions: [NotificationScope: Int] = [:]
    @ObservationIgnored private var muteExpiryTask: Task<Void, Never>?
    /// A tapped push that arrived before spaces loaded; opened in their place.
    @ObservationIgnored private var pendingNotificationRoute: NotificationRoute?
    public var previewingChannel: Bool { selectedChannel?.joined == false }
    public var selectedChannel: Channel? { detail?.channels.first { $0.id == selectedChannelID } }
    public var error: String?
    public var busy = false
    public var challengeID: String? {
        didSet {
            if challengeID != oldValue { loginAttemptsRemaining = nil }
            // "Use a different email" (or signing out) starts the resend allowance over.
            if challengeID == nil { codesSent = 0; codeSentAt = nil }
        }
    }
    /// Remaining code attempts reported by the last rejected verification, as on web.
    public var loginAttemptsRemaining: Int?
    /// Codes emailed for the current email entry: the first plus up to
    /// `CodeResend.maximumResends` resends (see `CodeResend`).
    public private(set) var codesSent = 0
    /// When the latest code was emailed; Resend code waits a minute after each.
    public private(set) var codeSentAt: Date?
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
    #if os(macOS)
    @ObservationIgnored private lazy var desktopNotifications = DesktopNotifications(model: self)
    #endif
    /// Web's membershipRevision: a background reconcile never applies a
    /// snapshot taken while a space or channel change here was running.
    @ObservationIgnored private var membershipChanges = 0
    @ObservationIgnored private var membershipRevision = 0
    @ObservationIgnored private var membershipRefreshing = false

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
        chatModel.viewerAccountID = { [weak self] in self?.account?.id }
        chatModel.blockAccount = { [weak self] target in try await self?.block(target) }
        chatModel.unblockAccount = { [weak self] accountID in try await self?.unblock(accountID: accountID) }
        chatModel.onDirectMessageBlocked = { [weak self] in
            let refresh = Task { await self?.refreshBlocks(); await self?.refreshDirectMessages() }
            _ = refresh
        }
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
            // A push tapped while signed out never opens for whoever signs in next.
            if account == nil { pendingNotificationRoute = nil }
            if phase == .ready { await loadSpaces(); startDirectMessageRefresh() }
        } catch {
            guard generation == attempt else { return }
            self.error = FriendlyError.message(for: error); phase = .signedOut
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
            self.codesSent += 1
            self.codeSentAt = Date()
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
        directMessages = []; directMessagesError = nil; selectedDirectMessageID = nil; people = nil; messageRequestsOpen = nil
        blockedAccounts = []; blockedIDs = []; blocksLoaded = false; blocksError = nil
        directMessagePrivacy = nil; privacyError = nil; chat.setBlockedAuthors([])
        clearNotificationState()
        spacesLoaded = false; spacesError = nil
        selectedSpaceID = nil; selectedChannelID = nil; challengeID = nil
        navigationGeneration += 1
        navigationTarget = nil; navigationError = nil
        openingSpaceID = nil; openingChannelID = nil
        limits = nil; navigationOpen = false
        busy = false; phase = .signedOut
        async let revoke: Void = api.logout()
        await chat.stop(discardingDrafts: true)
        await presence.stop()
        await voicePresence.stop()
        do { try await revoke } catch { self.error = FriendlyError.message(for: error) }
    }

    public func loadSpaces() async {
        guard account != nil else { return }
        let attempt = generation
        spacesError = nil
        await work(generation: attempt) {
            let response: SpacesResponse
            do { response = try await self.api.spaces() }
            catch {
                if self.generation == attempt { self.spacesError = FriendlyError.message(for: error) }
                throw error
            }
            guard self.generation == attempt else { return }
            self.limits = response.limits
            self.spaces = response.spaces
            self.invitations = response.invitations
            self.spacesLoaded = true
            // A tapped push opens its channel instead of the usual space.
            let route = self.pendingNotificationRoute
            self.pendingNotificationRoute = nil
            if case let .channel(spaceID, channelID)? = route,
               let space = self.spaces.first(where: { $0.id == spaceID }) {
                await self.navigate(space: space, channelID: channelID)
                return
            }
            if let selected = self.spaces.first(where: { $0.id == self.selectedSpaceID })
                ?? self.spaces.first(where: { $0.id == self.preferredInitialSpaceID })
                ?? self.spaces.first {
                await self.select(space: selected)
            }
            if case let .direct(conversationID)? = route, self.generation == attempt {
                await self.openDirectMessage(id: conversationID)
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
            let wasRequest = selectedDirectMessage?.status == .incoming
            directMessages = conversations
            directMessagesError = nil
            // Accepted elsewhere (or by sending from an older client): reopen it
            // with a chat session so the composer works.
            if wasRequest, let selected = selectedDirectMessage, selected.status != .incoming {
                await select(directMessage: selected)
            }
        } catch is CancellationError {} catch {
            guard generation == attempt else { return }
            directMessagesError = FriendlyError.message(for: error)
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

    public func createDirectMessage(username: String) async throws -> Bool {
        let exact = username.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !exact.isEmpty, account != nil, !busy else { return false }
        let attempt = generation
        navigationGeneration += 1
        let navigation = navigationGeneration
        busy = true
        defer { if generation == attempt { busy = false } }
        do {
            let conversation = try await api.createDirectMessage(username: exact)
            guard generation == attempt else { return false }
            if let index = directMessages.firstIndex(where: { $0.id == conversation.id }) { directMessages[index] = conversation }
            else { directMessages.append(conversation) }
            if navigationGeneration == navigation { await select(directMessage: conversation) }
            return generation == attempt
        } catch {
            guard generation == attempt else { return false }
            throw error
        }
    }

    /// Opens the account's notes conversation, creating it through the normal DM
    /// endpoint only when the server has not returned one yet.
    public func openSelfDirectMessage() async {
        guard let account, let username = account.username else { return }
        if let conversation = directMessages.first(where: { $0.peer.id == account.id }) {
            await select(directMessage: conversation)
        } else {
            error = nil
            do { _ = try await createDirectMessage(username: username) }
            catch { self.error = FriendlyError.message(for: error) }
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
            // An incoming request opens read-only: no chat session, no composer,
            // no read cursor and no chimes. Opening it never accepts it.
            let current = directMessages.first { $0.id == conversation.id } ?? conversation
            chat.directPeerID = current.peer.id == account?.id ? nil : current.peer.id
            if current.status == .incoming { await chat.preview(history: history) }
            else { await chat.open(history: history, displayName: account?.displayName ?? "") }
            guard generation == attempt, navigationGeneration == navigation else { return }
            markSelectedDirectRead()
        } catch { if generation == attempt, navigationGeneration == navigation { navigationError = FriendlyError.message(for: error) } }
    }

    public func openDirectMessage(id: String) async {
        await refreshDirectMessages()
        guard let conversation = directMessages.first(where: { $0.id == id }) else { return }
        await select(directMessage: conversation)
    }

    /// Opens a channel by id, as a tapped push does. A space this client has
    /// not loaded yet refreshes the space list first.
    public func openChannel(spaceID: String, channelID: String) async {
        guard account != nil else { return }
        if let space = spaces.first(where: { $0.id == spaceID }) {
            await navigate(space: space, channelID: channelID)
        } else {
            pendingNotificationRoute = .channel(spaceID: spaceID, channelID: channelID)
            await loadSpaces()
        }
    }

    // MARK: Message requests, blocking and DM privacy

    /// Accepts the request: it joins the main list and, when open, gains a composer.
    public func acceptRequest(_ conversation: DirectMessageConversation) async throws {
        let attempt = generation
        let accepted = try await api.acceptDirectMessage(id: conversation.id)
        guard generation == attempt else { return }
        replaceDirectMessage(accepted)
        // Closed with none left; the next request opened follows the view again.
        if messageRequests.isEmpty { messageRequestsOpen = nil }
        if selectedDirectMessageID == conversation.id { await select(directMessage: accepted) }
    }

    /// Declines the request for you only, then returns to the requests (or DM) list.
    public func declineRequest(_ conversation: DirectMessageConversation) async throws {
        let attempt = generation
        try await api.declineDirectMessage(id: conversation.id)
        guard generation == attempt else { return }
        directMessages.removeAll { $0.id == conversation.id }
        if selectedDirectMessageID == conversation.id { await leaveRequest() }
    }

    /// Blocks an account everywhere you share. A pending request from them is
    /// declined server-side, so it leaves the requests list too.
    public func block(_ target: BlockTarget) async throws {
        guard let account else { return }
        guard target.id != account.id else { throw UserFacingError(message: "You can't block yourself.") }
        let attempt = generation
        try await api.block(accountID: target.id)
        guard generation == attempt else { return }
        let leavingRequest = selectedDirectMessage.map { $0.peer.id == target.id && $0.status == .incoming } ?? false
        if !blockedIDs.contains(target.id) {
            blockedAccounts.insert(BlockedAccount(id: target.id, username: target.username ?? "", displayName: target.displayName,
                                                  avatarId: target.avatarId), at: 0)
        }
        blockedIDs.insert(target.id); chat.setBlockedAuthors(blockedIDs)
        directMessages = MessageRequests.applying(blocked: true, peerID: target.id, to: directMessages)
        if leavingRequest { await leaveRequest() }
        // The server's list carries the username a message author lacks.
        Task { [weak self] in await self?.refreshBlocks() }
    }

    public func unblock(accountID: String) async throws {
        guard account != nil else { return }
        let attempt = generation
        try await api.unblock(accountID: accountID)
        guard generation == attempt else { return }
        blockedAccounts.removeAll { $0.id == accountID }
        blockedIDs.remove(accountID); chat.setBlockedAuthors(blockedIDs)
        directMessages = MessageRequests.applying(blocked: false, peerID: accountID, to: directMessages)
    }

    public func refreshBlocks() async {
        guard account != nil else { return }
        let attempt = generation
        do {
            let blocks = try await api.blocks()
            guard generation == attempt, account != nil else { return }
            blockedAccounts = blocks; blockedIDs = Set(blocks.map(\.id)); blocksLoaded = true; blocksError = nil
            chat.setBlockedAuthors(blockedIDs)
        } catch is CancellationError {} catch {
            if generation == attempt { blocksError = FriendlyError.message(for: error) }
        }
    }

    public func loadPrivacy() async {
        guard account != nil else { return }
        let attempt = generation
        do {
            let settings = try await api.privacy()
            guard generation == attempt else { return }
            directMessagePrivacy = settings.directMessages; privacyError = nil
        } catch is CancellationError {} catch {
            if generation == attempt { privacyError = FriendlyError.message(for: error) }
        }
    }

    /// Saves immediately; a failed save reverts the choice and shows why.
    public func setDirectMessagePrivacy(_ value: DirectMessagePrivacy) async {
        guard account != nil, value != directMessagePrivacy else { return }
        let attempt = generation
        let previous = directMessagePrivacy
        directMessagePrivacy = value; privacyError = nil
        do {
            let settings = try await api.updatePrivacy(value)
            guard generation == attempt else { return }
            directMessagePrivacy = settings.directMessages
        } catch {
            guard generation == attempt else { return }
            directMessagePrivacy = previous; privacyError = FriendlyError.message(for: error)
        }
    }

    private func replaceDirectMessage(_ conversation: DirectMessageConversation) {
        if let index = directMessages.firstIndex(where: { $0.id == conversation.id }) { directMessages[index] = conversation }
        else { directMessages.append(conversation) }
    }

    /// After declining or blocking the open request: back to the remaining
    /// requests, or to the DM list when none are left.
    private func leaveRequest() async {
        selectedDirectMessageID = nil
        messageRequestsOpen = messageRequests.isEmpty ? nil : true
        if let space = detail?.space, detail?.channels.contains(where: \.joined) == true {
            await navigate(space: space, channelID: nil)
        } else {
            await chat.stop()
        }
        navigationOpen = true
    }

    public func applicationActivityChanged(active: Bool) {
        foreground = active
        if active, account != nil {
            markSelectedDirectRead()
            removeShownNotifications()
            Task { await refreshDirectMessages() }
            refreshNotificationSettingsIfStale()
            refreshMembership()
        }
    }

    /// Android clears the open conversation's notifications; so does iOS.
    private func removeShownNotifications() {
        guard foreground, account != nil, let id = selectedDirectMessageID ?? selectedChannelID else { return }
        removeDeliveredNotifications?(id)
    }

    private func markSelectedDirectRead() {
        guard foreground, account != nil, let id = selectedDirectMessageID, selectedDirectMessage?.status != .incoming,
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
                directMessages[current] = old.with(readSeq: seq)
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
        #if os(macOS)
        desktopNotifications.start()
        #endif
        directMessageRefreshTask?.cancel()
        let attempt = generation
        Task { [weak self] in await self?.refreshBlocks() }
        // After sign-in (every path starts here), like the block list.
        Task { [weak self] in await self?.loadNotificationSettings() }
        directMessageRefreshTask = Task { [weak self] in
            // Sign-in has just loaded the spaces; membership waits for the next pass.
            var reconcile = false
            while !Task.isCancelled {
                await self?.refreshDirectMessages()
                if reconcile { self?.refreshMembership() }
                reconcile = true
                try? await Task.sleep(for: .seconds(15))
                guard self?.generation == attempt else { return }
            }
        }
    }

    /// Web's membership reconcile, every 15 seconds and on returning to the
    /// app: invitations, and spaces or channels changed elsewhere, appear
    /// without a restart. Runs outside the refresh loop, which removing the
    /// open space restarts.
    private func refreshMembership() {
        guard account != nil, spacesLoaded, !busy, !membershipRefreshing, membershipChanges == 0 else { return }
        membershipRefreshing = true
        Task { [weak self] in
            await self?.reconcileMembership()
            self?.membershipRefreshing = false
        }
    }

    /// A change made here meanwhile wins over the older snapshot, and a failed
    /// read keeps what is shown: an outage is not revocation.
    private func reconcileMembership() async {
        let attempt = generation
        let revision = membershipRevision
        let knownSpaces = spaces, knownInvitations = invitations
        guard let response = try? await api.spaces(), generation == attempt, account != nil,
              membershipChanges == 0, membershipRevision == revision,
              spaces == knownSpaces, invitations == knownInvitations else { return }
        let available = Set(response.spaces.map(\.id))
        for space in spaces where !available.contains(space.id) { invalidateNavigation(spaceID: space.id) }
        limits = response.limits
        spaces = response.spaces
        invitations = response.invitations
        guard let current = detail, openingSpaceID == nil else { return }
        guard available.contains(current.space.id) else {
            // Removed from the open space elsewhere (or it was deleted).
            if voice.isActive(spaceID: current.space.id) { voice.leaveImmediately() }
            await removeCurrentSpace(id: current.space.id)
            navigationError = "This space is no longer available."
            return
        }
        guard selectedDirectMessageID == nil else { return }
        let navigation = navigationGeneration
        guard let refreshed = try? await api.space(current.space.id), refreshed.space.id == current.space.id,
              generation == attempt, navigationGeneration == navigation, openingSpaceID == nil,
              membershipChanges == 0, membershipRevision == revision,
              detail?.space.id == current.space.id, selectedDirectMessageID == nil else { return }
        replace(detail: refreshed)
        guard let channelID = selectedChannelID else { return }
        if let channel = refreshed.channels.first(where: { $0.id == channelID }) {
            // Joined or left elsewhere: reopen with or without the composer.
            if chat.isPreview == channel.joined { await select(channel: channel) }
        } else {
            // Deleted, or private access removed, elsewhere: as deleteChannel does.
            if voice.isActive(channelID: channelID) { voice.leaveImmediately() }
            invalidateNavigation(channelID: channelID)
            if let first = refreshed.channels.first(where: \.joined) { await select(channel: first) }
            else { selectedChannelID = nil; await chat.stop() }
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
                chat.directPeerID = nil
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
            navigationError = missingSpace ? "This space is no longer available." : FriendlyError.message(for: error)
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
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        clearNavigationCache()
        let attempt = generation
        if let error = WorkspaceValidation.spaceNameError(name) { throw APIError(status: 400, message: error) }
        let created = try await api.createSpace(name: name)
        guard generation == attempt, account != nil else { throw CancellationError() }
        spaces.append(created)
        await select(space: created)
    }

    public func renameSpace(_ name: String) async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        guard let detail else { return }
        clearNavigationCache()
        let attempt = generation
        if let error = WorkspaceValidation.spaceNameError(name) { throw APIError(status: 400, message: error) }
        let updated = try await api.updateSpace(id: detail.space.id, name: name)
        guard generation == attempt, self.detail?.space.id == detail.space.id else { throw CancellationError() }
        replace(detail: SpaceDetail(space: updated, channels: detail.channels, members: detail.members, channelInvitations: detail.channelInvitations))
    }

    public func deleteCurrentSpace() async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        guard let id = detail?.space.id else { return }
        clearNavigationCache()
        let attempt = generation
        try await api.deleteSpace(id: id)
        guard generation == attempt, detail?.space.id == id else { throw CancellationError() }
        if voice.isActive(spaceID: id) { voice.leaveImmediately() }
        await removeCurrentSpace(id: id)
    }

    public func leaveCurrentSpace() async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
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
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        let attempt = generation
        let accepted = try await api.acceptSpaceInvitation(spaceID: invitation.id)
        guard generation == attempt else { throw CancellationError() }
        invitations.removeAll { $0.id == invitation.id }
        spaces.removeAll { $0.id == accepted.id }; spaces.append(accepted)
        await select(space: accepted)
    }

    public func declineInvitation(_ invitation: Space) async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        let attempt = generation
        try await api.declineSpaceInvitation(spaceID: invitation.id)
        guard generation == attempt else { throw CancellationError() }
        invitations.removeAll { $0.id == invitation.id }
    }

    public func removeSpaceMember(_ member: Member) async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
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
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
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
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
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
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
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
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        guard let spaceID = detail?.space.id else { return }
        let attempt = generation
        clearNavigationCache()
        _ = try await api.joinChannel(spaceID: spaceID, channelID: channel.id)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        try await refreshDetail(spaceID: spaceID, selecting: channel.id)
    }

    public func leaveChannel(_ channel: Channel) async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
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
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        guard let spaceID = detail?.space.id else { return }
        let attempt = generation
        clearNavigationCache()
        _ = try await api.acceptChannelInvitation(spaceID: spaceID, channelID: invitation.channel.id)
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        try await refreshDetail(spaceID: spaceID, selecting: invitation.channel.id)
    }

    public func declineChannelInvitation(_ invitation: ChannelInvitation) async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
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
            navigationError = FriendlyError.message(for: error)
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
            self.error = FriendlyError.message(for: error)
        }
        if expectedGeneration == nil || generation == expectedGeneration { busy = false }
    }
}

/// "Resend code" on the sign-in code step, as on web.
enum CodeResend {
    /// Each code, the first included, starts a one-minute wait.
    static let wait: TimeInterval = 60
    /// Three codes per email entry: the server silently stops sending after
    /// three in 15 minutes, so a fourth would never arrive.
    static let maximumResends = 2

    /// Whole seconds until Resend code is available, rounded up; 0 when it is.
    static func secondsRemaining(sentAt: Date?, now: Date) -> Int {
        guard let sentAt else { return 0 }
        return max(0, Int((wait - now.timeIntervalSince(sentAt)).rounded(.up)))
    }

    /// The wait as m:ss, e.g. "0:42".
    static func countdown(_ seconds: Int) -> String {
        let seconds = max(0, seconds)
        return "\(seconds / 60):" + (seconds % 60 < 10 ? "0" : "") + "\(seconds % 60)"
    }

    /// The button's label while waiting, then "Resend code".
    static func label(secondsRemaining: Int) -> String {
        secondsRemaining > 0 ? "Resend code in \(countdown(secondsRemaining))" : "Resend code"
    }
}

// MARK: Notifications

extension AppModel {
    /// Loads after sign-in, on returning to the app, and when settings or a
    /// menu opens. A change made meanwhile wins over the older snapshot.
    public func loadNotificationSettings() async {
        guard account != nil else { return }
        let attempt = generation
        notificationRequest += 1
        let request = notificationRequest
        notificationsLoadedAt = Date()
        do {
            let settings = try await api.notificationSettings()
            guard generation == attempt, notificationRequest == request, notificationChangesInFlight == 0, account != nil else { return }
            notificationSettings = settings
            notificationsLoadError = nil
            scheduleMuteExpiry()
        } catch is CancellationError {} catch {
            guard generation == attempt, notificationRequest == request else { return }
            notificationsLoadError = NotificationLabels.loadFailed
        }
    }

    /// For menus: reloads unless the settings loaded in the last 30 seconds.
    public func refreshNotificationSettingsIfStale() {
        guard account != nil else { return }
        if let loadedAt = notificationsLoadedAt, Date().timeIntervalSince(loadedAt) < 30 { return }
        Task { [weak self] in await self?.loadNotificationSettings() }
    }

    /// Muted itself or, for a channel, through its space.
    public func notificationsMuted(_ scope: NotificationScope) -> Bool {
        _ = muteExpiryTick
        return notificationSettings?.isMuted(scope, now: Date()) ?? false
    }

    /// The scope's own active mute, for "Unmute …" and "Muted until …".
    public func notificationMute(_ scope: NotificationScope) -> MuteUntil? {
        _ = muteExpiryTick
        return notificationSettings?.ownMute(for: scope, now: Date())
    }

    public func notificationsMutedWithSpace(_ scope: NotificationScope) -> Bool {
        _ = muteExpiryTick
        return notificationSettings?.isMutedWithSpace(scope, now: Date()) ?? false
    }

    /// "Notify me about". Optimistic; a failure reverts and shows why.
    public func setAccountNotificationLevel(_ level: NotificationLevel) async {
        guard let current = notificationSettings, current.level != level else { return }
        await saveAccountNotifications(NotificationAccountChange(level: level), field: "level")
    }

    /// "Send to this phone". Optimistic; a failure reverts and shows why.
    public func setMobilePushPolicy(_ mobile: MobilePushPolicy) async {
        guard let current = notificationSettings, current.mobile != mobile else { return }
        await saveAccountNotifications(NotificationAccountChange(mobile: mobile), field: "mobile")
    }

    /// A space or channel level (nil is Default), or a DM's on/off (`nothing` or nil).
    public func setNotificationLevel(_ level: NotificationLevel?, for scope: NotificationScope) async {
        guard let current = notificationSettings, current.overrideLevel(for: scope) != level else { return }
        await saveOverride(.level(level), for: scope)
    }

    /// Mutes until a time or `forever`; nil unmutes.
    public func setMute(_ until: MuteUntil?, for scope: NotificationScope) async {
        guard notificationSettings != nil else { return }
        await saveOverride(.mute(until), for: scope)
    }

    /// Whether a push's conversation is the one open now.
    public func isShowing(_ route: NotificationRoute) -> Bool {
        route.isOpen(spaceID: selectedSpaceID, channelID: selectedChannelID, directMessageID: selectedDirectMessageID)
    }

    /// Opens a tapped push's channel or DM. Before spaces load, it waits and
    /// opens in place of the usual first space.
    public func open(_ route: NotificationRoute) async {
        guard phase != .signedOut else { return }
        guard account != nil, spacesLoaded else {
            pendingNotificationRoute = route
            return
        }
        switch route {
        case let .direct(conversationID): await openDirectMessage(id: conversationID)
        case let .channel(spaceID, channelID): await openChannel(spaceID: spaceID, channelID: channelID)
        }
    }

    private func saveAccountNotifications(_ change: NotificationAccountChange, field: String) async {
        guard account != nil, let current = notificationSettings else { return }
        let attempt = generation
        let revision = (accountNotificationRevisions[field] ?? 0) + 1
        accountNotificationRevisions[field] = revision
        notificationRequest += 1
        notificationChangesInFlight += 1
        notificationSettings = current.applying(change)
        notificationError = nil
        let restore = NotificationAccountChange(level: change.level == nil ? nil : current.level,
                                                mobile: change.mobile == nil ? nil : current.mobile)
        do {
            let saved = try await api.updateNotificationSettings(change)
            guard generation == attempt else { return }
            notificationChangesInFlight -= 1
            guard accountNotificationRevisions[field] == revision else { return }
            // Only this change's field: other fields may have changes in flight.
            let confirmed = NotificationAccountChange(level: change.level == nil ? nil : saved.level,
                                                      mobile: change.mobile == nil ? nil : saved.mobile)
            notificationSettings = notificationSettings?.applying(confirmed)
        } catch {
            guard generation == attempt else { return }
            notificationChangesInFlight -= 1
            guard accountNotificationRevisions[field] == revision else { return }
            notificationSettings = notificationSettings?.applying(restore)
            notificationError = NotificationLabels.saveFailed
        }
    }

    private func saveOverride(_ change: NotificationOverrideChange, for scope: NotificationScope) async {
        guard account != nil, let current = notificationSettings else { return }
        let attempt = generation
        let revision = (overrideRevisions[scope] ?? 0) + 1
        overrideRevisions[scope] = revision
        notificationRequest += 1
        notificationChangesInFlight += 1
        let previous = current.override(for: scope)
        notificationSettings = current.applying(change, to: scope)
        notificationError = nil
        scheduleMuteExpiry()
        do {
            let saved = try await api.updateNotificationOverride(scope, change: change)
            guard generation == attempt else { return }
            notificationChangesInFlight -= 1
            guard overrideRevisions[scope] == revision else { return }
            notificationSettings = notificationSettings?.replacing(saved, for: scope)
        } catch {
            guard generation == attempt else { return }
            notificationChangesInFlight -= 1
            guard overrideRevisions[scope] == revision else { return }
            notificationSettings = notificationSettings?.replacing(previous, for: scope)
            notificationError = NotificationLabels.saveFailed
        }
        scheduleMuteExpiry()
    }

    /// Redraws muted rows when the next timed mute ends.
    private func scheduleMuteExpiry() {
        muteExpiryTask?.cancel()
        muteExpiryTask = nil
        guard let next = notificationSettings?.nextMuteExpiry(after: Date()) else { return }
        let delay: TimeInterval = max(0, next.timeIntervalSinceNow) + 1
        let attempt = generation
        muteExpiryTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled, let self, self.generation == attempt else { return }
            self.muteExpiryTick += 1
            self.scheduleMuteExpiry()
        }
    }

    private func clearNotificationState() {
        #if os(macOS)
        desktopNotifications.stop()
        #endif
        muteExpiryTask?.cancel(); muteExpiryTask = nil
        notificationSettings = nil; notificationError = nil; notificationsLoadError = nil
        notificationsLoadedAt = nil; notificationChangesInFlight = 0
        accountNotificationRevisions = [:]; overrideRevisions = [:]
        pendingNotificationRoute = nil
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
    private var mutations = MessageMutations()
    public var displayedMessages: [ChatMessage] { messages.map(mutations.project) }
    public var displayedPins: [ChatMessage] { mutations.pinned(messages: messages, confirmed: pinnedMessages) }
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
    public var hasNewer = false
    public var loadingNewer = false
    public var focusedMessageID: String?
    public var focusRevision = 0
    public var jumpingToMessage = false
    public var jumpError: String?
    private var historyAnchorRequest = 0
    private var windowStart: String?
    private var windowEnd: String?
    public var typingNames: [String] = []
    public var reactionErrors: [String: String] = [:]
    public var pinErrors: [String: String] = [:]
    public var pendingPins: Set<String> = []
    public var threadRootID: String?
    public var threadLoading = false
    public var threadHasMore = false
    public var threadHasNewer = false
    private var threadAfter: String?
    private var threadWindowStart: String?
    private var threadWindowEnd: String?
    public var threadError: String?
    private var threadBefore: String?
    private var threadRequest = 0
    private var threadPages: [String: (hasMore: Bool, before: String?, hasNewer: Bool, after: String?, windowStart: String?, windowEnd: String?)] = [:]
    private var threadOnlyRows: Set<String> = []
    private var threadDrafts: [String: String] = [:]
    /// Unsent text of other conversations by channel or DM ID, restored on return.
    private var drafts: [String: String] = [:]
    private var restoredDraft: String?
    private var threadBroadcasts: [String: Bool] = [:]
    public var channelMessages: [ChatMessage] { messages.filter { $0.isChannelMessage && !threadOnlyRows.contains($0.id) && Self.inWindow($0, start: windowStart, end: windowEnd) } }
    public var displayedChannelMessages: [ChatMessage] { channelMessages.map(mutations.project) }
    public var threadMessages: [ChatMessage] { messages.filter { $0.threadRootId != nil && $0.threadRootId == threadRootID && Self.inWindow($0, start: threadWindowStart, end: threadWindowEnd) } }
    public var displayedThreadMessages: [ChatMessage] { threadMessages.map(mutations.project) }
    private static func inWindow(_ message: ChatMessage, start: String?, end: String?) -> Bool {
        (start == nil || (try? Sequence.compare(message.seq, start!)) != .orderedAscending) &&
        (end == nil || (try? Sequence.compare(message.seq, end!)) != .orderedDescending)
    }
    public var threadDraft: String {
        get { threadRootID.flatMap { threadDrafts[$0] } ?? "" }
        set { if let threadRootID { threadDrafts[threadRootID] = newValue } }
    }
    public var threadBroadcast: Bool {
        get { threadRootID.flatMap { threadBroadcasts[$0] } ?? false }
        set { if let threadRootID { threadBroadcasts[threadRootID] = newValue } }
    }
    public var currentAuthor: ChatAuthor? { session?.author }
    public private(set) var canForward = false
    public var forwardTarget: ChatMessage?
    public var forwardConversationTarget: ChatMessage?
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
    /// Accounts you blocked: their messages collapse in every timeline, and
    /// they never chime or show as typing. Kept in step by `AppModel`.
    public private(set) var blockedAuthorIDs: Set<String> = []
    /// The other person in the open DM, kept by `AppModel`. Blocking them stops
    /// reactions, pins and edits as well as sending; the server refuses them too.
    public var directPeerID: String?
    /// Reacting, pinning and editing need a chat session (not a preview), and
    /// never reach someone you blocked.
    public var canInteract: Bool {
        !isPreview && currentAuthor != nil && !(directPeerID.map { blockedAuthorIDs.contains($0) } ?? false)
    }
    /// The signed-in account, also while previewing without a chat session.
    @ObservationIgnored public var viewerAccountID: () -> String? = { nil }
    /// Blocks from message actions in panels that only hold the chat model.
    @ObservationIgnored public var blockAccount: ((BlockTarget) async throws -> Void)?
    /// Unblocks from message actions, by account id.
    @ObservationIgnored public var unblockAccount: ((String) async throws -> Void)?
    /// A DM send refused with `dm_blocked`: the block list needs a refresh.
    @ObservationIgnored public var onDirectMessageBlocked: (() -> Void)?
    public var viewerID: String? { session?.author.id ?? viewerAccountID() }
    private let api: APIClient
    private var channelID: String?
    private var spaceID: String?
    private var session: ChatSession?
    private var subscriptionID: String?
    private var generation = 0
    private var delivery = ChatDeliveryState()
    private var reactionSnapshots = ReactionSnapshots()
    private var pinSnapshots = PinSnapshots()
    private var forwardSnapshots = ForwardSnapshots()
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

    func setBlockedAuthors(_ ids: Set<String>) {
        guard ids != blockedAuthorIDs else { return }
        blockedAuthorIDs = ids
        refreshTypers()
    }

    /// Blocks from a message action; a failure shows as the conversation error.
    public func block(_ target: BlockTarget) async {
        do { try await blockAccount?(target) } catch { self.error = FriendlyError.message(for: error) }
    }

    /// Unblocks a message's author; a failure shows web's wording as the conversation error.
    public func unblock(_ author: ChatAuthor) {
        Task {
            do { try await unblockAccount?(author.id) } catch { self.error = "Couldn’t unblock \(author.name). Try again." }
        }
    }

    func canEdit(_ message: ChatMessage) -> Bool {
        message.forward == nil && canInteract && channelID == message.channelId && currentAuthor?.isGuest == false && currentAuthor?.id == message.author.id
    }

    func editMessage(_ message: ChatMessage, text: String) async throws {
        guard canEdit(message), let session else { throw UserFacingError(message: "Only the author can edit while participating.") }
        guard mutations.edits[message.id] == nil else { throw UserFacingError(message: "This message is already being saved.") }
        let request = generation
        mutations.edits[message.id] = (text, message.revision ?? 1)
        defer { if request == generation { mutations.edits[message.id] = nil } }
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
            messages: channelMessages.map { forwardSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0))) },
            pinnedMessages: pinnedMessages.map { forwardSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0))) },
            cursor: delivery.cursor,
            hasMore: hasMore, hasNewer: hasNewer
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
        let signedIn = await api.isSignedIn
        guard generation == requestGeneration else { return }
        canForward = signedIn
        channelID = history.channel?.id; spaceID = history.space?.id
        reactionSnapshots.seed(history.messages + history.pinnedMessages)
        pinSnapshots.replace(history.messages + history.pinnedMessages, cursor: history.cursor)
        forwardSnapshots.seed(history.messages + history.pinnedMessages)
        editSnapshots.seed(history.messages + history.pinnedMessages)
        messages = history.messages.map { forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0)))) }
        pinnedMessages = history.pinnedMessages.map { forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0)))) }
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
        threadPages = [:]
        mutations = MessageMutations(); pendingPins = []
        let requestGeneration = generation
        let oldSubscription = subscriptionID
        subscriptionID = nil
        let preservedDraft = draft
        let preservedMessages = channelMessages
        let preservedCursor = delivery.cursor
        let preservedHasMore = hasMore
        let preservedHasNewer = hasNewer
        historyAnchorRequest += 1
        loadingNewer = false; jumpingToMessage = false
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
        let restoredDraft = channelID.flatMap { drafts.removeValue(forKey: $0) } ?? ""
        draft = preservingPending ? preservedDraft : restoredDraft
        self.restoredDraft = draft.isEmpty ? nil : draft
        // Prepared history is already loaded and authorized by navigation.
        // Show it immediately; obtaining a sending capability is not a new
        // history load (and must not flash the previous channel's title).
        if let prepared {
            self.channelID = prepared.channel?.id; spaceID = prepared.space?.id
            reactionSnapshots.seed(prepared.messages + prepared.pinnedMessages)
            pinSnapshots.replace(prepared.messages + prepared.pinnedMessages, cursor: prepared.cursor)
            forwardSnapshots.seed(prepared.messages + prepared.pinnedMessages)
            editSnapshots.seed(messages + pinnedMessages + prepared.messages + prepared.pinnedMessages)
            messages = prepared.messages.map { forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0)))) }
            pinnedMessages = prepared.pinnedMessages.map { forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0)))) }
            delivery.reset(cursor: prepared.cursor); hasMore = prepared.hasMore
            hasNewer = prepared.hasNewer
            windowStart = prepared.hasNewer ? prepared.messages.first?.seq : nil
            windowEnd = prepared.hasNewer ? prepared.messages.last?.seq : nil
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
            let signedIn = await api.isSignedIn
            guard self.channelID == channelID, generation == requestGeneration else { return }
            canForward = signedIn
            let resolvedChannelID = history.channel?.id
            spaceID = history.space?.id
            self.channelID = resolvedChannelID
            let firstRefreshed = history.messages.first?.seq
            let canRetain = preservingTimeline && !preservedHasNewer && Self.refreshAccountsForMissingEvents(
                messages: history.messages,
                after: preservedCursor,
                through: history.cursor
            )
            if prepared == nil {
                // The latest history response is authoritative for the channel-wide
                // pin list. Older pagination responses are deliberately ignored.
                reactionSnapshots.seed(history.messages + history.pinnedMessages)
                pinSnapshots.replace(history.messages + history.pinnedMessages, cursor: history.cursor)
                forwardSnapshots.seed(history.messages + history.pinnedMessages)
                // Overflow requires fresh pages; never keep the resync flag latched.
                if editSnapshots.unseenOverflowed { editSnapshots.reset() }
                editSnapshots.seed(messages + pinnedMessages + history.messages + history.pinnedMessages)
                let candidates = Dictionary((pinnedMessages + history.pinnedMessages).map { ($0.id, $0) }, uniquingKeysWith: { _, next in next })
                pinnedMessages = candidates.values.map { forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0)))) }.filter { $0.pin != nil }
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
                    messages = history.messages.map { forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0)))) }
                    reactionSnapshots.reset()
                    reactionSnapshots.seed(messages + pinnedMessages)
                }
                delivery.reset(cursor: history.cursor, preservingPending: preservingPending)
                let retainedOlderPrefix = canRetain && firstRefreshed.map { first in
                    preservedMessages.contains { (try? Sequence.compare($0.seq, first)) == .orderedAscending }
                } == true
                hasMore = retainedOlderPrefix ? preservedHasMore : history.hasMore
                hasNewer = false; windowStart = nil; windowEnd = nil; focusedMessageID = nil
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
        let anchorRequest = historyAnchorRequest
        loadingOlder = true; olderError = nil
        defer { if generation == requestGeneration { loadingOlder = false } }
        do {
            let page = try await api.history(channelID: channelID, before: before)
            guard generation == requestGeneration, anchorRequest == historyAnchorRequest, self.channelID == channelID else { return }
            threadOnlyRows.subtract(page.messages.map(\.id))
            merge(page.messages)
            if windowStart != nil { windowStart = page.messages.first?.seq ?? windowStart }
            hasMore = page.hasMore
        } catch {
            guard generation == requestGeneration, anchorRequest == historyAnchorRequest, self.channelID == channelID else { return }
            if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) {
                onAccessRevoked?(channelID)
                await stop()
                guard generation == requestGeneration + 1 else { return }
                self.error = error.localizedDescription
            } else { olderError = error.localizedDescription }
        }
    }

    public func closeThread() { threadRequest += 1; threadRootID = nil; threadLoading = false }

    public func goToMessage(_ message: ChatMessage) async -> Bool {
        guard !loading, !jumpingToMessage, message.channelId == channelID else { return false }
        let requestGeneration = generation
        historyAnchorRequest += 1
        let request = historyAnchorRequest
        jumpingToMessage = true; jumpError = nil
        loadingNewer = false; loadingOlder = false
        defer { if generation == requestGeneration && request == historyAnchorRequest { jumpingToMessage = false } }
        do {
            if let root = message.threadRootId {
                await openThread(root, around: message.id)
                guard generation == requestGeneration, request == historyAnchorRequest else { return false }
                if let error = threadError { throw APIError(status: 502, message: error) }
                guard threadMessages.contains(where: { $0.id == message.id }) else { return false }
            } else {
                let page = try await api.history(channelID: message.channelId, around: message.id)
                guard generation == requestGeneration, request == historyAnchorRequest else { return false }
                guard page.messages.contains(where: { $0.id == message.id }) else { throw APIError(status: 502, message: "Message context is unavailable.") }
                windowStart = page.messages.first?.seq; windowEnd = page.hasNewer ? page.messages.last?.seq : nil
                threadOnlyRows.subtract(page.messages.map(\.id)); merge(page.messages)
                hasMore = page.hasMore; hasNewer = page.hasNewer; closeThread()
            }
            focusedMessageID = message.id; focusRevision += 1
            return true
        } catch {
            if generation == requestGeneration && request == historyAnchorRequest { jumpError = error.localizedDescription }
            return false
        }
    }

    public func loadNewer() async {
        guard hasNewer, !loadingNewer, let channelID, let after = channelMessages.last?.seq else { return }
        let requestGeneration = generation; let request = historyAnchorRequest
        loadingNewer = true
        defer { if generation == requestGeneration && request == historyAnchorRequest { loadingNewer = false } }
        do {
            let page = try await api.history(channelID: channelID, after: after)
            guard generation == requestGeneration, request == historyAnchorRequest else { return }
            threadOnlyRows.subtract(page.messages.map(\.id)); merge(page.messages)
            hasNewer = page.hasNewer; windowEnd = page.hasNewer ? page.messages.last?.seq ?? windowEnd : nil
        } catch {
            if generation == requestGeneration && request == historyAnchorRequest { self.error = error.localizedDescription }
        }
    }

    public func openThread(_ rootID: String, around: String? = nil) async {
        guard around != nil || threadRootID != rootID else { return }
        threadRequest += 1
        threadRootID = rootID; threadError = nil
        let cached = around == nil ? threadPages[rootID] : nil
        threadBefore = cached?.before; threadHasMore = cached?.hasMore ?? false
        threadAfter = cached?.after; threadHasNewer = cached?.hasNewer ?? false
        threadWindowStart = cached?.windowStart; threadWindowEnd = cached?.windowEnd
        threadLoading = false
        if cached == nil { await loadThread(around: around) }
    }

    public func loadThread(older: Bool = false, newer: Bool = false, around: String? = nil) async {
        guard let rootID = threadRootID, let channelID else { return }
        threadRequest += 1
        let request = threadRequest; let channelGeneration = generation
        if !older && !newer {
            threadBefore = nil; threadAfter = nil; threadHasNewer = false
            threadWindowStart = nil; threadWindowEnd = nil
        }
        threadLoading = true; threadError = nil
        do {
            let page = try await api.thread(channelID: channelID, rootID: rootID, before: older ? threadBefore : nil, after: newer ? threadAfter : nil, around: around)
            guard request == threadRequest, generation == channelGeneration, threadRootID == rootID else { return }
            let loaded = Set(messages.map(\.id))
            let rows = [page.root] + page.messages
            threadOnlyRows.formUnion(rows.filter { $0.isChannelMessage && !loaded.contains($0.id) }.map(\.id))
            merge(rows)
            if !newer { threadHasMore = page.hasMore; threadBefore = page.messages.first?.seq ?? threadBefore; threadWindowStart = threadBefore }
            if !older { threadHasNewer = page.hasNewer ?? false; threadAfter = page.messages.last?.seq ?? threadAfter; threadWindowEnd = threadHasNewer ? threadAfter : nil }
            threadPages[rootID] = (threadHasMore, threadBefore, threadHasNewer, threadAfter, threadWindowStart, threadWindowEnd)
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
        // "Also send to channel" applies to one reply, so it resets with the draft.
        if newSubmission { if inThread { threadDraft = ""; threadBroadcast = false } else { draft = "" } }
        let requestGeneration = generation
        sending = true; error = nil
        defer {
            sending = false
            if delivery.pending == nil, self.channelID == channelID, generation == requestGeneration {
                if inThread, let rootID, threadHasNewer {
                    focusedMessageID = nil
                    Task { if self.channelID == channelID, generation == requestGeneration, threadRootID == rootID { await loadThread() } }
                } else if !inThread, hasNewer {
                    Task { if self.channelID == channelID, generation == requestGeneration { await retryLoad() } }
                }
            }
        }
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
                return
            }
            if let apiError = error as? APIError, [400, 404, 409, 413, 422].contains(apiError.status)
                || DirectMessageErrors.isRefusal(status: apiError.status, code: apiError.code) {
                delivery.reject(id: command.id)
                self.error = apiError.localizedDescription
                if apiError.code == "dm_blocked" { onDirectMessageBlocked?() }
            } else {
                self.error = "Send outcome is unknown. Retry to safely resend the same message. \(error.localizedDescription)"
            }
        }
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
        guard let message = (messages + pinnedMessages).first(where: { $0.id == messageID }) else { return }
        pendingPins.insert(messageID); pinErrors[messageID] = nil
        failedPinActions[messageID] = nil
        let requestGeneration = generation
        mutations.pins[messageID] = MessageMutations.PinIntent(message: message,
            pin: active ? MessagePin(author: session.author, createdAt: ISO8601DateFormatter().string(from: Date())) : nil)
        defer { if generation == requestGeneration { pendingPins.remove(messageID); mutations.pins[messageID] = nil } }
        do {
            let event = try await api.setPin(channelID: channelID, messageID: messageID, sessionToken: session.token, active: active)
            guard generation == requestGeneration, self.channelID == channelID else { return }
            applyPin(event.message)
        } catch {
            guard generation == requestGeneration, self.channelID == channelID else { return }
            let apiError = error as? APIError
            // A DM block refusal keeps the conversation open with the reason inline.
            if let apiError, [401, 403, 404].contains(apiError.status),
               !DirectMessageErrors.isRefusal(status: apiError.status, code: apiError.code) {
                onAccessRevoked?(channelID); await stop(); return
            }
            failedPinActions[messageID] = active
            pinErrors[messageID] = DirectMessageErrors.message(code: apiError?.code)
                ?? "Couldn’t \(active ? "pin" : "unpin") message. Try again."
        }
    }

    public func retryPin(messageID: String) async {
        guard let active = failedPinActions[messageID] else { return }
        await setPin(messageID: messageID, active: active)
    }

    public func forwardDestinations() async throws -> [ForwardDestination] { try await api.forwardDestinations() }

    public func forward(message: ChatMessage, destinationID: String, key: String, text: String) async throws {
        let requestGeneration = generation
        guard await api.isSignedIn else { throw APIError(status: 401, message: "Sign in required.") }
        let capability: ChatSession
        if let session { capability = session }
        else {
            capability = try await api.chatSession(name: "Forward")
            guard generation == requestGeneration else { throw CancellationError() }
            session = capability
        }
        guard !capability.author.isGuest else { throw APIError(status: 401, message: "Sign in required.") }
        let accepted = try await api.forward(source: message, destinationID: destinationID, sessionToken: capability.token, clientMessageID: key, text: text)
        guard generation == requestGeneration else { return }
        if channelID == destinationID { merge([accepted]) }
    }

    public func forwardedConversation(message: ChatMessage, before: String? = nil) async throws -> ForwardConversationHistory {
        try await api.forwardedConversation(message: message, before: before)
    }

    @discardableResult public func discardRejected(edit: Bool = false) -> Bool {
        guard !sending, delivery.rejected, !edit || draft.isEmpty else { return false }
        guard let text = delivery.discardRejected() else { return false }
        if edit { draft = text }
        error = nil
        return true
    }

    /// Typing follows edits, not a draft restored on opening its conversation.
    public func draftChanged(_ value: String) {
        if let restored = restoredDraft {
            restoredDraft = nil
            if value == restored { return }
        }
        setTyping(!value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
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

    public func stop(discardingDrafts: Bool = false) async {
        generation += 1
        let oldSubscription = subscriptionID
        subscriptionID = nil
        clearLocal()
        if discardingDrafts { drafts = [:] }
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
        if type == "message.forward" {
            guard let data = try? JSONSerialization.data(withJSONObject: event),
                  let update = try? JSONDecoder().decode(MessageForwardEvent.self, from: data), update.isValid,
                  update.channelId == eventChannelID, delivery.receive(seq: update.seq) else {
                requestResync(generation: eventGeneration, channelID: eventChannelID); return
            }
            forwardSnapshots.seed([update.message])
            messages = messages.map { forwardSnapshots.overlay($0) }
            pinnedMessages = pinnedMessages.map { forwardSnapshots.overlay($0) }
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
                if newMessage, let author = session?.author, author.id != message.author.id,
                   !BlockedMessages.isHidden(message, blocked: blockedAuthorIDs, viewerID: author.id) { CaperEffects.shared.play(.message) }
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

    /// Keeps the open conversation's unsent text so returning to it restores it.
    private func stashDraft() {
        guard let channelID else { return }
        drafts[channelID] = draft.isEmpty ? nil : draft
    }

    private func clearLocal(preservingPending: Bool = false) {
        stashDraft()
        typingTask?.cancel(); typingIdleTask?.cancel(); typingExpiryTask?.cancel()
        typingTask = nil; typingIdleTask = nil; typingExpiryTask = nil
        typers = [:]; typingNames = []; typingActive = false; typingSent = false
        delivery.reset(preservingPending: preservingPending)
        reactionSnapshots.reset(); pendingReactions = [:]; reactionWorkers = []; reactionErrors = [:]; failedReactions = [:]
        reactorCache = [:]; reactorRequests = [:]; reactorFailures = []
        pinSnapshots.reset(); pinnedMessages = []; pendingPins = []; pinErrors = [:]
        forwardSnapshots.reset()
        mutations = MessageMutations()
        canForward = false; forwardTarget = nil; forwardConversationTarget = nil
        editSnapshots.reset()
        failedPinActions = [:]
        closeThread(); threadPages = [:]; threadOnlyRows = []; threadDrafts = [:]; threadBroadcasts = [:]
        isPreview = false
        session = nil; channelID = nil; spaceID = nil; messages = []; draft = ""; hasMore = false
        hasNewer = false; loadingNewer = false; windowStart = nil; windowEnd = nil
        focusedMessageID = nil; jumpingToMessage = false; jumpError = nil; historyAnchorRequest += 1
        channelName = "general"; spaceName = "Caper"; error = nil
        loadingOlder = false; olderError = nil
        loading = false; sending = false; liveState = .disconnected
    }

    private func merge(_ incoming: [ChatMessage]) {
        reactionSnapshots.seed(incoming)
        pinSnapshots.seed(incoming)
        forwardSnapshots.seed(incoming)
        editSnapshots.seed(messages + pinnedMessages + incoming)
        var byID = Dictionary(uniqueKeysWithValues: messages.map { ($0.id, $0) })
        var summaries: [String: ThreadSummary] = [:]
        for message in messages + incoming {
            guard let summary = message.thread else { continue }
            let root = message.threadRootId ?? message.id
            if summaries[root] == nil || (try? Sequence.compare(summary.seq, summaries[root]!.seq)) == .orderedDescending { summaries[root] = summary }
        }
        incoming.forEach { byID[$0.id] = forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0)))) }
        byID = byID.mapValues { message in
            var updated = forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay(message))))
            updated.thread = summaries[message.threadRootId ?? message.id] ?? message.thread
            return updated
        }
        messages = byID.values.sorted { (try? Sequence.compare($0.seq, $1.seq)) == .orderedAscending }
        renderReactions()
    }

    private func applyPin(_ message: ChatMessage) {
        editSnapshots.seed(messages + pinnedMessages + [message])
        reactionSnapshots.seed([message])
        guard pinSnapshots.apply(message) else { renderReactions(); return }
        messages = messages.map { editSnapshots.overlay(pinSnapshots.overlay($0)) }
        pinnedMessages.removeAll { $0.id == message.id }
        let updated = forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay(message))))
        if updated.pin != nil {
            pinnedMessages.append(updated)
            pinnedMessages.sort { (try? Sequence.compare($0.pinSeq ?? "0", $1.pinSeq ?? "0")) == .orderedDescending }
        }
        renderReactions()
    }

    private func applyReactions(_ event: MessageReactionsEvent) {
        guard reactionSnapshots.apply(messageID: event.messageId, seq: event.seq, reactions: event.reactions) else { return }
        renderReactions()
    }

    private func renderReactions() {
        let authorID = session?.author.id
        let project: (ChatMessage) -> ChatMessage = { [self] message in
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
        messages = messages.map(project)
        pinnedMessages = pinnedMessages.map(project)
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
                let apiError = error as? APIError
                // A DM block refusal keeps the conversation open with the reason inline.
                if let apiError, [401, 403, 404].contains(apiError.status),
                   !DirectMessageErrors.isRefusal(status: apiError.status, code: apiError.code) {
                    onAccessRevoked?(channelID)
                    await stop()
                    return
                }
                if pendingReactions[messageID]?[emoji] == desired {
                    pendingReactions[messageID]?[emoji] = nil
                    if pendingReactions[messageID]?.isEmpty == true { pendingReactions[messageID] = nil }
                    reactionErrors[messageID] = DirectMessageErrors.message(code: apiError?.code) ?? "Couldn’t save reaction. Retry."
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
            entry.typing && entry.expires > now && entry.author.id != session?.author.id
                && !blockedAuthorIDs.contains(entry.author.id) ? entry.author.name : nil
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
