import Foundation
import Observation

@MainActor @Observable
public final class AppModel {
    public enum Phase { case loading, signedOut, onboarding, ready }
    public var phase: Phase = .loading
    public var account: Account?
    public var spaces: [Space] = []
    public private(set) var voiceActivitySpaceIDs: Set<String> = []
    private var voiceActivityRefreshing = false
    public var invitations: [Space] = []
    public var notificationInvitation: Space?
    public var notificationChannelInvitation: ChannelInvitation?
    /// Other people's newest names and pictures from the live `profiles`
    /// feed, applied over every retained copy of them (your own come from `account`).
    public private(set) var profileUpdates = ProfileDirectory()
    private var pendingMemberSnapshots: [Member] = []
    public var pendingMembers: [Member] {
        get { pendingMemberSnapshots.map { profiled($0) } }
        set { pendingMemberSnapshots = newValue }
    }
    private var detailSnapshot: SpaceDetail?
    public var detail: SpaceDetail? {
        get { detailSnapshot.map { profileUpdates.apply($0) }?.withProfile(account) }
        set { detailSnapshot = newValue }
    }
    public var selectedSpaceID: String?
    public var selectedChannelID: String? { didSet { if selectedChannelID != oldValue {
        removeShownNotifications()
        beginChannelUnreadVisit()
    } } }
    private var unreadVisitToken = UUID()
    private var unreadVisitReady = false
    private var unreadVisit: UnreadChannel?

    /// Resolve the entry snapshot before any automatic acknowledgement can advance it.
    private func beginChannelUnreadVisit() {
        let token = UUID(), attempt = generation
        unreadVisitToken = token
        unreadVisitReady = false
        unreadVisit = nil
        guard let id = selectedChannelID, account != nil else { return }
        Task { [weak self] in
            guard let self else { return }
            let snapshot = try? await api.unreadSpaces()
            guard generation == attempt, unreadVisitToken == token, selectedChannelID == id else { return }
            unreadVisit = snapshot?.spaces.flatMap(\.channels).first { $0.id == id }
            // A failed/old-server snapshot deliberately yields no divider, not a guessed boundary.
            unreadVisitReady = true
            markSelectedChannelRead()
        }
    }

    public func channelNewMessagesDividerID() -> String? {
        guard selectedDirectMessageID == nil, visit == nil, selectedChannelID == chat.channelID,
              let unreadVisit, let boundary = unreadVisit.readSeq else { return nil }
        return ChannelUnreadDivider.messageID(messages: chat.channelMessages, readSeq: boundary,
            lastSeq: unreadVisit.lastSeq, hasMore: chat.hasMore, viewerID: account?.id,
            blocked: chat.blockedAuthorIDs)
    }
    private var directMessageSnapshots: [DirectMessageConversation] = []
    public var directMessages: [DirectMessageConversation] {
        get {
            directMessageSnapshots.map { snapshot in
                let conversation = profileUpdates.apply(snapshot)
                guard let account, conversation.peer.id == account.id, let username = account.username, let name = account.displayName else { return conversation }
                let picture = account.ownPicture(asset: conversation.peer.avatarAsset, url: conversation.peer.avatarUrl)
                return DirectMessageConversation(id: conversation.id,
                    peer: DirectMessagePeer(admin: conversation.peer.admin, id: conversation.peer.id, username: username, displayName: name, avatarId: conversation.peer.avatarId, team: conversation.peer.team,
                                            avatarAsset: picture.asset, avatarUrl: picture.url),
                    lastSeq: conversation.lastSeq, readSeq: conversation.readSeq, status: conversation.status, blocked: conversation.blocked)
            }
        }
        set { directMessageSnapshots = newValue }
    }
    public private(set) var directMessagesError: String?
    public var selectedDirectMessageID: String? { didSet { if selectedDirectMessageID != oldValue { removeShownNotifications() } } }
    /// `GET /api/people` for `@` suggestions in DMs; nil until the first load
    /// succeeds. A refresh keeps the previous list until it completes.
    public private(set) var people: [Person]? {
        get { peopleSnapshot.map { list in list.map { profileUpdates.apply($0) } } }
        set { peopleSnapshot = newValue }
    }
    private var peopleSnapshot: [Person]?
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
    public private(set) var blockedAccounts: [BlockedAccount] {
        get { blockedAccountSnapshots.map { profileUpdates.apply($0) } }
        set { blockedAccountSnapshots = newValue }
    }
    private var blockedAccountSnapshots: [BlockedAccount] = []
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
    /// Removes the notification shown for a deleted message (`messageId`),
    /// from the `profiles` feed's `notification.retracted`.
    @ObservationIgnored public var retractNotification: ((String) -> Void)?
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
    /// The feedback team's inbox (`GET /api/feedback`), newest first.
    public private(set) var feedbackReports: [FeedbackReport] {
        get { feedbackReportSnapshots.map { profileUpdates.apply($0) } }
        set { feedbackReportSnapshots = newValue }
    }
    private var feedbackReportSnapshots: [FeedbackReport] = []
    /// `?before=` for the next page; nil when everything is listed.
    public private(set) var feedbackCursor: String?
    public private(set) var feedbackLoading = false
    public private(set) var feedbackLoaded = false
    public private(set) var feedbackError: String?
    @ObservationIgnored private var feedbackRequest = 0
    /// The inbox is open: a pushed page in Browse on narrow layouts, an
    /// expanded sidebar section otherwise.
    public var showingFeedback = false
    /// The inbox lists archived threads instead ("Show archived").
    public private(set) var showingArchivedFeedback = false
    /// A conversation the feedback team is visiting without joining it. It is
    /// open while no channel or DM is selected, and any other navigation ends it.
    public private(set) var visit: FeedbackVisit? {
        didSet {
            guard visit?.channelID != oldValue?.channelID else { return }
            removeShownNotifications()
            watchVisitVoice()
        }
    }
    /// Who is in voice in a visited space channel, which the space's own
    /// rosters don't cover.
    public let visitVoicePresence: VoicePresenceModel
    /// What closing the visit returns to on wider layouts.
    @ObservationIgnored private var visitReturn: (spaceID: String?, channelID: String?, directMessageID: String?)?
    /// Shows the Feedback inbox and opens feedback notifications as visits.
    public var isFeedbackTeam: Bool { account?.isFeedbackTeam == true }
    /// An invite link the system opened the app with, until the join dialog
    /// takes it (see `openInviteLink`). Memory only: never kept across launches.
    public private(set) var openedInviteLink: OpenedInviteLink?
    @ObservationIgnored private var lastOpenedInvite: (code: String, at: Date)?
    @ObservationIgnored private var notificationRouteRevision = 0
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
    /// True from launch or sign-in until the first space and its conversation
    /// are ready (or fail). The loading screen stays up meanwhile, so launch
    /// goes straight to the restored conversation instead of flashing Browse
    /// and an empty stage. Capped by `restoreLimit` so a slow network never
    /// hides the workspace.
    public private(set) var restoringWorkspace = false
    static let restoreLimit: Duration = .seconds(5)
    public var spacesError: String?
    public var navigationOpen = false {
        didSet { if !navigationOpen { markSelectedDirectRead() } }
    }
    /// Phone-width windows slide Browse over the conversation; wider ones show
    /// both, so an open Browse only hides the conversation on narrow layouts.
    public var narrowLayout = false {
        didSet { if narrowLayout != oldValue { markSelectedDirectRead() } }
    }
    private var browseCoversConversation: Bool { narrowLayout && navigationOpen }
    public private(set) var unreadSpaces: [UnreadSpace] = []
    public private(set) var feedbackWaiting: Int = 0
    public func channelUnread(_ id: String) -> Bool { unreadSpaces.contains { $0.channels.contains { $0.id == id } } }
    @ObservationIgnored private var unreadRevision = 0
    @ObservationIgnored private var unreadRefreshScheduled = false
    @ObservationIgnored private var channelReads: [String: String] = [:]
    public var openingSpaceID: String?
    public var openingChannelID: String?
    /// Voice availability by media root ("general" or a channel id); nil while unchecked.
    public private(set) var voiceAvailability: [String: Bool] = [:]
    public private(set) var pendingVoiceChannelID: String?
    public private(set) var pendingVoiceStartedAt: Int64 = 0
    public var navigationError: String?
    public let api: APIClient
    public let whatsNew: WhatsNewModel
    public let chat: ChatModel
    public let voice: VoiceClient
    public let presence: PresenceModel
    public let voicePresence: VoicePresenceModel
    private let preferredInitialSpaceID: String?
    private var generation = 0
    private var navigationGeneration = 0
    private var openingForward = false
    private var profileMessageDrafts: [String: ProfileMessageDraft] = [:]
    private var voiceJoinGeneration = 0
    private var peopleRequest = 0
    private var navigationCacheEpoch = 0
    private var navigationTarget: (space: Space, channelID: String?)?
    private var openingDirectID: String?
    private var directHistoryTask: Task<PreparedNavigation, Error>?
    private struct PreparedNavigation {
        let detail: SpaceDetail?
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
    /// The live `profiles` feed's own socket, open while signed in.
    @ObservationIgnored private lazy var profileGateway = Gateway(baseURL: api.baseURL, token: { [api] in await api.authorizationToken() }) { _, _ in }
    /// Keeps opening and closing the feed in order across sign-in and logout.
    @ObservationIgnored private var profileFeedTask: Task<Void, Never>?
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
        whatsNew = WhatsNewModel(api: api)
        self.preferredInitialSpaceID = preferredInitialSpaceID
        let chatModel = ChatModel(api: api)
        let voiceClient = VoiceClient(api: api)
        chat = chatModel
        voice = voiceClient
        presence = PresenceModel(api: api)
        voicePresence = VoicePresenceModel(api: api)
        visitVoicePresence = VoicePresenceModel(api: api)
        chatModel.onReadCursor = { [weak self] in
            self?.markSelectedDirectRead()
            self?.scheduleUnreadRefresh()
        }
        // A reply from the team can answer a report: refresh the inbox's status.
        chatModel.onMessageSent = { [weak self] in
            guard let self, self.isFeedbackTeam, self.visit != nil || self.feedbackLoaded else { return }
            Task { await self.refreshFeedback() }
        }
        chatModel.viewerAccountID = { [weak self] in self?.account?.id }
        chatModel.viewerProfile = { [weak self] in self?.account }
        chatModel.profileDirectory = { [weak self] in self?.profileUpdates ?? ProfileDirectory() }
        chatModel.onOpenForward = { [weak self] message in await self?.openForwardedConversation(message) ?? false }
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
            let next: Phase = account == nil ? .signedOut : needsProfile ? .onboarding : .ready
            if next == .ready { restoringWorkspace = true }
            phase = next
            // A push tapped while signed out never opens for whoever signs in next.
            if account == nil { pendingNotificationRoute = nil }
            if phase == .ready { await openWorkspace(generation: attempt) }
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
            let next: Phase = self.needsProfile ? .onboarding : .ready
            if next == .ready { self.restoringWorkspace = true }
            self.phase = next
            if self.phase == .ready { await self.openWorkspace(generation: attempt) }
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
            else { self.refreshOwnIdentity(account) }
        }
    }

    /// Own profile refresh (docs/media.md): retained self identities read
    /// `account` (members, invitations, self-DMs, authors, pins, threads,
    /// forwards, replies, reactions, profile cards, the voice roster); the
    /// open conversation's session author and self-DM title follow it here
    /// without reopening chat or touching drafts.
    private func refreshOwnIdentity(_ account: Account) {
        chat.updateAuthor(account: account)
        if selectedDirectMessage?.peer.id == account.id {
            chat.channelName = account.displayName ?? chat.channelName
        }
    }

    /// Uploads a prepared picture as the account's profile picture. The
    /// server applies it on completion; the returned account then replaces
    /// the signed-in identity everywhere in this client.
    public func changeAvatar(_ picture: PreparedPicture) async throws {
        guard let current = account else { return }
        let attempt = generation
        let updated = try await ProfilePictureUploader.uploadAvatar(picture, api: api)
        guard generation == attempt, account?.id == current.id, updated.id == current.id else { throw CancellationError() }
        account = updated
        refreshOwnIdentity(updated)
    }

    /// A retained member as currently known: someone's live profile update,
    /// then your own account for your own row.
    public func profiled(_ member: Member) -> Member { profileUpdates.apply(member).withProfile(account) }

    /// A request whose result lists people's current profiles. They become
    /// the newest known state for those IDs, repairing any missed
    /// `profile.updated`, unless an event for someone arrived meanwhile.
    private func fetchingProfiles<T>(_ request: () async throws -> T, people: (T) -> [Person]) async throws -> T {
        let mark = profileUpdates.mark
        let value = try await request()
        var next = profileUpdates
        next.recordFetched(people(value), since: mark)
        if next != profileUpdates {
            profileUpdates = next
            refreshDirectTitle()
        }
        return value
    }

    /// An open DM is titled with the other person's name: a repaired profile
    /// (a missed `profile.updated`) renames its header too, not only its row.
    private func refreshDirectTitle() {
        guard let conversation = selectedDirectMessage, chat.channelID == conversation.id,
              chat.channelName != conversation.peer.displayName else { return }
        chat.channelName = conversation.peer.displayName
    }

    /// Opens (or reopens after sign-in) the live `profiles` feed. An older
    /// gateway without it leaves things as they were: changes then show on
    /// the next load, as before.
    private func startProfileUpdates() {
        let gateway = profileGateway, previous = profileFeedTask
        let handler: Gateway.Handler = { [weak self] event in self?.receiveProfileFeed(event) }
        profileFeedTask = Task {
            await previous?.value
            await gateway.subscribeProfiles(handler: handler)
        }
    }

    private func stopProfileUpdates() {
        profileUpdates = ProfileDirectory()
        let gateway = profileGateway, previous = profileFeedTask
        profileFeedTask = Task {
            await previous?.value
            await gateway.unsubscribe(Gateway.profilesSubscriptionID)
        }
    }

    func receiveProfileFeed(_ raw: [String: Any]) {
        guard account != nil, let event = ProfileFeedEvent.parse(raw) else { return }
        switch event {
        case .person(let person): applyProfileUpdate(person)
        case .space(let space): applySpaceUpdate(space)
        case .notificationRetracted(let messageID): retractNotification?(messageID)
        }
    }

    /// `profile.updated`: your own account (changed on another device) runs
    /// the own-profile refresh; anyone else's newest name, username and
    /// picture overlay every retained copy of them. Text and cursors stay.
    func applyProfileUpdate(_ person: Person) {
        guard let current = account else { return }
        if person.id == current.id {
            let updated = current.applying(person)
            guard updated != current else { return }
            account = updated
            refreshOwnIdentity(updated)
            return
        }
        profileUpdates.record(person)
        // An open DM is titled with the other person's name.
        if selectedDirectMessage?.peer.id == person.id { chat.channelName = person.displayName }
    }

    /// `space.updated`: the space's name and icon in the rail, invitations,
    /// the open space and remembered navigation, and its name on a feedback
    /// visit and inbox rows. Spaces you don't have are ignored.
    func applySpaceUpdate(_ update: Space) {
        guard spaces.contains(where: { $0.id == update.id }) || invitations.contains(where: { $0.id == update.id })
                || detailSnapshot?.space.id == update.id || notificationInvitation?.id == update.id
                || visit?.spaceID == update.id || feedbackReportSnapshots.contains(where: { $0.space?.id == update.id }) else { return }
        visit = visit?.applying(update)
        feedbackReportSnapshots = feedbackReportSnapshots.map { $0.applying(update) }
        spaces = spaces.map { $0.applying(update) }
        invitations = invitations.map { $0.applying(update) }
        notificationInvitation = notificationInvitation?.applying(update)
        detailSnapshot = detailSnapshot?.applying(update)
        visited = visited.mapValues { PreparedNavigation(detail: $0.detail?.applying(update), channelID: $0.channelID, history: $0.history) }
        chat.renameSpace(id: update.id, name: update.name)
    }

    /// Back to the same saved mascot everywhere.
    public func removeAvatar(restore: Bool = false) async throws {
        guard let current = account else { return }
        let attempt = generation
        let updated: Account
        if restore { updated = try await api.restoreAvatar(avatarAsset: current.avatarAsset) }
        else { updated = try await api.removeAvatar() }
        guard generation == attempt, account?.id == current.id, updated.id == current.id else { throw CancellationError() }
        account = updated
        refreshOwnIdentity(updated)
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
        profileMessageDrafts.values.forEach { $0.discard() }
        profileMessageDrafts = [:]
        stopProfileUpdates()
        account = nil; spaces = []; invitations = []; pendingMembers = []; detail = nil
        notificationInvitation = nil; notificationChannelInvitation = nil
        directMessages = []; directMessagesError = nil; selectedDirectMessageID = nil; people = nil; messageRequestsOpen = nil
        blockedAccounts = []; blockedIDs = []; blocksLoaded = false; blocksError = nil
        directMessagePrivacy = nil; privacyError = nil; chat.setBlockedAuthors([])
        clearFeedbackState()
        clearNotificationState()
        spacesLoaded = false; spacesError = nil
        selectedSpaceID = nil; selectedChannelID = nil; challengeID = nil
        navigationGeneration += 1
        navigationTarget = nil; navigationError = nil
        openingSpaceID = nil; openingChannelID = nil
        limits = nil; navigationOpen = false; restoringWorkspace = false
        openedInviteLink = nil; lastOpenedInvite = nil
        busy = false; phase = .signedOut
        async let revoke: Void = api.logout()
        await chat.stop(discardingDrafts: true)
        await presence.stop()
        await voicePresence.stop()
        do { try await revoke } catch { self.error = FriendlyError.message(for: error) }
    }

    /// The first space list and its landing conversation, behind the loading screen.
    private func openWorkspace(generation attempt: Int) async {
        let limit = Task { [weak self] in
            try? await Task.sleep(for: Self.restoreLimit)
            guard !Task.isCancelled, let self, self.generation == attempt else { return }
            self.restoringWorkspace = false
        }
        await loadSpaces()
        limit.cancel()
        if generation == attempt { restoringWorkspace = false }
        startDirectMessageRefresh()
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
            if case let .feedback(target)? = route, self.generation == attempt {
                await self.openFeedback(target)
            }
            if case .invitation? = route, let route, self.generation == attempt {
                await self.open(route)
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
            if navigationTarget != nil || openingDirectID != nil || busy {
                navigationGeneration += 1
                navigationTarget = nil
                directHistoryTask?.cancel(); directHistoryTask = nil; openingDirectID = nil
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
            let conversations = try await fetchingProfiles({ try await api.directMessages() }, people: { $0.map(\.peer.profile) })
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
            let loaded = try await fetchingProfiles({ try await api.people() }, people: { $0 })
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

    func profileMessageDraft(personID: String) -> ProfileMessageDraft {
        if let draft = profileMessageDrafts[personID] { return draft }
        let draft = ProfileMessageDraft()
        profileMessageDrafts[personID] = draft
        return draft
    }

    /// Only explicit Send resolves a DM. Incoming requests stay read-only.
    /// Nothing touches the active
    /// ChatModel's composer or navigation until the message is confirmed.
    func sendProfileMessage(to person: MentionCardPerson) async -> Bool {
        let draft = profileMessageDraft(personID: person.id)
        guard let account, !person.isSelf, let username = person.username, draft.canSend else { return false }
        guard !blockedIDs.contains(person.id) else {
            draft.error = DirectMessageErrors.message(code: "dm_blocked")
            return false
        }
        let attempt = generation
        draft.sending = true; draft.error = nil
        var attemptedSend = false
        defer { draft.sending = false; draft.progress = nil }
        do {
            let conversation: DirectMessageConversation
            if let known = directMessages.first(where: { $0.peer.id == person.id }) ?? draft.conversation { conversation = known }
            else {
                // POST /api/dms accepts an existing incoming request. Look it
                // up first so profile Send never silently accepts one.
                let conversations = try await fetchingProfiles({ try await api.directMessages() }, people: { $0.map(\.peer.profile) })
                try draft.checkActive()
                guard generation == attempt else { throw CancellationError() }
                directMessages = conversations
                if let known = conversations.first(where: { $0.peer.id == person.id }) { conversation = known }
                else { conversation = try await api.createDirectMessage(username: username) }
            }
            try draft.checkActive()
            guard generation == attempt else { throw CancellationError() }
            // A stale username must never send this person's draft to a new owner.
            guard conversation.peer.id == person.id else {
                throw UserFacingError(message: "This username no longer belongs to this person. Reopen their profile and try again.")
            }
            if conversation.blocked { throw APIError(status: 403, message: DirectMessageErrors.message(code: "dm_blocked")!, code: "dm_blocked") }
            if conversation.status == .incoming {
                throw UserFacingError(message: "Open conversation and accept this message request before sending.")
            }
            draft.conversation = conversation
            replaceDirectMessage(conversation)
            let session = try await api.chatSession(name: account.displayName ?? "")
            try draft.checkActive()
            guard generation == attempt, session.author.id == account.id, !session.author.isGuest else { throw CancellationError() }
            if draft.pending == nil {
                let attachments = try await draft.upload(channelID: conversation.id, api: api)
                try draft.checkActive()
                draft.pending = PendingMessage(id: UUID().uuidString.lowercased(), text: draft.text, attachments: attachments)
            }
            let command = draft.pending!
            draft.progress = "Sending…"
            attemptedSend = true
            let message = try await api.send(channelID: conversation.id, sessionToken: session.token, clientMessageID: command.id,
                                             text: command.text, attachmentIDs: command.attachmentIDs,
                                             spoilerAttachmentIDs: command.attachments.filter(\.spoiler).map(\.id))
            try draft.checkActive()
            guard generation == attempt else { throw CancellationError() }
            guard MessageValidation.acceptsResponse(message, channelID: conversation.id, command: command, authorID: account.id),
                  (message.content.attachments ?? []).map(\.id) == command.attachmentIDs else {
                throw APIError(status: 502, message: "The chat service returned an invalid message.")
            }
            draft.clear()
            chat.receiveProfileMessage(message)
            invalidateNavigation(channelID: conversation.id)
            await select(directMessage: conversation)
            return generation == attempt
        } catch {
            guard generation == attempt, !draft.discarded else { return false }
            let refusal = error as? APIError
            if attemptedSend, let refusal, [400, 401, 403, 404, 409, 413, 422].contains(refusal.status) {
                draft.pending = nil
                draft.error = FriendlyError.message(for: refusal)
            } else if draft.pending != nil {
                draft.error = "Send outcome is unknown. Retry Send safely resends the same message. \(FriendlyError.message(for: error))"
            } else { draft.error = FriendlyError.message(for: error) }
            if refusal?.code == "dm_blocked" { Task { await refreshBlocks(); await refreshDirectMessages() } }
            return false
        }
    }

    /// Whether `sendUpdateReport` can post: signed in with a username.
    public var canSendUpdateReport: Bool { account?.username != nil }

    /// Posts a failed update's error as `@feedback` in the account's notes, where
    /// the Caper team can read it and reply. Never navigates. Returns a message
    /// to show on failure, or nil once the message is sent.
    public func sendUpdateReport(error report: String, installed: String, target: String) async -> String? {
        guard let account, let username = account.username else { return "Sign in to send this to the Caper team." }
        do {
            let conversation: DirectMessageConversation
            if let known = directMessages.first(where: { $0.peer.id == account.id }) { conversation = known }
            else { conversation = try await api.createDirectMessage(username: username) }
            guard conversation.peer.id == account.id else { return "Couldn’t open your notes. Try again." }
            replaceDirectMessage(conversation)
            let session = try await api.chatSession(name: account.displayName ?? "")
            let text = "@feedback My Caper update didn’t finish.\n\n\(report)\n\n\(installed) → Caper \(target) on macOS"
            let message = try await api.send(channelID: conversation.id, sessionToken: session.token,
                                             clientMessageID: UUID().uuidString.lowercased(), text: text)
            chat.receiveProfileMessage(message)
            return nil
        } catch {
            return FriendlyError.message(for: error)
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

    public func select(directMessage conversation: DirectMessageConversation, contextHistory: ChatHistory? = nil) async {
        if openingDirectID == conversation.id { return }
        directHistoryTask?.cancel(); directHistoryTask = nil; openingDirectID = nil
        if contextHistory == nil, selectedDirectMessageID == conversation.id, !chat.loadFailed,
           chat.isPreview == (conversation.status == .incoming) {
            navigationGeneration += 1
            navigationTarget = nil; navigationError = nil
            openingSpaceID = nil; openingChannelID = nil
            navigationOpen = false
            return
        }
        rememberCurrentNavigation()
        let attempt = generation
        navigationGeneration += 1
        let navigation = navigationGeneration
        let epoch = navigationCacheEpoch
        openingDirectID = conversation.id
        navigationTarget = nil; navigationError = nil
        openingSpaceID = nil; openingChannelID = nil
        defer {
            if navigationGeneration == navigation { openingDirectID = nil; directHistoryTask = nil }
        }
        do {
            let key = navigationKey(spaceID: "", channelID: conversation.id)
            let prepared: PreparedNavigation
            if let contextHistory { prepared = PreparedNavigation(detail: nil, channelID: conversation.id, history: contextHistory) }
            else if let retained = visited[key] { prepared = retained }
            else {
                prefetch(directMessage: conversation)
                let entry = prefetches[key]!
                removePrefetch(key)
                directHistoryTask = entry.task
                prepared = try await entry.task.value
            }
            guard generation == attempt, navigationGeneration == navigation, navigationCacheEpoch == epoch else { return }
            guard let history = prepared.history,
                  history.space?.id == "", history.channel?.id == conversation.id,
                  history.channel?.direct == true else {
                throw APIError(status: 502, message: "Caper returned another conversation.")
            }
            rememberCurrentNavigation()
            remember(prepared)
            await presence.stop(); await voicePresence.stop()
            guard generation == attempt, navigationGeneration == navigation, navigationCacheEpoch == epoch else { return }
            selectedDirectMessageID = conversation.id; selectedChannelID = nil
            visit = nil; visitReturn = nil
            navigationOpen = false
            Task { [weak self] in await self?.refreshPeople() }
            // An incoming request opens read-only: no chat session, no composer,
            // no read cursor and no chimes. Opening it never accepts it.
            let current = directMessages.first { $0.id == conversation.id } ?? conversation
            chat.directPeerID = current.peer.id == account?.id ? nil : current.peer.id
            if current.status == .incoming { await chat.preview(history: history) }
            else { await chat.open(history: history, displayName: account?.displayName ?? "") }
            guard generation == attempt, navigationGeneration == navigation else { return }
            markSelectedDirectRead()
        } catch is CancellationError {} catch {
            guard generation == attempt, navigationGeneration == navigation else { return }
            if let error = error as? APIError, [401, 403, 404].contains(error.status) { invalidateNavigation(channelID: conversation.id) }
            navigationError = FriendlyError.message(for: error)
        }
    }

    public func openForwardedConversation(_ wrapper: ChatMessage) async -> Bool {
        guard !openingForward else { return false }
        openingForward = true
        defer { openingForward = false }
        let attempt = generation; let navigation = navigationGeneration
        do {
            let conversation = try await chat.forwardedConversation(message: wrapper)
            guard generation == attempt, navigationGeneration == navigation else { return false }
            guard let source = conversation.source else {
                chat.forwardConversationTarget = wrapper
                return false
            }
            guard source.message.id == conversation.root?.id, source.message.channelId == conversation.root?.channelId else {
                throw APIError(status: 502, message: "Conversation is unavailable.")
            }
            if chat.channelID != source.message.channelId {
                if let spaceID = source.spaceId {
                    // navigate(space:) rechecks membership itself; fetch only a space this list lacks.
                    let space: Space
                    if let known = spaces.first(where: { $0.id == spaceID }) { space = known } else {
                        space = try await api.space(spaceID).space
                        guard generation == attempt, navigationGeneration == navigation else { return false }
                    }
                    await navigate(space: space, channelID: source.message.channelId)
                } else {
                    let conversations = try await fetchingProfiles({ try await api.directMessages() }, people: { $0.map(\.peer.profile) })
                    guard generation == attempt, navigationGeneration == navigation else { return false }
                    guard let direct = conversations.first(where: { $0.id == source.message.channelId }) else {
                        throw APIError(status: 404, message: "Conversation is unavailable.")
                    }
                    directMessages = conversations
                    await select(directMessage: direct)
                }
                guard generation == attempt, navigationGeneration == navigation + 1 else { return false }
            }
            guard chat.channelID == source.message.channelId else { return false }
            guard await chat.goToMessage(source.message) else { return false }
            if source.message.threadRootId == nil { await chat.openThread(source.message.id) }
            return true
        } catch is CancellationError { return false }
        catch {
            if generation == attempt, navigationGeneration == navigation { navigationError = "Couldn’t open the conversation. Try again." }
            return false
        }
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
        invalidateNavigation(channelID: conversation.id)
        directMessages.removeAll { $0.id == conversation.id }
        if selectedDirectMessageID == conversation.id { await leaveRequest() }
    }

    /// Blocks an account everywhere you share. A pending request from them is
    /// declined server-side, so it leaves the requests list too.
    public func block(_ target: BlockTarget) async throws {
        guard let account else { return }
        guard target.canBlock else { throw UserFacingError(message: "Caper admins and the Caper team can't be blocked.") }
        guard target.id != account.id else { throw UserFacingError(message: "You can't block yourself.") }
        let attempt = generation
        try await api.block(accountID: target.id)
        guard generation == attempt else { return }
        let leavingRequest = selectedDirectMessage.map { $0.peer.id == target.id && $0.status == .incoming } ?? false
        for request in directMessages where request.peer.id == target.id && request.status == .incoming {
            invalidateNavigation(channelID: request.id)
        }
        if !blockedIDs.contains(target.id) {
            blockedAccounts.insert(BlockedAccount(id: target.id, username: target.username ?? "", displayName: target.displayName,
                                                  avatarId: target.avatarId, avatarAsset: target.avatarAsset, avatarUrl: target.avatarUrl), at: 0)
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
            let blocks = try await fetchingProfiles({ try await api.blocks() }, people: { $0.map(\.profile) })
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
            chat.refreshDraftUploads()
            markSelectedDirectRead()
            removeShownNotifications()
            Task { await refreshVoiceActivity(); await refreshDirectMessages(); await refreshUnreadSpaces() }
            refreshNotificationSettingsIfStale()
            refreshMembership()
        }
    }

    /// Android clears the open conversation's notifications; so does iOS.
    private func removeShownNotifications() {
        guard foreground, account != nil, let id = selectedDirectMessageID ?? selectedChannelID ?? visit?.channelID else { return }
        removeDeliveredNotifications?(id)
    }

    private func markSelectedDirectRead() {
        markSelectedChannelRead()
        guard foreground, !browseCoversConversation, account != nil, let id = selectedDirectMessageID,
              selectedDirectMessage?.status != .incoming,
              let history = chat.currentSnapshot(), history.channel?.id == id, history.hasNewer != true,
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

    /// Live notifications refresh unread state between polls; a burst of them
    /// coalesces into one request about a second after the first.
    public func scheduleUnreadRefresh() {
        guard account != nil, !unreadRefreshScheduled else { return }
        unreadRefreshScheduled = true
        let attempt = generation
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(1))
            self?.unreadRefreshScheduled = false
            guard self?.generation == attempt else { return }
            await self?.refreshUnreadSpaces()
        }
    }

    private func refreshVoiceActivity() async {
        guard account != nil, !voiceActivityRefreshing else { return }
        voiceActivityRefreshing = true
        defer { voiceActivityRefreshing = false }
        let attempt = generation
        do {
            let snapshot = try await api.spaceVoiceActivity()
            guard generation == attempt, account != nil, !Task.isCancelled else { return }
            voiceActivitySpaceIDs = Set(snapshot.spaceIds)
        } catch {
            guard generation == attempt else { return }
            voiceActivitySpaceIDs = []
        }
    }

    public func refreshUnreadSpaces() async {
        guard account != nil else { return }
        let accountGeneration = generation
        await chat.refreshUnreadThreads()
        guard generation == accountGeneration, account != nil else { return }
        acknowledgeVisibleThread()
        unreadRevision += 1
        let attempt = generation, revision = unreadRevision
        guard let snapshot = try? await api.unreadSpaces(), generation == attempt, unreadRevision == revision else { return }
        unreadSpaces = snapshot.spaces
        feedbackWaiting = snapshot.feedbackWaiting
        markSelectedChannelRead()
    }

    public func threadVisibilityChanged(rootID: String?, seq: String?) {
        chat.visibleThreadReply = rootID.flatMap { root in seq.map { (root, $0) } }
        acknowledgeVisibleThread()
    }

    private func acknowledgeVisibleThread() {
        guard foreground, !browseCoversConversation, account != nil else { return }
        chat.acknowledgeVisibleThread()
    }

    private func markSelectedChannelRead() {
        guard unreadVisitReady, foreground, !browseCoversConversation, account != nil, selectedDirectMessageID == nil,
              let spaceID = detail?.space.id, !spaceID.isEmpty, let id = selectedChannelID,
              let history = chat.currentSnapshot(), history.channel?.id == id, history.hasNewer != true,
              (try? Sequence.compare(history.cursor, channelReads[id] ?? "0")) == .orderedDescending else { return }
        let attempt = generation, cursor = history.cursor
        Task {
            do {
                try await api.markChannelRead(spaceID: spaceID, channelID: id, seq: cursor)
                guard generation == attempt else { return }
                unreadRevision += 1
                if (try? Sequence.compare(cursor, channelReads[id] ?? "0")) == .orderedDescending { channelReads[id] = cursor }
                unreadSpaces = unreadSpaces.compactMap { space in
                    var next = space
                    if space.id == spaceID {
                        next.channels.removeAll { $0.id == id && (try? Sequence.compare($0.lastSeq, cursor)) != .orderedDescending }
                    }
                    return next.channels.isEmpty ? nil : next
                }
            } catch { /* Retry on the next foreground poll or committed cursor. */ }
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
        startProfileUpdates()
        directMessageRefreshTask?.cancel()
        let attempt = generation
        Task { [weak self] in await self?.refreshBlocks() }
        // After sign-in (every path starts here), like the block list.
        Task { [weak self] in await self?.loadNotificationSettings() }
        if isFeedbackTeam { Task { [weak self] in await self?.refreshFeedback() } }
        directMessageRefreshTask = Task { [weak self] in
            // Sign-in has just loaded the spaces; membership waits for the next pass.
            var reconcile = false
            while !Task.isCancelled {
                await self?.refreshVoiceActivity()
                await self?.refreshDirectMessages()
                await self?.refreshUnreadSpaces()
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
        guard let refreshed = try? await fetchingProfiles({ try await api.space(current.space.id) }, people: { $0.profiles }),
              refreshed.space.id == current.space.id,
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

    /// DM hover/focus reads share the channel cache's bounds and account lifetime.
    public func prefetch(directMessage conversation: DirectMessageConversation) {
        let key = navigationKey(spaceID: "", channelID: conversation.id)
        if visited[key] != nil { return }
        if let entry = prefetches[key], entry.expires > Date(), entry.generation == generation { return }
        prefetches[key]?.task.cancel()
        let api = self.api
        let cacheGeneration = generation
        let task = Task<PreparedNavigation, Error> {
            let history = try await api.history(channelID: conversation.id)
            guard history.space?.id == "", history.channel?.id == conversation.id, history.channel?.direct == true else {
                throw APIError(status: 502, message: "Caper returned another conversation.")
            }
            return PreparedNavigation(detail: nil, channelID: conversation.id, history: history)
        }
        let entryID = UUID()
        prefetches[key] = PrefetchEntry(id: entryID, expires: Date().addingTimeInterval(5), generation: cacheGeneration, task: task)
        prefetchOrder.removeAll { $0 == key }; prefetchOrder.append(key)
        while prefetchOrder.count > 4 { prefetches.removeValue(forKey: prefetchOrder.removeFirst())?.task.cancel() }
        Task { [weak self] in
            do { _ = try await task.value }
            catch {
                guard let self, self.generation == cacheGeneration, self.prefetches[key]?.id == entryID else { return }
                self.removePrefetch(key)
                if let failure = error as? APIError, [401, 403, 404].contains(failure.status) { self.invalidateNavigation(channelID: conversation.id) }
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

    private func navigate(space: Space, channelID: String?, contextHistory: ChatHistory? = nil) async {
        directHistoryTask?.cancel(); directHistoryTask = nil; openingDirectID = nil
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
            if contextHistory == nil, let entry = prefetches[key], entry.expires > Date(), entry.generation == generation {
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
            detail = try await self.fetchingProfiles({ try await self.api.space(space.id) }, people: { $0.profiles })
            guard navigationGeneration == navigation, generation == attempt, navigationCacheEpoch == cacheEpoch else { return }
            guard detail.space.id == space.id else { throw APIError(status: 502, message: "The service returned another space.") }
            spaceVerified = true
            let restoredChannelID = channelID ?? prepared?.channelID ?? lastChannelBySpace[space.id]
            let restoredChannel = restoredChannelID.flatMap { id in detail.channels.first { $0.id == id } }
            let channel = channelID == nil ? (restoredChannel?.joined == true ? restoredChannel : detail.channels.first { $0.joined }) : restoredChannel
            if channelID != nil && channel == nil { throw APIError(status: 404, message: "This channel is no longer accessible.") }
            let history: ChatHistory?
            if let channel {
                if let contextHistory { history = contextHistory }
                else if let retained = visitedNavigation(spaceID: space.id, channelID: channel.id)?.history { history = retained }
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
            visit = nil; visitReturn = nil
            self.detail = detail
            lastChannelBySpace[space.id] = channel?.id
            // A space tile changes the channels shown in Browse. Only an
            // explicit channel selection should reveal chat on a phone.
            if channelID != nil || !narrowLayout { navigationOpen = false }
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
            // A private channel's readers lead the member list.
            else { await self.presence.watch(spaceID: detail.space.id, members: detail.memberOrder(channelID: selectedChannelID)) }
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
                if visit != nil { visit = nil; visitReturn = nil }
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
        return visitedOrder.reversed().compactMap { visited[$0] }.first { $0.detail?.space.id == spaceID }
    }
    private func remember(_ value: PreparedNavigation) {
        let key = navigationKey(spaceID: value.detail?.space.id ?? "", channelID: value.channelID)
        visited[key] = value; visitedOrder.removeAll { $0 == key }; visitedOrder.append(key)
        while visitedOrder.count > 20 { visited.removeValue(forKey: visitedOrder.removeFirst()) }
        lastChannelBySpace = lastChannelBySpace.filter { visited[navigationKey(spaceID: $0.key, channelID: $0.value)] != nil }
    }
    private func rememberCurrentNavigation() {
        // A visited conversation is never a space's or the DM list's to restore.
        guard visit == nil, let snapshot = chat.currentSnapshot(), snapshot.hasNewer != true else { return }
        if let id = selectedDirectMessageID, snapshot.channel?.id == id, snapshot.space?.id == "" {
            guard directMessages.contains(where: { $0.id == id }) else { return }
            remember(PreparedNavigation(detail: nil, channelID: id, history: snapshot))
            return
        }
        guard let detail, snapshot.space?.id == detail.space.id else { return }
        lastChannelBySpace[detail.space.id] = snapshot.channel?.id
        remember(PreparedNavigation(detail: detail, channelID: snapshot.channel?.id, history: snapshot))
    }
    private func clearNavigationCache() {
        navigationCacheEpoch += 1
        directHistoryTask?.cancel(); directHistoryTask = nil; openingDirectID = nil
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
        replace(detail: detail.with(space: updated))
    }

    /// Owner only: uploads a prepared picture as the open space's icon,
    /// then shows it in the rail, list and header.
    public func changeSpaceIcon(_ picture: PreparedPicture) async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        guard let detail else { return }
        clearNavigationCache()
        let attempt = generation
        let updated = try await ProfilePictureUploader.uploadSpaceIcon(picture, spaceID: detail.space.id, api: api)
        applySpaceIcon(updated, to: detail, attempt: attempt)
    }

    /// Owner only: back to the space's initial.
    public func removeSpaceIcon() async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        guard let detail else { return }
        clearNavigationCache()
        let attempt = generation
        let updated = try await api.removeSpaceIcon(spaceID: detail.space.id)
        applySpaceIcon(updated, to: detail, attempt: attempt)
    }

    /// The changed space everywhere it shows, even if another space is open by now.
    private func applySpaceIcon(_ updated: Space, to detail: SpaceDetail, attempt: Int) {
        guard generation == attempt, updated.id == detail.space.id else { return }
        // The completion's space may omit fields the list carries (`demo`, `inviter`).
        func merged(_ space: Space) -> Space {
            var space = space
            space.iconAsset = updated.iconAsset
            space.iconUrl = updated.iconUrl
            return space
        }
        if let current = self.detail, current.space.id == updated.id {
            replace(detail: current.with(space: merged(current.space)))
        } else {
            spaces = spaces.map { $0.id == updated.id ? merged($0) : $0 }
        }
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

    public func addSpaceMember(username: String, spaceID: String? = nil) async throws {
        guard let spaceID = spaceID ?? detail?.space.id else { return }
        if let error = WorkspaceValidation.usernameError(username) { throw APIError(status: 400, message: error) }
        let attempt = generation
        let member = try await api.addSpaceMember(spaceID: spaceID, username: username)
        guard generation == attempt else { throw CancellationError() }
        if self.detail?.space.id == spaceID {
            pendingMembers.removeAll { $0.id == member.id }
            pendingMembers.append(member)
        }
    }

    public func loadSpaceInvitations() async throws {
        guard let id = detail?.space.id, isOwner else { pendingMembers = []; return }
        let attempt = generation
        let members = try await fetchingProfiles({ try await api.spaceInvitations(spaceID: id) }, people: { $0.map(\.profile) })
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

    // MARK: Invite links

    /// The shareable URL for a link code, on the configured server's origin.
    public func inviteLinkURL(_ code: String) -> String { InviteLinks.url(code: code, baseURL: api.baseURL) }

    /// Opens "Join with invite link" for a link from another app
    /// (`caper://invite/<code>`, or `https://caper.chat/invite/<code>` on iPhone)
    /// with its preview loading. Signed out or before the profile is finished,
    /// the link waits for sign-in and onboarding. A newer link replaces a
    /// waiting one; anything that is not an invite link is ignored.
    public func openInviteLink(_ url: URL) {
        guard let code = InviteLinks.code(opening: url) else { return }
        // One universal link can arrive both as a URL and as a browsing activity.
        let now = Date()
        if let last = lastOpenedInvite, last.code == code, now.timeIntervalSince(last.at) < 1 { return }
        lastOpenedInvite = (code, now)
        openedInviteLink = OpenedInviteLink(link: url.absoluteString)
    }

    /// The opened link, once the workspace (signed in, profile finished,
    /// spaces loaded) can show its dialog.
    public var deliverableInviteLink: OpenedInviteLink? {
        guard phase == .ready, account != nil, spacesLoaded, !restoringWorkspace else { return nil }
        return openedInviteLink
    }

    /// Hands the opened link to the dialog that shows it, exactly once.
    public func takeInviteLink() -> OpenedInviteLink? {
        guard let link = deliverableInviteLink else { return nil }
        openedInviteLink = nil
        return link
    }

    public func invitePreview(code: String) async throws -> InvitePreview {
        let attempt = generation
        let preview = try await api.invitePreview(code: code)
        guard generation == attempt else { throw CancellationError() }
        return preview
    }

    /// Joins by link (or finds the space already joined), refreshes the space
    /// list and opens the link's channel, or the space's default one.
    public func joinInvite(code: String) async throws {
        membershipChanges += 1; defer { membershipChanges -= 1; membershipRevision += 1 }
        clearNavigationCache()
        let attempt = generation
        let result = try await api.joinInvite(code: code)
        guard generation == attempt, account != nil else { throw CancellationError() }
        // A failed refresh still opens the joined space; the periodic reconcile catches up.
        if let response = try? await api.spaces(), generation == attempt {
            limits = response.limits
            spaces = response.spaces
            invitations = response.invitations
            spacesLoaded = true
        }
        guard generation == attempt, account != nil else { throw CancellationError() }
        invitations.removeAll { $0.id == result.space.id }
        let space = spaces.first { $0.id == result.space.id } ?? result.space
        if !spaces.contains(where: { $0.id == space.id }) { spaces.append(space) }
        await navigate(space: space, channelID: result.channelId)
    }

    /// The open space's live links, newest first (owners only).
    public func loadInviteLinks() async throws -> [InviteLink] {
        guard let id = detail?.space.id, isOwner else { return [] }
        let attempt = generation
        let links = try await api.inviteLinks(spaceID: id)
        guard generation == attempt, detail?.space.id == id else { throw CancellationError() }
        return links
    }

    public func defaultInviteLink(channelID: String? = nil) async throws -> InviteLink {
        guard let id = detail?.space.id, isOwner else { throw CancellationError() }
        let attempt = generation
        let link = try await api.defaultInviteLink(spaceID: id, channelID: channelID)
        guard generation == attempt, detail?.space.id == id else { throw CancellationError() }
        return link
    }

    public func createInviteLink(channelID: String?, maxAge: Int?, maxUses: Int?) async throws -> InviteLink {
        guard let id = detail?.space.id, isOwner else { throw CancellationError() }
        let attempt = generation
        let link = try await api.createInviteLink(spaceID: id, channelID: channelID, maxAge: maxAge, maxUses: maxUses)
        guard generation == attempt, detail?.space.id == id else { throw CancellationError() }
        return link
    }

    public func regenerateInviteLink(_ current: InviteLink) async throws -> InviteLink {
        guard let id = detail?.space.id, isOwner else { throw CancellationError() }
        let attempt = generation
        let link = try await api.regenerateInviteLink(spaceID: id, code: current.code)
        guard generation == attempt, detail?.space.id == id else { throw CancellationError() }
        return link
    }

    public func revokeInviteLink(_ link: InviteLink) async throws {
        guard let id = detail?.space.id, isOwner else { throw CancellationError() }
        let attempt = generation
        try await api.revokeInviteLink(spaceID: id, code: link.code)
        guard generation == attempt, detail?.space.id == id else { throw CancellationError() }
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
        detail = detail.with(members: detail.members.filter { $0.id != member.id })
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
        detail = detail.with(channels: detail.channels + [channel])
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
        detail = detail.with(channels: detail.channels.map { $0.id == updated.id ? updated : $0 })
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
        detail = detail.with(channels: detail.channels.filter { $0.id != channel.id })
        replace(detail: detail)
        if selectedChannelID == channel.id {
            if let first = detail.channels.first(where: \.joined) { await select(channel: first) }
            else { selectedChannelID = nil; await chat.stop() }
        }
    }

    public func channelMembers(_ channel: Channel) async throws -> [Member] {
        guard let spaceID = detail?.space.id else { return [] }
        let attempt = generation
        let members = try await fetchingProfiles({ try await api.channelMembers(spaceID: spaceID, channelID: channel.id) }, people: { $0.map(\.profile) })
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        return members.map { profiled($0) }
    }

    public func channelInvitations(_ channel: Channel) async throws -> [Member] {
        guard let spaceID = detail?.space.id else { return [] }
        let attempt = generation
        let invitations = try await fetchingProfiles({ try await api.channelInvitations(spaceID: spaceID, channelID: channel.id) }, people: { $0.map(\.profile) })
        guard generation == attempt, detail?.space.id == spaceID else { throw CancellationError() }
        return invitations.map { profiled($0) }
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
            replace(detail: current.with(channels: channels))
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
        let refreshed = try await fetchingProfiles({ try await api.space(spaceID) }, people: { $0.profiles })
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
        // A DM or a feedback visit stays open when the space behind it goes.
        let directMessageOpen = selectedDirectMessageID != nil || visit != nil
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
        if isVisitVoice(channel) {
            if visitVoicePresence.unavailableChannels.contains(channel.id) { return false }
            return voiceAvailability[channel.id]
        }
        guard let detail, detail.channels.contains(where: { $0.id == channel.id }) else { return nil }
        if voicePresence.unavailableChannels.contains(channel.id) { return false }
        return voiceAvailability[detail.space.demo == true ? "general" : channel.id]
    }

    public func refreshVoiceAvailability(channel: Channel? = nil) async {
        if let channel, isVisitVoice(channel) {
            let accountGeneration = generation
            let enabled = (try? await api.mediaStatus(channelID: channel.id)) ?? false
            guard !Task.isCancelled, generation == accountGeneration, visit?.channelID == channel.id else { return }
            voiceAvailability[channel.id] = enabled
            return
        }
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
        if isVisitVoice(channel), let visit { await joinVisitVoice(visit); return }
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
        route.isOpen(spaceID: selectedSpaceID, channelID: selectedChannelID, directMessageID: selectedDirectMessageID,
                     visitChannelID: visit?.channelID)
    }

    /// Opens a tapped push's channel or DM. Before spaces load, it waits and
    /// opens in place of the usual first space.
    public func open(_ route: NotificationRoute) async {
        guard phase != .signedOut else { return }
        notificationRouteRevision += 1
        let routeRevision = notificationRouteRevision
        notificationInvitation = nil; notificationChannelInvitation = nil
        guard account != nil, spacesLoaded else {
            pendingNotificationRoute = route
            return
        }
        switch route {
        case let .direct(conversationID): await openDirectMessage(id: conversationID)
        case let .channel(spaceID, channelID): await openChannel(spaceID: spaceID, channelID: channelID)
        case let .feedback(target): await openFeedback(target)
        case let .invitation(spaceID, channelID):
            let attempt = generation
            let navigation = navigationGeneration
            do {
                let response = try await api.spaces()
                guard generation == attempt, notificationRouteRevision == routeRevision, navigationGeneration == navigation else { return }
                spaces = response.spaces; invitations = response.invitations; limits = response.limits
                if let channelID {
                    guard let space = spaces.first(where: { $0.id == spaceID }) else {
                        error = "This invitation is no longer available."; return
                    }
                    let detail = try await api.space(spaceID)
                    guard generation == attempt, notificationRouteRevision == routeRevision, navigationGeneration == navigation else { return }
                    guard let invite = detail.channelInvitations.first(where: { $0.channel.id == channelID }) else {
                        error = "This invitation is no longer available."; return
                    }
                    await select(space: space)
                    guard generation == attempt, notificationRouteRevision == routeRevision,
                          navigationGeneration == navigation + 1, selectedSpaceID == spaceID else { return }
                    notificationChannelInvitation = invite
                } else {
                    notificationInvitation = invitations.first(where: { $0.id == spaceID })
                    if notificationInvitation == nil { error = "This invitation is no longer available." }
                }
            } catch {
                if generation == attempt, notificationRouteRevision == routeRevision, navigationGeneration == navigation {
                    self.error = FriendlyError.message(for: error)
                }
            }
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
        voiceActivitySpaceIDs = []
        unreadSpaces = []; feedbackWaiting = 0; channelReads = [:]; unreadRevision += 1
        unreadVisitToken = UUID(); unreadVisit = nil; unreadVisitReady = false
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

// MARK: Feedback

extension AppModel {
    /// The inbox's first page, replacing what is listed. On open, on pull to
    /// refresh, after a team reply and when a `feedback` notification arrives.
    public func refreshFeedback() async {
        guard account != nil, isFeedbackTeam else { return }
        await loadFeedback(before: nil)
    }

    /// The next 50 older reports.
    public func loadMoreFeedback() async {
        guard account != nil, isFeedbackTeam, !feedbackLoading, let cursor = feedbackCursor else { return }
        await loadFeedback(before: cursor)
    }

    /// A live `feedback` notification: a new report is waiting.
    public func feedbackNotificationArrived() {
        guard isFeedbackTeam else { return }
        Task { [weak self] in await self?.refreshFeedback() }
    }

    private func loadFeedback(before: String?) async {
        let attempt = generation
        feedbackRequest += 1
        let request = feedbackRequest
        feedbackLoading = true
        defer { if generation == attempt, feedbackRequest == request { feedbackLoading = false } }
        do {
            let page = try await api.feedbackReports(before: before, archived: showingArchivedFeedback)
            guard generation == attempt, feedbackRequest == request, account != nil else { return }
            feedbackReports = before == nil ? page.reports : FeedbackInbox.merging(feedbackReports, page.reports)
            feedbackCursor = page.cursor
            feedbackLoaded = true; feedbackError = nil
        } catch is CancellationError {} catch {
            guard generation == attempt, feedbackRequest == request else { return }
            if (error as? APIError)?.status == 404 {
                // No longer on the team: the inbox goes away.
                account?.feedbackTeam = nil
                feedbackReports = []; feedbackCursor = nil; feedbackLoaded = false; feedbackError = nil
                showingFeedback = false; showingArchivedFeedback = false
                return
            }
            feedbackError = FeedbackInbox.loadFailed
        }
    }

    /// "Show archived" / "Back to feedback": switches the list and reloads it.
    public func showArchivedFeedback(_ archived: Bool) async {
        guard archived != showingArchivedFeedback else { return }
        showingArchivedFeedback = archived
        feedbackRequest += 1
        feedbackReports = []; feedbackCursor = nil; feedbackLoaded = false; feedbackError = nil; feedbackLoading = false
        await refreshFeedback()
    }

    /// Archives (or restores) a report's whole thread for the team. It leaves
    /// the list at once; only a failure reloads it and shows the error.
    public func setFeedbackArchived(_ report: FeedbackReport, archived: Bool) async {
        guard account != nil, isFeedbackTeam else { return }
        let attempt = generation
        feedbackRequest += 1
        feedbackLoading = false
        feedbackReports.removeAll { $0.sharesThread(with: report) }
        feedbackError = nil
        do {
            try await api.setFeedbackArchived(channelID: report.channel.id, threadRootID: report.threadRootId, archived: archived)
        } catch is CancellationError {} catch {
            guard generation == attempt else { return }
            await refreshFeedback()
            guard generation == attempt else { return }
            feedbackError = archived ? FeedbackInbox.archiveFailed : FeedbackInbox.restoreFailed
        }
    }

    /// Opens an inbox row's conversation.
    public func openFeedback(_ report: FeedbackReport) async {
        await openFeedback(report.target)
    }

    /// Opens a reported conversation around the reported message, opening a
    /// thread only for a reported reply. A conversation you
    /// belong to (no `visitor` flag) that your lists hold opens the normal
    /// way; anything else opens as a visit, without joining.
    public func openFeedback(_ target: FeedbackTarget) async {
        guard account != nil else { return }
        directHistoryTask?.cancel(); directHistoryTask = nil; openingDirectID = nil
        rememberCurrentNavigation()
        let attempt = generation
        navigationGeneration += 1
        let navigation = navigationGeneration
        navigationTarget = nil; navigationError = nil
        openingSpaceID = nil; openingChannelID = nil
        do {
            // The thread root is in the timeline; a reply is not.
            let history = try await api.history(channelID: target.channelID, around: target.rootID)
            guard generation == attempt, navigationGeneration == navigation else { return }
            guard history.channel?.id == target.channelID else {
                throw APIError(status: 502, message: "Caper returned another conversation.")
            }
            let direct = history.channel?.direct == true || history.space?.id == ""
            if !history.visitor {
                if direct, let conversation = directMessages.first(where: { $0.id == target.channelID }) {
                    await select(directMessage: conversation, contextHistory: history)
                    guard generation == attempt, selectedDirectMessageID == target.channelID, chat.channelID == target.channelID else { return }
                    await focusFeedback(target)
                    return
                }
                if !direct, let spaceID = history.space?.id, let space = spaces.first(where: { $0.id == spaceID }) {
                    await navigate(space: space, channelID: target.channelID, contextHistory: history)
                    guard generation == attempt, selectedChannelID == target.channelID, chat.channelID == target.channelID else { return }
                    await focusFeedback(target)
                    return
                }
            }
            // Presence stays with the space shown in Browse; a visit has no
            // member list or read markers. Its inbox row holds voice.
            if visit == nil {
                visitReturn = (selectedSpaceID, selectedChannelID, selectedDirectMessageID)
            }
            visit = FeedbackVisit(channelID: target.channelID, spaceID: direct ? "" : history.space?.id ?? "",
                                  spaceName: history.space?.name ?? "", channelName: history.channel?.name ?? "",
                                  direct: direct, reportID: target.messageID, reporter: target.reporter)
            selectedDirectMessageID = nil; selectedChannelID = nil
            navigationOpen = false
            // Back (phones) returns to the inbox.
            if isFeedbackTeam { showingFeedback = true }
            Task { [weak self] in await self?.refreshPeople() }
            chat.directPeerID = nil
            await chat.open(history: history, displayName: account?.displayName ?? "")
            guard generation == attempt, navigationGeneration == navigation, visit?.channelID == target.channelID,
                  chat.channelID == target.channelID else { return }
            await focusFeedback(target)
        } catch is CancellationError {} catch {
            guard generation == attempt, navigationGeneration == navigation else { return }
            navigationError = FriendlyError.message(for: error)
        }
    }

    /// Root reports stay in the loaded timeline; replies open their existing thread.
    private func focusFeedback(_ target: FeedbackTarget) async {
        if target.isReply { await chat.openThread(target.rootID, around: target.messageID) }
        else { chat.closeThread() }
        guard chat.channelID == target.channelID else { return }
        chat.focusedMessageID = target.messageID
        chat.focusRevision += 1
    }

    /// Closes the visit on wider layouts, back to the conversation shown before it.
    public func closeVisit() async {
        guard visit != nil else { return }
        let back = visitReturn
        if let id = back?.directMessageID, let conversation = directMessages.first(where: { $0.id == id }) {
            await select(directMessage: conversation)
            if selectedDirectMessageID == id { return }
        } else if let spaceID = back?.spaceID ?? selectedSpaceID, let space = spaces.first(where: { $0.id == spaceID }) {
            await navigate(space: space, channelID: back?.channelID)
            if visit == nil { return }
        }
        // Nothing to return to (or it failed to open): an empty stage.
        guard visit != nil else { return }
        navigationGeneration += 1
        visit = nil; visitReturn = nil
        await chat.stop()
    }

    /// A visited space channel, for its voice slot. DMs have no voice.
    public var visitChannel: Channel? {
        guard let visit, !visit.direct, !visit.spaceID.isEmpty else { return nil }
        return Channel(id: visit.channelID, spaceId: visit.spaceID, name: visit.channelName, private: false, joined: true)
    }

    /// Who is in voice in a channel: the open space's rosters, or the visit's.
    public func voiceRoster(for channelID: String) -> [VoiceSpectator] {
        if let channel = visitChannel, channel.id == channelID, isVisitVoice(channel) {
            return visitVoicePresence.roster(for: channelID)
        }
        return voicePresence.roster(for: channelID)
    }

    /// The visited space channel, when the open space doesn't list it itself.
    func isVisitVoice(_ channel: Channel) -> Bool {
        guard let visitChannel, visitChannel.id == channel.id else { return false }
        return detail?.channels.contains(where: { $0.id == channel.id }) != true
    }

    /// Watches the visited space channel's voice roster while the visit is open.
    private func watchVisitVoice() {
        let channel = visitChannel
        let presence = visitVoicePresence
        Task {
            if let channel { await presence.watch(spaceID: channel.spaceId, channels: [channel], demo: false) }
            else { await presence.stop() }
        }
    }

    /// Joins voice in the visited space channel, like any channel: access is
    /// rechecked at click time and a denied switch keeps the current call.
    private func joinVisitVoice(_ visit: FeedbackVisit) async {
        guard pendingVoiceChannelID == nil, voice.phase != .joining, voice.phase != .reconnecting, voice.phase != .leaving,
              voice.context?.channelID != visit.channelID || voice.phase == .idle || voice.phase == .failed else { return }
        voiceJoinGeneration += 1
        let joinAttempt = voiceJoinGeneration
        let clicked = Int64(Date().timeIntervalSince1970 * 1_000)
        pendingVoiceStartedAt = clicked
        pendingVoiceChannelID = visit.channelID
        defer { if voiceJoinGeneration == joinAttempt { pendingVoiceChannelID = nil } }
        let accountGeneration = generation
        do {
            let history = try await api.history(channelID: visit.channelID)
            guard voiceJoinGeneration == joinAttempt, generation == accountGeneration else { return }
            guard history.channel?.id == visit.channelID, self.visit?.channelID == visit.channelID else {
                visitVoicePresence.revoke(channelID: visit.channelID)
                return
            }
            if voice.phase != .idle && voice.phase != .failed { voice.leaveImmediately() }
            await voice.join(channelID: visit.channelID,
                             context: VoiceContext(channelID: visit.channelID, channelName: visit.channelName,
                                                   spaceID: visit.spaceID, spaceName: visit.spaceName),
                             name: account?.displayName ?? "Guest", joinStartedAt: clicked,
                             sessionStartedAt: visitVoicePresence.sessionStartedAt(for: visit.channelID))
        } catch {
            guard voiceJoinGeneration == joinAttempt, generation == accountGeneration else { return }
            visitVoicePresence.revoke(channelID: visit.channelID)
            navigationError = FriendlyError.message(for: error)
        }
    }

    private func clearFeedbackState() {
        feedbackRequest += 1
        feedbackReports = []; feedbackCursor = nil; feedbackLoading = false; feedbackLoaded = false; feedbackError = nil
        showingFeedback = false; showingArchivedFeedback = false
        visit = nil; visitReturn = nil
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
    private var messageSnapshots: [ChatMessage] = []
    /// Authors (and their nested copies) show other people's live profile
    /// updates and your own saved profile.
    public var messages: [ChatMessage] {
        get { profiled(messageSnapshots) }
        set { messageSnapshots = newValue }
    }
    private var pinnedMessageSnapshots: [ChatMessage] = []
    public var pinnedMessages: [ChatMessage] {
        get { profiled(pinnedMessageSnapshots) }
        set { pinnedMessageSnapshots = newValue }
    }
    private func profiled(_ messages: [ChatMessage]) -> [ChatMessage] {
        let profiles = profileDirectory(), own = profileAuthor
        return messages.map { profiles.apply($0).withProfile(own) }
    }
    private var mutations = MessageMutations()
    private func project(_ message: ChatMessage) -> ChatMessage { mutations.project(message).withProfile(profileAuthor) }
    public var displayedMessages: [ChatMessage] { messages.map(project) }
    public var displayedPins: [ChatMessage] { profiled(mutations.pinned(messages: messages, confirmed: pinnedMessages)) }
    public var channelName = "general"
    public var spaceName = "Caper"
    public var draft = "" { didSet { if oldValue != draft { composerRevision += 1 } } }
    private var composerRevision = 0
    private var uploadSubmission: UUID?
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
    /// `GET /api/assets/usage` succeeded; otherwise the attach control stays disabled.
    public private(set) var uploadsEnabled = false
    /// A usage check has answered, so a disabled control is unavailable rather than still checking.
    public private(set) var uploadsChecked = false
    /// Client compression settings served with usage.
    public private(set) var attachmentCompression = AttachmentCompression()
    public var attachmentDrafts: [AttachmentDraft] { attachmentFiles() }
    public var threadAttachmentDrafts: [AttachmentDraft] { attachmentFiles(inThread: true) }
    public private(set) var sendUploadFailed = false
    private var preparedUploads: [String: PreparedAttachment] = [:]
    private var pendingDrafts: [AttachmentDraft] = []
    private var pendingNeedsUpload = false
    /// Latest ephemeral `attachment.progress` percent per processing attachment id.
    public private(set) var attachmentProgress: [String: Int] = [:]
    /// Composer-level attachment message, such as the 10-file limit.
    public var attachmentNotice: String? {
        get { composerTarget().flatMap { attachmentNotices[$0] } }
        set { if let target = composerTarget() { attachmentNotices[target] = newValue } }
    }
    public var threadAttachmentNotice: String? {
        get { composerTarget(inThread: true).flatMap { attachmentNotices[$0] } }
        set { if let target = composerTarget(inThread: true) { attachmentNotices[target] = newValue } }
    }
    public var canAttach: Bool { uploadsEnabled && !isPreview && session != nil && channelID != nil && !loadFailed }
    /// Every client's wording for a disabled attach control: checking while
    /// usage or the conversation loads, otherwise unavailable.
    public var attachUnavailableReason: String {
        (uploadsChecked && !uploadsEnabled) || isPreview || channelID == nil || loadFailed ? "File uploads are unavailable" : "Checking file uploads…"
    }
    /// Failed files may be retried by Send; initial uploads must settle first.
    public var canSubmit: Bool { canSubmit(inThread: false) }
    public func canSubmit(inThread: Bool) -> Bool {
        if sending || uploadSubmission != nil || delivery.rejected { return false }
        let root = inThread ? threadRootID : nil
        if inThread && root == nil { return false }
        if let pending = delivery.pending { return pending.threadRootId == root }
        let files = attachmentFiles(inThread: inThread)
        return files.allSatisfy { $0.attachment != nil || $0.error != nil }
            && MessageValidation.error(for: inThread ? threadDraft : draft, attachmentCount: files.count) == nil
    }
    public var pinErrors: [String: String] = [:]
    public var pendingPins: Set<String> = []
    public var threadRootID: String? {
        didSet { if oldValue != threadRootID { visibleThreadReply = nil; composerRevision += 1; uploadSubmission = nil } }
    }
    public private(set) var unreadThreads: [String: String] = [:]
    private var unreadThreadChannel: String?
    private var unreadThreadRevision = 0
    private var threadReadInFlight: Set<String> = []
    public var visibleThreadReply: (rootID: String, seq: String)?

    public func threadUnread(_ rootID: String) -> Bool {
        unreadThreadChannel == channelID && unreadThreads[rootID] != nil
    }

    public func refreshUnreadThreads() async {
        guard let channelID else { return }
        let attempt = generation
        unreadThreadRevision += 1
        let revision = unreadThreadRevision
        guard let rows = try? await api.unreadThreads(channelID: channelID),
              generation == attempt, self.channelID == channelID, unreadThreadRevision == revision else { return }
        unreadThreadChannel = channelID
        unreadThreads = rows.reduce(into: [:]) { $0[$1.rootId] = $1.seq }
    }

    /// Only the rendered latest window supplies this cursor; never a fetched history cursor.
    public func acknowledgeVisibleThread() {
        guard let channelID, unreadThreadChannel == channelID,
              let visible = visibleThreadReply, threadRootID == visible.rootID,
              threadMessages.contains(where: { $0.seq == visible.seq }),
              !threadHasNewer, !threadLoading, let unread = unreadThreads[visible.rootID],
              (try? Sequence.compare(visible.seq, unread)) != .orderedAscending else { return }
        let attempt = generation
        let key = "\(attempt):\(channelID):\(visible.rootID)"
        guard threadReadInFlight.insert(key).inserted else { return }
        Task {
            defer { threadReadInFlight.remove(key) }
            do {
                try await api.markThreadRead(channelID: channelID, rootID: visible.rootID, seq: visible.seq)
                guard generation == attempt, self.channelID == channelID else { return }
                unreadThreadRevision += 1 // Discard GETs started before the durable acknowledgement.
                if let current = unreadThreads[visible.rootID],
                   (try? Sequence.compare(current, visible.seq)) != .orderedDescending {
                    unreadThreads[visible.rootID] = nil
                }
            } catch { /* Keep the badge; the next visibility change or unread poll retries. */ }
        }
    }

    public var replyTargets: [String: MessageReply] = [:]
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
    /// A picker owns its original composer even if selection finishes after
    /// navigation. Access IDs invalidate old callbacks after sign-out/revocation.
    struct AttachmentTarget: Hashable {
        let channelID: String
        let threadRootID: String?
        let accessID: UUID
    }
    private var attachmentAccessIDs: [String: UUID] = [:]
    /// Files of every composer, open or not. Uploads carry on while it is closed.
    private var stagedFiles: [AttachmentTarget: [AttachmentDraft]] = [:]
    private var attachmentNotices: [AttachmentTarget: String] = [:]
    /// The conversation each staged file uploads to, open or not.
    private var draftChannels: [String: String] = [:]
    private var restoredDraft: String?
    private var threadBroadcasts: [String: Bool] = [:]
    public var channelMessages: [ChatMessage] { messages.filter { $0.isChannelMessage && !threadOnlyRows.contains($0.id) && Self.inWindow($0, start: windowStart, end: windowEnd) } }
    public var displayedChannelMessages: [ChatMessage] { channelMessages.map(project) }
    public var threadMessages: [ChatMessage] { messages.filter { $0.threadRootId != nil && $0.threadRootId == threadRootID && Self.inWindow($0, start: threadWindowStart, end: threadWindowEnd) } }
    public var displayedThreadMessages: [ChatMessage] { threadMessages.map(project) }
    private static func inWindow(_ message: ChatMessage, start: String?, end: String?) -> Bool {
        (start == nil || (try? Sequence.compare(message.seq, start!)) != .orderedAscending) &&
        (end == nil || (try? Sequence.compare(message.seq, end!)) != .orderedDescending)
    }
    public var threadDraft: String {
        get { threadRootID.flatMap { threadDrafts[$0] } ?? "" }
        set {
            if let threadRootID {
                if threadDrafts[threadRootID] != newValue { composerRevision += 1 }
                threadDrafts[threadRootID] = newValue
            }
        }
    }
    public var threadBroadcast: Bool {
        get { threadRootID.flatMap { threadBroadcasts[$0] } ?? false }
        set { if let threadRootID { threadBroadcasts[threadRootID] = newValue } }
    }
    private var profileAuthor: ChatAuthor? {
        guard let account = viewerProfile(), let name = account.displayName else { return session?.author }
        return ChatAuthor(id: account.id, name: name, isGuest: false, avatarId: account.avatarId,
                          avatarAsset: account.avatarAsset, avatarUrl: account.avatarUrl)
    }
    public var currentAuthor: ChatAuthor? {
        guard let author = session?.author else { return nil }
        guard let profile = profileAuthor, profile.id == author.id else { return author }
        return profile
    }
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
    @ObservationIgnored public var onOpenForward: ((ChatMessage) async -> Bool)?
    /// Accounts you blocked: their messages collapse in every timeline, and
    /// they never chime or show as typing. Kept in step by `AppModel`.
    public private(set) var blockedAuthorIDs: Set<String> = []
    /// The other person in the open DM, kept by `AppModel`. Blocking them stops
    /// reactions, pins and edits as well as sending; the server refuses them too.
    public var directPeerID: String?
    /// After the server confirms a sent message or reply.
    @ObservationIgnored public var onMessageSent: (() -> Void)?
    /// Reacting, pinning and editing need a chat session (not a preview), and
    /// never reach someone you blocked.
    public var canInteract: Bool {
        !isPreview && currentAuthor != nil && !(directPeerID.map { blockedAuthorIDs.contains($0) } ?? false)
    }
    /// The signed-in account, also while previewing without a chat session.
    @ObservationIgnored public var viewerAccountID: () -> String? = { nil }
    @ObservationIgnored var viewerProfile: () -> Account? = { nil }
    /// Other people's live profile updates (`AppModel.profileUpdates`).
    @ObservationIgnored var profileDirectory: () -> ProfileDirectory = { ProfileDirectory() }

    /// A `space.updated` for the open conversation's space.
    func renameSpace(id: String, name: String) {
        if spaceID == id { spaceName = name }
    }
    /// Blocks from message actions in panels that only hold the chat model.
    @ObservationIgnored public var blockAccount: ((BlockTarget) async throws -> Void)?
    /// Unblocks from message actions, by account id.
    @ObservationIgnored public var unblockAccount: ((String) async throws -> Void)?
    /// A DM send refused with `dm_blocked`: the block list needs a refresh.
    @ObservationIgnored public var onDirectMessageBlocked: (() -> Void)?
    public var viewerID: String? { session?.author.id ?? viewerAccountID() }
    private let api: APIClient
    /// Read by AppModel to follow a forwarded message to its conversation.
    private(set) var channelID: String?
    private var spaceID: String?
    private var session: ChatSession?
    private var subscriptionID: String?
    private var generation = 0
    private var delivery = ChatDeliveryState()
    private var reactionSnapshots = ReactionSnapshots()
    private var attachmentSnapshots = AttachmentSnapshots()
    /// Staged originals this device sent, shown while the server processes
    /// them; removed once the attachment is ready or failed.
    private var localAttachmentCopies: [String: URL] = [:]
    private var pinSnapshots = PinSnapshots()
    private var forwardSnapshots = ForwardSnapshots()
    private var editSnapshots = EditSnapshots()
    private var deletingMessages: Set<String> = []
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
    private var uploadTasks: [String: Task<Void, Never>] = [:]
    private var uploadGeneration = 0
    private var urlRefreshQueue: Set<String> = []
    /// The URL each attachment held when its refresh was requested: a stale
    /// URL is refreshed at most once, but a later expiry can refresh again.
    private var urlRefreshAttempts: [String: String] = [:]
    private var urlRefreshTask: Task<Void, Never>?
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
        message.forward == nil && canDelete(message)
    }

    /// Your own messages, forwards included, while you can participate.
    func canDelete(_ message: ChatMessage) -> Bool {
        !message.isDeleted && canInteract && channelID == message.channelId && currentAuthor?.isGuest == false && currentAuthor?.id == message.author.id
    }

    /// Whether the loaded copy of a message is now a tombstone.
    func isDeleted(_ messageID: String) -> Bool {
        (messageSnapshots + pinnedMessageSnapshots).contains { $0.id == messageID && $0.isDeleted }
    }

    /// Merges the returned tombstone like a saved edit; a failure shows as the conversation error.
    func deleteMessage(_ message: ChatMessage) async {
        guard canDelete(message), let session, !deletingMessages.contains(message.id) else { return }
        let request = generation
        deletingMessages.insert(message.id)
        defer { if request == generation { deletingMessages.remove(message.id) } }
        do {
            let result = try await api.deleteMessage(channelID: message.channelId, messageID: message.id, sessionToken: session.token)
            guard request == generation, channelID == message.channelId else { return }
            guard result.author.id == session.author.id else { error = "Message author mismatch."; return }
            if !applyEditSnapshot(result) { requestResync(generation: request, channelID: message.channelId) }
        } catch {
            guard request == generation, channelID == message.channelId, !(error is CancellationError) else { return }
            self.error = MessageDeletion.failureMessage(for: error)
        }
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
        self.session = ChatSession(token: session.token, author: ChatAuthor(id: account.id, name: name, isGuest: false, avatarId: account.avatarId,
                                                                            avatarAsset: account.avatarAsset, avatarUrl: account.avatarUrl))
    }

    /// A token-free copy of only the timeline currently retained by this model.
    public func currentSnapshot() -> ChatHistory? {
        guard session != nil || isPreview, let channelID, let spaceID else { return nil }
        return ChatHistory(
            space: HistoryIdentity(id: spaceID, name: spaceName),
            channel: HistoryIdentity(id: channelID, name: channelName, direct: spaceID.isEmpty ? true : nil),
            messages: channelMessages.map { attachmentSnapshots.overlay(forwardSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0)))) },
            pinnedMessages: pinnedMessages.map { attachmentSnapshots.overlay(forwardSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay($0)))) },
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
        channelID = history.channel?.id; spaceID = history.space?.id
        seedSnapshots(history.messages + history.pinnedMessages)
        pinSnapshots.replace(history.messages + history.pinnedMessages, cursor: history.cursor)
        forwardSnapshots.seed(history.messages + history.pinnedMessages)
        editSnapshots.seed(history.messages + history.pinnedMessages)
        messages = history.messages.map { overlaySnapshots($0) }
        pinnedMessages = history.pinnedMessages.map { overlayPinned($0) }
        delivery.reset(cursor: history.cursor); hasMore = history.hasMore
        hasNewer = history.hasNewer
        windowStart = history.hasNewer ? history.messages.first?.seq : nil
        windowEnd = history.hasNewer ? history.messages.last?.seq : nil
        channelName = history.channel?.name ?? "general"; spaceName = history.space?.name ?? "Caper"
        session = nil; loading = false; loadFailed = false; sessionError = nil
        if let oldSubscription { await gateway.unsubscribe(oldSubscription) }
        let signedIn = await api.isSignedIn
        guard generation == requestGeneration else { return }
        canForward = signedIn
        guard let actualChannel = channelID else { return }
        let subscription = await gateway.subscribeChat(channelID: actualChannel, after: delivery.cursor) { [weak self] event in
            self?.receive(event, generation: requestGeneration, channelID: actualChannel)
        }
        guard generation == requestGeneration, channelID == actualChannel else {
            await gateway.unsubscribe(subscription)
            return
        }
        subscriptionID = subscription
        await refreshUnreadThreads()
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
        unreadThreads = [:]; unreadThreadChannel = nil; visibleThreadReply = nil
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
        if let channelID { restoreStagedFiles(channelID: channelID) }
        draft = preservingPending ? preservedDraft : restoredDraft
        self.restoredDraft = draft.isEmpty ? nil : draft
        // Prepared history is already loaded and authorized by navigation.
        // Show it immediately; obtaining a sending capability is not a new
        // history load (and must not flash the previous channel's title).
        if let prepared {
            self.channelID = prepared.channel?.id; spaceID = prepared.space?.id
            seedSnapshots(prepared.messages + prepared.pinnedMessages)
            pinSnapshots.replace(prepared.messages + prepared.pinnedMessages, cursor: prepared.cursor)
            forwardSnapshots.seed(prepared.messages + prepared.pinnedMessages)
            editSnapshots.seed(messages + pinnedMessages + prepared.messages + prepared.pinnedMessages)
            messages = prepared.messages.map { overlaySnapshots($0) }
            pinnedMessages = prepared.pinnedMessages.map { overlayPinned($0) }
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
            var sessionFailure: String?
            do { chatSession = try await sessionRequest } catch {
                if let apiError = error as? APIError, [401, 403, 404].contains(apiError.status) { throw error }
                chatSession = nil
                sessionFailure = error.localizedDescription
            }
            let signedIn = await api.isSignedIn
            // A late failure from a channel opened earlier never reaches the current one.
            guard self.channelID == channelID, generation == requestGeneration else { return }
            sessionError = sessionFailure
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
                attachmentSnapshots.seed(history.pinnedMessages)
                let candidates = Dictionary((pinnedMessages + history.pinnedMessages).map { ($0.id, $0) }, uniquingKeysWith: { _, next in next })
                pinnedMessages = candidates.values.map { overlayPinned($0) }.filter { $0.pin != nil }
                    .sorted { (try? Sequence.compare($0.pinSeq ?? "0", $1.pinSeq ?? "0")) == .orderedDescending }
                messages = channelMessages
                threadOnlyRows = []
                if canRetain {
                    // Include HTTP confirmations received while refresh was pending.
                    merge(history.messages) // The refreshed representation wins overlapping IDs.
                } else {
                    // Preserve a newer reaction revision on overlapping rows, but
                    // discard snapshots for rows no longer in the fresh window.
                    seedSnapshots(history.messages)
                    messages = history.messages.map { overlaySnapshots($0) }
                    reactionSnapshots.reset(); attachmentSnapshots.reset()
                    seedSnapshots(messages + pinnedMessages)
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
                discardStagedFiles(channelID: channelID)
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
                discardStagedFiles(channelID: channelID)
                onAccessRevoked?(channelID)
                await stop()
                guard generation == requestGeneration + 1 else { return }
                self.error = error.localizedDescription
            } else { olderError = error.localizedDescription }
        }
    }

    public func closeThread() { threadRequest += 1; threadRootID = nil; threadLoading = false }

    public func goToMessage(_ message: ChatMessage) async -> Bool {
        await goToMessage(id: message.id, channelID: message.channelId, threadRootID: message.threadRootId)
    }

    public func goToReply(_ reference: MessageReply) async -> Bool {
        guard let channelID else { return false }
        return await goToMessage(id: reference.id, channelID: channelID, threadRootID: reference.threadRootId)
    }

    private func goToMessage(id: String, channelID: String, threadRootID: String?) async -> Bool {
        guard !loading, !jumpingToMessage, channelID == self.channelID else { return false }
        let requestGeneration = generation
        historyAnchorRequest += 1
        let request = historyAnchorRequest
        jumpingToMessage = true; jumpError = nil
        loadingNewer = false; loadingOlder = false
        defer { if generation == requestGeneration && request == historyAnchorRequest { jumpingToMessage = false } }
        do {
            if let root = threadRootID {
                await openThread(root, around: id)
                guard generation == requestGeneration, request == historyAnchorRequest else { return false }
                if let error = threadError { throw APIError(status: 502, message: error) }
                guard threadMessages.contains(where: { $0.id == id }) else { return false }
            } else {
                let page = try await api.history(channelID: channelID, around: id)
                guard generation == requestGeneration, request == historyAnchorRequest else { return false }
                guard page.messages.contains(where: { $0.id == id }) else { throw APIError(status: 502, message: "Message context is unavailable.") }
                windowStart = page.messages.first?.seq; windowEnd = page.hasNewer ? page.messages.last?.seq : nil
                threadOnlyRows.subtract(page.messages.map(\.id)); merge(page.messages)
                hasMore = page.hasMore; hasNewer = page.hasNewer; closeThread()
            }
            focusedMessageID = id; focusRevision += 1
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
        for file in threadAttachmentDrafts where file.needsUpload { retryAttachmentDraft(id: file.id) }
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

    public func reply(to message: ChatMessage, inThread: Bool = false) {
        guard canInteract, delivery.pending == nil, message.channelId == channelID, !message.isDeleted else { return }
        guard !inThread || threadRootID != nil else { return }
        let root = inThread ? threadRootID : (message.isChannelMessage ? nil : message.threadRootId)
        replyTargets[root ?? "channel"] = MessageReply(message: message)
        if !inThread, let root { Task { await openThread(root) } }
    }

    public func send(inThread: Bool = false, retryPending: Bool = false) async {
        guard !sending, uploadSubmission == nil, !delivery.rejected else { return }
        guard !retryPending || delivery.pending != nil else { return }
        guard let channelID, let session else { error = "Messaging session is unavailable."; return }
        let rootID = inThread ? (retryPending ? delivery.pending?.threadRootId : threadRootID) : nil
        guard !inThread || rootID != nil else { return }
        if let pending = delivery.pending, pending.threadRootId != rootID { return }
        let text = inThread ? threadDraft : draft
        let target = composerTarget(inThread: inThread)
        var files = attachmentFiles(inThread: inThread)
        if delivery.pending == nil {
            if let validation = MessageValidation.error(for: text, attachmentCount: files.count) { error = validation; return }
            if files.contains(where: { $0.attachment == nil }) {
                let submission = UUID()
                let expectedGeneration = generation
                let expectedUploads = uploadGeneration
                let revision = composerRevision
                let broadcast = threadBroadcast
                let reply = replyTargets[rootID ?? "channel"]?.id
                let ids = files.map(\.id)
                uploadSubmission = submission
                defer { if uploadSubmission == submission { uploadSubmission = nil } }
                for file in files where file.error != nil { retryAttachmentDraft(id: file.id) }
                let tasks = files.compactMap { uploadTasks[$0.id] }
                for task in tasks { await task.value }
                guard !Task.isCancelled, uploadSubmission == submission, generation == expectedGeneration, uploadGeneration == expectedUploads,
                      self.channelID == channelID, self.session?.token == session.token,
                      composerTarget(inThread: inThread) == target, composerRevision == revision,
                      !inThread || (threadRootID == rootID && threadBroadcast == broadcast),
                      replyTargets[rootID ?? "channel"]?.id == reply,
                      delivery.pending == nil else { return }
                files = attachmentFiles(inThread: inThread)
                guard files.map(\.id) == ids, files.allSatisfy({ $0.attachment != nil }) else { return }
            }
        }
        let newSubmission = delivery.pending == nil
        // The pending row previews the local originals (which this device can
        // decode even when browsers cannot, e.g. HEIC) until the message is confirmed.
        let localAttachments: [ChatAttachment] = newSubmission ? files.compactMap { item in
            guard var attachment = item.attachment else { return nil }
            attachment.kind = item.kind
            attachment.status = .ready
            attachment.animated = false
            attachment.url = item.localURL.absoluteString
            attachment.previewUrl = nil
            attachment.spoiler = item.spoiler
            return attachment
        } : []
        var command = delivery.begin(text: text, attachments: localAttachments, threadRootId: rootID, broadcast: inThread && threadBroadcast, replyTo: replyTargets[rootID ?? "channel"])
        if newSubmission {
            replyTargets[rootID ?? "channel"] = nil
            pendingDrafts = files
            pendingNeedsUpload = files.contains { $0.needsUpload }
            // Pending and processing rows retain originals from either composer.
            for item in files { if let id = item.attachment?.id { localAttachmentCopies[id] = item.localURL } }
            if let target { stagedFiles[target] = nil; attachmentNotices[target] = nil }
            // "Also send to channel" applies to one reply, so it resets with the draft.
            if inThread { threadDraft = ""; threadBroadcast = false } else {
                draft = ""
            }
        }
        let requestGeneration = generation
        sending = true; error = nil; sendUploadFailed = false
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
            var recovered = false
            let message: ChatMessage
            while true {
                if pendingNeedsUpload {
                    guard !pendingDrafts.isEmpty else { throw APIError(status: 502, message: "Failed to upload.") }
                    var attachments: [ChatAttachment] = []
                    for file in pendingDrafts {
                        var attachment = try await uploadAttachmentDraft(id: file.id)
                        attachment.kind = file.kind; attachment.status = .ready; attachment.animated = false
                        attachment.url = file.localURL.absoluteString; attachment.previewUrl = nil
                        attachment.spoiler = file.spoiler
                        attachments.append(attachment)
                    }
                    guard self.channelID == channelID,
                          let refreshed = delivery.replaceAttachments(id: command.id, attachments: attachments) else { return }
                    for file in pendingDrafts {
                        if let id = file.attachment?.id { localAttachmentCopies[id] = file.localURL }
                    }
                    command = refreshed; pendingNeedsUpload = false
                }
                do {
                    message = try await api.send(channelID: channelID, sessionToken: session.token, clientMessageID: command.id, text: command.text,
                                                 attachmentIDs: command.attachmentIDs, spoilerAttachmentIDs: command.attachments.filter(\.spoiler).map(\.id), threadRootId: command.threadRootId, broadcast: command.broadcast, replyToId: command.replyTo?.id)
                    break
                } catch let refusal as APIError where refusal.status == 409 && refusal.code == "attachment_unavailable" {
                    guard self.channelID == channelID, delivery.pending?.id == command.id else { return }
                    pendingNeedsUpload = true
                    for index in pendingDrafts.indices { pendingDrafts[index].uploadedAt = .distantPast }
                    if recovered { throw APIError(status: 502, message: "Failed to upload.") }
                    recovered = true
                }
            }
            guard self.channelID == channelID,
                  generation == requestGeneration || delivery.pending?.id == command.id else { return }
            guard MessageValidation.acceptsResponse(message, channelID: channelID, command: command, authorID: session.author.id) else {
                throw APIError(status: 502, message: "The chat service returned an invalid message.")
            }
            merge([message]); delivery.confirmHTTP(id: command.id)
            releasePendingUploads()
            onMessageSent?()
        } catch {
            guard self.channelID == channelID,
                  generation == requestGeneration || delivery.pending?.id == command.id else { return }
            if delivery.pending?.id != command.id {
                self.error = nil
                return
            }
            if pendingNeedsUpload {
                sendUploadFailed = true; self.error = "Failed to upload."
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
            if let people = list.people(for: reaction, viewer: reactorViewer(viewerID)) {
                let account = viewerProfile(), profiles = profileDirectory()
                return .loaded(people.map { person in
                    guard let account, person.id == account.id else { return profiles.apply(person) }
                    let picture = account.ownPicture(asset: person.avatarAsset, url: person.avatarUrl)
                    return ReactorPerson(id: person.id, username: account.username, displayName: account.displayName, avatarId: person.avatarId,
                                         avatarAsset: picture.asset, avatarUrl: picture.url)
                })
            }
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
        guard let author = profileAuthor, author.id == id else { return ReactorPerson(id: id) }
        return ReactorPerson(id: id, displayName: author.name, avatarId: author.avatarId, avatarAsset: author.avatarAsset, avatarUrl: author.avatarUrl)
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
        let root = delivery.pending?.threadRootId
        let target = channelID.flatMap { channel in attachmentAccessIDs[channel].map { AttachmentTarget(channelID: channel, threadRootID: root, accessID: $0) } }
        let textEmpty = root.map { (threadDrafts[$0] ?? "").isEmpty } ?? draft.isEmpty
        guard !sending, delivery.rejected,
              !edit || (textEmpty && (target.flatMap { stagedFiles[$0] } ?? []).isEmpty) else { return false }
        let reference = delivery.pending?.replyTo
        let broadcast = delivery.pending?.broadcast ?? false
        let attachments = delivery.pending?.attachmentIDs ?? []
        guard let text = delivery.discardRejected() else { return false }
        // Only definitively rejected sends may be discarded.
        for id in attachments {
            localAttachmentCopies[id] = nil
        }
        if edit {
            if let target { stagedFiles[target] = pendingDrafts }
            if let root { threadDrafts[root] = text; threadBroadcasts[root] = broadcast } else { draft = text }
            replyTargets[root ?? "channel"] = reference
        } else {
            pendingDrafts.forEach(abandon)
        }
        releasePendingUploads(keepingDrafts: edit)
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
        if discardingDrafts {
            drafts = [:]; threadDrafts = [:]; threadBroadcasts = [:]
            discardStagedFiles()
        }
        if let oldSubscription { await gateway.unsubscribe(oldSubscription) }
    }

    /// The generation `receive` currently accepts (tests deliver gateway events with it).
    var eventGeneration: Int { generation }

    func receive(_ event: [String: Any], generation eventGeneration: Int, channelID eventChannelID: String) {
        guard generation == eventGeneration, channelID == eventChannelID else { return }
        guard let type = event["type"] as? String else { return }
        if type == "subscription.error", let status = event["status"] as? Int, [401, 403, 404].contains(status) {
            clearLocal(); error = event["error"] as? String ?? "Channel access ended."
            discardStagedFiles(channelID: eventChannelID)
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
        if type == "message.attachments" {
            guard let data = try? JSONSerialization.data(withJSONObject: event),
                  let attachmentsEvent = try? JSONDecoder().decode(MessageAttachmentsEvent.self, from: data),
                  attachmentsEvent.isValid, attachmentsEvent.channelId == eventChannelID,
                  delivery.receive(seq: attachmentsEvent.seq) else {
                requestResync(generation: eventGeneration, channelID: eventChannelID)
                return
            }
            applyAttachments(attachmentsEvent)
            onReadCursor?()
            if attachmentSnapshots.unseenOverflowed {
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
        if type == "attachment.progress" {
            // Ephemeral and unsequenced, like typing: never a resync.
            if event["channelId"] as? String == eventChannelID { receiveAttachmentProgress(event) }
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
                   message.replyTo?.id == delivery.pending?.replyTo?.id,
                   (message.broadcast ?? false) == (delivery.pending?.broadcast ?? false),
                   delivery.confirmGateway(clientMessageID: message.clientMessageId, authorID: message.author.id, ownAuthorID: session?.author.id) {
                    error = nil
                    releasePendingUploads()
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

    /// Returns a reopened conversation's staged files, renewing any that
    /// failed or expired while it was closed.
    private func restoreStagedFiles(channelID: String) {
        let files = stagedFiles.filter { $0.key.channelID == channelID }.values.flatMap { $0 }
        for file in files where file.needsUpload && preparedUploads[file.id] != nil { retryAttachmentDraft(id: file.id) }
    }

    private func hasDraft(_ id: String) -> Bool {
        pendingDrafts.contains(where: { $0.id == id })
            || stagedFiles.values.contains(where: { $0.contains(where: { $0.id == id }) })
    }

    /// Gives up staged files that can no longer be sent: on sign-out, or for a
    /// conversation whose access ended.
    private func discardStagedFiles(channelID: String? = nil) {
        let targets = stagedFiles.keys.filter { channelID == nil || $0.channelID == channelID }
        for target in targets {
            for draft in stagedFiles.removeValue(forKey: target) ?? [] { abandon(draft) }
        }
        attachmentNotices = attachmentNotices.filter { channelID != nil && $0.key.channelID != channelID }
        if let channelID { attachmentAccessIDs[channelID] = nil } else { attachmentAccessIDs = [:]; uploadGeneration += 1 }
    }

    private func abandon(_ draft: AttachmentDraft) {
        uploadTasks[draft.id]?.cancel()
        uploadTasks[draft.id] = nil
        preparedUploads[draft.id] = nil
        draftChannels[draft.id] = nil
        if let asset = draft.attachment?.id { Task { await api.cancelAsset(id: asset) } }
        AttachmentStaging.remove(draft.localURL)
        if let original = draft.original { AttachmentStaging.remove(original.url) }
        if let preview = draft.editPreviewURL { AttachmentStaging.remove(preview) }
    }

    private func clearLocal(preservingPending: Bool = false) {
        uploadSubmission = nil
        unreadThreads = [:]; unreadThreadChannel = nil; visibleThreadReply = nil
        stashDraft()
        typingTask?.cancel(); typingIdleTask?.cancel(); typingExpiryTask?.cancel()
        typingTask = nil; typingIdleTask = nil; typingExpiryTask = nil
        typers = [:]; typingNames = []; typingActive = false; typingSent = false
        delivery.reset(preservingPending: preservingPending)
        reactionSnapshots.reset(); attachmentSnapshots.reset(); pendingReactions = [:]; reactionWorkers = []; reactionErrors = [:]; failedReactions = [:]
        if !preservingPending {
            pendingDrafts.forEach { AttachmentStaging.remove($0.localURL) }
            releasePendingUploads()
            localAttachmentCopies.values.forEach { AttachmentStaging.remove($0) }
            localAttachmentCopies = [:]
        }
        attachmentProgress = [:]
        urlRefreshTask?.cancel(); urlRefreshTask = nil; urlRefreshQueue = []; urlRefreshAttempts = [:]
        reactorCache = [:]; reactorRequests = [:]; reactorFailures = []
        pinSnapshots.reset(); pinnedMessages = []; pendingPins = []; pinErrors = [:]
        forwardSnapshots.reset()
        mutations = MessageMutations()
        canForward = false; forwardTarget = nil; forwardConversationTarget = nil
        editSnapshots.reset()
        failedPinActions = [:]
        closeThread(); threadPages = [:]; threadOnlyRows = []
        replyTargets = [:]
        isPreview = false
        session = nil; channelID = nil; spaceID = nil; messages = []; draft = ""; hasMore = false
        hasNewer = false; loadingNewer = false; windowStart = nil; windowEnd = nil
        focusedMessageID = nil; jumpingToMessage = false; jumpError = nil; historyAnchorRequest += 1
        channelName = "general"; spaceName = "Caper"; error = nil
        loadingOlder = false; olderError = nil
        loading = false; sending = false; liveState = .disconnected
    }

    private func merge(_ incoming: [ChatMessage]) {
        seedSnapshots(incoming)
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
        incoming.forEach { byID[$0.id] = overlaySnapshots($0) }
        byID = byID.mapValues { message in
            var updated = overlaySnapshots(message)
            updated.thread = summaries[message.threadRootId ?? message.id] ?? message.thread
            return updated
        }
        messages = byID.values.sorted { (try? Sequence.compare($0.seq, $1.seq)) == .orderedAscending }
        renderReactions()
    }

    private func applyPin(_ message: ChatMessage) {
        editSnapshots.seed(messages + pinnedMessages + [message])
        seedSnapshots([message])
        guard pinSnapshots.apply(message) else { renderReactions(); return }
        messages = messages.map { attachmentSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay($0))) }
        pinnedMessages.removeAll { $0.id == message.id }
        let updated = overlayPinned(message)
        if updated.pin != nil {
            pinnedMessages.append(updated)
            pinnedMessages.sort { (try? Sequence.compare($0.pinSeq ?? "0", $1.pinSeq ?? "0")) == .orderedDescending }
        }
        renderReactions()
    }

    private func seedSnapshots(_ incoming: [ChatMessage]) {
        reactionSnapshots.seed(incoming)
        attachmentSnapshots.seed(incoming)
    }

    /// Every per-message snapshot over a timeline row. Attachments go last so
    /// an edit or forward overlay never restores an older attachment state.
    private func overlaySnapshots(_ message: ChatMessage) -> ChatMessage {
        attachmentSnapshots.overlay(forwardSnapshots.overlay(editSnapshots.overlay(pinSnapshots.overlay(reactionSnapshots.overlay(message)))))
    }

    /// Pinned-list rows: the same snapshots as timeline rows, so pinned
    /// copies show current reactions and files too.
    private func overlayPinned(_ message: ChatMessage) -> ChatMessage {
        overlaySnapshots(message)
    }

    /// Replaces a message's attachments when the event is newer than its
    /// `attachmentsSeq`, then drops progress and local copies for files that
    /// are no longer processing.
    private func applyAttachments(_ event: MessageAttachmentsEvent) {
        guard attachmentSnapshots.apply(messageID: event.messageId, seq: event.seq, attachments: event.attachments) else { return }
        renderReactions()
        pinnedMessages = pinnedMessages.map { attachmentSnapshots.overlay($0) }
        for attachment in event.attachments where attachment.status != .processing || attachment.unavailable {
            attachmentProgress[attachment.id] = nil
            if let local = localAttachmentCopies.removeValue(forKey: attachment.id) { AttachmentStaging.remove(local) }
        }
    }

    private func receiveAttachmentProgress(_ event: [String: Any]) {
        guard let attachmentID = event["attachmentId"] as? String, APIClient.validAssetID(attachmentID),
              let messageID = event["messageId"] as? String, !messageID.isEmpty,
              let percent = (event["percent"] as? NSNumber)?.doubleValue, percent.isFinite else { return }
        // Only files still shown as processing; a late frame after "ready" is dropped.
        guard let message = messages.first(where: { $0.id == messageID }) ?? pinnedMessages.first(where: { $0.id == messageID }),
              message.content.attachments?.contains(where: { $0.id == attachmentID && $0.status == .processing }) == true else { return }
        attachmentProgress[attachmentID] = Int(min(100, max(0, percent)).rounded())
    }

    /// The original this device uploaded, while the server still processes it.
    public func localCopy(for attachmentID: String) -> URL? { localAttachmentCopies[attachmentID] }
    private func applyReactions(_ event: MessageReactionsEvent) {
        guard reactionSnapshots.apply(messageID: event.messageId, seq: event.seq, reactions: event.reactions) else { return }
        renderReactions()
    }

    private func renderReactions() {
        let authorID = session?.author.id
        let project: (ChatMessage) -> ChatMessage = { [self] message in
            var result = attachmentSnapshots.overlay(reactionSnapshots.overlay(message))
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

    // MARK: Attachments

    func receiveProfileMessage(_ message: ChatMessage) {
        if channelID == message.channelId { merge([message]) }
    }

    /// Uploads are optional server configuration; only a successful usage
    /// response enables the attach control.
    public func checkUploadAvailability() async {
        // A cancelled check (switching conversations) is superseded, not an answer.
        defer { if !Task.isCancelled { uploadsChecked = true } }
        do {
            let usage = try await api.assetUsage()
            attachmentCompression = usage.compression
            uploadsEnabled = true
        } catch {
            uploadsEnabled = false
        }
    }

    private func composerTarget(inThread: Bool = false) -> AttachmentTarget? {
        guard let channelID, let accessID = attachmentAccessIDs[channelID], !inThread || threadRootID != nil else { return nil }
        return AttachmentTarget(channelID: channelID, threadRootID: inThread ? threadRootID : nil, accessID: accessID)
    }

    /// Capture before opening a picker, not after its asynchronous import.
    func attachmentTarget(inThread: Bool = false) -> AttachmentTarget? {
        guard canAttach, let channelID, !inThread || threadRootID != nil else { return nil }
        if attachmentAccessIDs[channelID] == nil { attachmentAccessIDs[channelID] = UUID() }
        return composerTarget(inThread: inThread)
    }

    func attachmentFiles(inThread: Bool = false) -> [AttachmentDraft] {
        composerTarget(inThread: inThread).flatMap { stagedFiles[$0] } ?? []
    }

    func setAttachmentNotice(_ notice: String?, target: AttachmentTarget) {
        guard attachmentAccessIDs[target.channelID] == target.accessID else { return }
        attachmentNotices[target] = notice
    }

    func attachmentNotice(target: AttachmentTarget) -> String? { attachmentNotices[target] }

    /// Return false for ordinary text/links so the native editor handles them.
    func pasteAttachments(inThread: Bool = false) -> Bool {
        guard let target = attachmentTarget(inThread: inThread) else { return false }
        do {
            let files = try NativeAttachmentPaste.stage()
            guard !files.isEmpty else { return false }
            addAttachments(files, to: target)
        } catch { setAttachmentNotice("Pasted files couldn’t be added. Try selecting them with Files.", target: target) }
        return true
    }

    /// Compresses (with the server's settings) and uploads staged files for
    /// the next message, up to 10.
    public func addAttachments(_ files: [LocalAttachmentFile], inThread: Bool = false) {
        guard let target = attachmentTarget(inThread: inThread) else {
            files.forEach { AttachmentStaging.remove($0.url) }
            return
        }
        addAttachments(files, to: target)
    }

    func addAttachments(_ files: [LocalAttachmentFile], to target: AttachmentTarget) {
        guard attachmentAccessIDs[target.channelID] == target.accessID, !files.isEmpty else {
            files.forEach { AttachmentStaging.remove($0.url) }
            return
        }
        let room = AttachmentPolicy.maxAttachments - (stagedFiles[target]?.count ?? 0)
        guard room > 0 else {
            setAttachmentNotice("You can attach up to \(AttachmentPolicy.maxAttachments) files.", target: target)
            files.forEach { AttachmentStaging.remove($0.url) }
            return
        }
        composerRevision += 1
        setAttachmentNotice(files.count > room ? "Only \(room) more file\(room == 1 ? "" : "s") can be attached." : nil, target: target)
        files.dropFirst(room).forEach { AttachmentStaging.remove($0.url) }
        if error == "Write a message first." || error == "Wait for files to finish uploading." { error = nil }
        let expectedGeneration = uploadGeneration
        for file in files.prefix(room) {
            let id = UUID().uuidString
            draftChannels[id] = target.channelID
            var draft = AttachmentDraft(id: id, name: file.name, kind: AttachmentKind.local(contentType: file.contentType),
                                        localURL: file.url, sourceSize: file.size)
            draft.original = file
            stagedFiles[target, default: []].append(draft)
            uploadTasks[id] = Task { [weak self] in
                // Only headers are read, so the pencil appears before compressing.
                let editable = await AttachmentPreparer.canEdit(file)
                // Removed or access revoked meanwhile (that removed the file too).
                guard let self, self.uploadGeneration == expectedGeneration, !Task.isCancelled, self.hasDraft(id) else { return }
                self.updateDraft(id) { $0.editable = editable }
                await self.prepareDraft(id: id, original: file, edit: .original, generation: expectedGeneration)
            }
        }
    }

    /// Compresses a staged file's original with `edit` applied, then uploads
    /// the result. An edit that can't be applied fails the draft (with Retry)
    /// and nothing is uploaded: the unedited file is never a fallback.
    private func prepareDraft(id: String, original file: LocalAttachmentFile, edit: AttachmentEdit, generation expected: Int) async {
        let prepared: PreparedAttachment
        do {
            prepared = try await AttachmentPreparer.prepare(file, edit: edit, settings: attachmentCompression)
        } catch {
            guard uploadGeneration == expected, !Task.isCancelled, hasDraft(id) else { return }
            updateDraft(id) { $0.preparing = false; $0.error = AttachmentEdit.failureMessage }
            uploadTasks[id] = nil
            return
        }
        guard uploadGeneration == expected, !Task.isCancelled, hasDraft(id) else {
            // Removed, edited again or access revoked while compressing.
            if prepared.fileURL != file.url { AttachmentStaging.remove(prepared.fileURL) }
            if !hasDraft(id) { AttachmentStaging.remove(file.url) }
            return
        }
        // Editable drafts keep their original for the editor until sent or removed.
        let keepsOriginal = stagedDraft(id)?.editable == true
        if prepared.fileURL != file.url, !keepsOriginal { AttachmentStaging.remove(file.url) }
        preparedUploads[id] = prepared
        updateDraft(id) {
            if !keepsOriginal { $0.original = nil }
            $0.name = prepared.name
            $0.kind = AttachmentKind.local(contentType: prepared.contentType)
            $0.localURL = prepared.fileURL
            $0.storedSize = prepared.byteSize
            $0.preparing = false
        }
        do {
            _ = try await uploadAttachmentDraft(id: id)
        } catch {
            guard uploadGeneration == expected, !Task.isCancelled else { return }
            updateDraft(id) { $0.error = "Failed to upload." }
        }
        uploadTasks[id] = nil
    }

    /// Saves the editor's change to a staged file (docs/media.md, "Editing
    /// attachments before sending"): cancels its compression or upload and
    /// the server reservation, as Remove does, then prepares the original
    /// with `edit` applied and uploads that in the same place. `preview`, a
    /// staged render of the edit, is the chip's thumbnail meanwhile.
    public func editAttachmentDraft(id: String, edit: AttachmentEdit, preview: URL?) {
        guard let draft = stagedDraft(id), draft.editable, let original = draft.original, edit != draft.edit else {
            if let preview { AttachmentStaging.remove(preview) }
            return
        }
        uploadTasks[id]?.cancel()
        uploadTasks[id] = nil
        preparedUploads[id] = nil
        if let asset = draft.attachment?.id {
            localAttachmentCopies[asset] = nil
            Task { await api.cancelAsset(id: asset) }
        }
        if draft.localURL != original.url { AttachmentStaging.remove(draft.localURL) }
        if let old = draft.editPreviewURL { AttachmentStaging.remove(old) }
        // Back to the original: the chip shows it again.
        let thumbnail = edit.isEdited ? preview : nil
        if thumbnail == nil, let preview { AttachmentStaging.remove(preview) }
        // Same place and name; the old compressed file is gone, so it shows the original until the new one is ready.
        updateDraft(id) {
            $0.edit = edit
            $0.editPreviewURL = thumbnail
            $0.localURL = original.url
            $0.storedSize = nil
            $0.preparing = true
            $0.progress = 0
            $0.error = nil
            $0.attachment = nil
            $0.uploadedAt = nil
        }
        let expected = uploadGeneration
        uploadTasks[id] = Task { [weak self] in
            guard let self else { return }
            await self.prepareDraft(id: id, original: original, edit: edit, generation: expected)
        }
    }

    private func stagedDraft(_ id: String) -> AttachmentDraft? {
        for files in stagedFiles.values {
            if let draft = files.first(where: { $0.id == id }) { return draft }
        }
        return nil
    }

    public func removeAttachmentDraft(id: String) {
        for (target, files) in stagedFiles {
            guard let draft = files.first(where: { $0.id == id }) else { continue }
            abandon(draft)
            stagedFiles[target]?.removeAll { $0.id == id }
            attachmentNotices[target] = nil
            return
        }
    }

    public func toggleAttachmentSpoiler(id: String) {
        updateDraft(id) { $0.spoiler.toggle() }
    }

    private func updateDraft(_ id: String, _ change: (inout AttachmentDraft) -> Void) {
        for (target, files) in stagedFiles {
            if let index = files.firstIndex(where: { $0.id == id }) { change(&stagedFiles[target]![index]) }
        }
        if let index = pendingDrafts.firstIndex(where: { $0.id == id }) { change(&pendingDrafts[index]) }
    }

    public func refreshDraftUploads() {
        for file in stagedFiles.values.flatMap({ $0 }) where file.needsUpload { retryAttachmentDraft(id: file.id) }
        if sendUploadFailed { Task { await send(inThread: delivery.pending?.threadRootId != nil, retryPending: true) } }
    }

    public func retryAttachmentDraft(id: String) {
        guard uploadTasks[id] == nil, let draft = stagedDraft(id) else { return }
        guard preparedUploads[id] != nil else {
            // Edits that couldn't be applied are tried again, from the original.
            guard draft.error != nil, let original = draft.original else { return }
            updateDraft(id) { $0.error = nil; $0.preparing = true; $0.progress = 0 }
            let expected = uploadGeneration
            uploadTasks[id] = Task { [weak self] in
                guard let self else { return }
                await self.prepareDraft(id: id, original: original, edit: draft.edit, generation: expected)
            }
            return
        }
        uploadTasks[id] = Task { [weak self] in
            guard let self else { return }
            do { _ = try await uploadAttachmentDraft(id: id) }
            catch { if !Task.isCancelled { updateDraft(id) { $0.error = "Failed to upload." } } }
            // An edit cancels this task and starts its own.
            if !Task.isCancelled { uploadTasks[id] = nil }
        }
    }

    private func uploadAttachmentDraft(id: String) async throws -> ChatAttachment {
        let staged = stagedFiles.values.flatMap { $0 }
        guard let channelID = draftChannels[id] ?? channelID,
              let file = (pendingDrafts + staged).first(where: { $0.id == id }),
              let prepared = preparedUploads[id] else { throw CancellationError() }
        if !file.needsUpload, let attachment = file.attachment { return attachment }
        let expected = uploadGeneration
        updateDraft(id) { $0.attachment = nil; $0.error = nil; $0.progress = 0 }
        if let old = file.attachment?.id {
            // The bytes remain owned by the draft, not the expired server ID.
            localAttachmentCopies[old] = nil
            await api.cancelAsset(id: old)
        }
        let attachment = try await AttachmentUploader.upload(prepared, channelID: channelID, api: api) { fraction in
            Task { @MainActor [weak self] in
                guard let self, self.uploadGeneration == expected else { return }
                self.updateDraft(id) { if $0.attachment == nil && $0.error == nil { $0.progress = fraction } }
            }
        }
        guard uploadGeneration == expected, !Task.isCancelled, hasDraft(id) else {
            await api.cancelAsset(id: attachment.id)
            throw CancellationError()
        }
        updateDraft(id) { $0.attachment = attachment; $0.uploadedAt = Date(); $0.progress = 1; $0.error = nil }
        return attachment
    }

    private func releasePendingUploads(keepingDrafts: Bool = false) {
        if !keepingDrafts {
            for file in pendingDrafts {
                preparedUploads[file.id] = nil; draftChannels[file.id] = nil
                // Kept only for editing; the pending and processing rows show the uploaded copy.
                if let original = file.original, original.url != file.localURL { AttachmentStaging.remove(original.url) }
                if let preview = file.editPreviewURL { AttachmentStaging.remove(preview) }
            }
        }
        pendingDrafts = []; pendingNeedsUpload = false; sendUploadFailed = false
    }

    /// Batches `POST /api/assets/urls` for attachments whose signed URLs are
    /// expiring or failed to load. Each stale URL is refreshed at most once.
    public func requestFreshAttachmentURLs(ids: [String]) {
        for id in ids {
            guard let current = signedURL(id: id), !current.hasPrefix("file:"), urlRefreshAttempts[id] != current else { continue }
            urlRefreshQueue.insert(id)
        }
        guard !urlRefreshQueue.isEmpty, urlRefreshTask == nil else { return }
        urlRefreshTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(100))
            guard !Task.isCancelled else { return }
            await self?.flushAttachmentURLRefresh()
        }
    }

    private func flushAttachmentURLRefresh() async {
        urlRefreshTask = nil
        let ids = Array(urlRefreshQueue.prefix(100))
        urlRefreshQueue.subtract(ids)
        for id in ids { urlRefreshAttempts[id] = signedURL(id: id) ?? "" }
        if !urlRefreshQueue.isEmpty { requestFreshAttachmentURLs(ids: []) }
        guard !ids.isEmpty else { return }
        let requestChannel = channelID
        guard let urls = try? await api.refreshAttachmentURLs(ids: ids), !urls.isEmpty, channelID == requestChannel else { return }
        applyFreshAttachmentURLs(urls)
    }

    /// The URL to open or play now, refreshed first when it is expiring.
    public func currentURL(for attachment: ChatAttachment) async -> URL? {
        var latest = self.attachment(id: attachment.id) ?? attachment
        if AttachmentURLPolicy.needsRefresh(latest.url) {
            let requestChannel = channelID
            if let urls = try? await api.refreshAttachmentURLs(ids: [attachment.id]), !urls.isEmpty, channelID == requestChannel {
                applyFreshAttachmentURLs(urls)
                latest = self.attachment(id: attachment.id) ?? latest
            }
        }
        return latest.url.flatMap { URL(string: $0) }
    }

    private func attachment(id: String) -> ChatAttachment? {
        for message in Array(messages.reversed()) + pinnedMessages {
            if let match = message.content.attachments?.first(where: { $0.id == id }) { return match.unavailable ? nil : match }
        }
        return nil
    }

    /// The delivery URL a refresh replaces: the file, or a processing file's preview.
    private func signedURL(id: String) -> String? {
        guard let attachment = attachment(id: id) else { return nil }
        return attachment.url ?? attachment.previewUrl
    }

    private func applyFreshAttachmentURLs(_ urls: [String: AttachmentURLs]) {
        func refresh(_ attachment: ChatAttachment) -> ChatAttachment {
            guard let fresh = urls[attachment.id], !attachment.unavailable else { return attachment }
            var refreshed = attachment
            // Only ready files have `url`; never clear a URL the response omits.
            refreshed.url = fresh.url ?? attachment.url
            refreshed.previewUrl = fresh.previewUrl ?? attachment.previewUrl
            return refreshed
        }
        func refreshMessage(_ message: ChatMessage) -> ChatMessage {
            guard let attachments = message.content.attachments, attachments.contains(where: { urls[$0.id] != nil }) else { return message }
            var updated = message
            updated.content.attachments = attachments.map(refresh)
            return updated
        }
        attachmentSnapshots.updateAttachments(refresh)
        messages = messages.map(refreshMessage)
        pinnedMessages = pinnedMessages.map(refreshMessage)
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
