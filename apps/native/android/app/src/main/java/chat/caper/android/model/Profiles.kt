package chat.caper.android.model

import kotlinx.serialization.Serializable

/**
 * A person's current public profile: a `profile.updated` event from the gateway's `profiles`
 * subscription, or your own account. Absent picture fields mean the picture was removed;
 * absent names (never sent today) keep what each copy has.
 */
@Serializable data class ProfilePerson(
    val id: String,
    val username: String? = null,
    val displayName: String? = null,
    val avatarId: Int? = null,
    val avatarAsset: String? = null,
    val avatarUrl: String? = null,
    val admin: Boolean? = null,
) {
    /**
     * The picture a retained copy (showing [asset] at [url]) takes. With [keepSignedUrl], a copy
     * that already names this picture keeps its own URL, which may have been signed more recently.
     */
    internal fun picture(asset: String?, url: String?, keepSignedUrl: Boolean): Pair<String?, String?> = when {
        avatarAsset == null -> null to null
        keepSignedUrl && asset == avatarAsset && url != null -> asset to url
        else -> avatarAsset to avatarUrl
    }
}

/** One event from the gateway's `profiles` subscription. */
sealed interface ProfileEvent {
    /** `profile.updated`: someone who shares a space or DM with you (or you, on another device). */
    data class Person(val person: ProfilePerson) : ProfileEvent
    /** `space.updated`: a space you are in was renamed or its icon changed. */
    data class SpaceChanged(val space: Space) : ProfileEvent
}

fun Account.profile(): ProfilePerson = ProfilePerson(id, username, displayName, avatarId, avatarAsset, avatarUrl)

// Server lists (members, invitations, DM peers, people, blocks) return each person's current profile.
fun Member.profile(): ProfilePerson = ProfilePerson(id, username, displayName, avatarId, avatarAsset, avatarUrl, admin)
fun DirectPeer.profile(): ProfilePerson = ProfilePerson(id, username, displayName, avatarId, avatarAsset, avatarUrl, admin)
fun Person.profile(): ProfilePerson = ProfilePerson(id, username, displayName, avatarId, avatarAsset, avatarUrl, admin)
fun BlockedAccount.profile(): ProfilePerson = ProfilePerson(id, username, displayName, avatarId, avatarAsset, avatarUrl)

/** Every author [ChatMessage.withPerson] rewrites: its own, the pinner, thread participants, the reply reference's and the forwarded original's. */
fun ChatMessage.authors(): Sequence<ChatAuthor> = sequence {
    yield(author)
    pin?.let { yield(it.author) }
    thread?.let { yieldAll(it.participants) }
    replyTo?.let { yield(it.author) }
    forward?.message?.let { yieldAll(it.authors()) }
}

/**
 * Of [fetched] (current profiles a server list just returned), the people a retained copy in this
 * state or in [cached] conversations still shows with another name, username or picture: a
 * `profile.updated` missed while the gateway was away, since there is no replay. Your own copies
 * follow the account instead ([withAccountProfile]).
 */
fun AppUiState.staleProfiles(fetched: List<ProfilePerson>, cached: Collection<ChatHistory> = emptyList()): List<ProfilePerson> {
    val current = fetched.filter { it.id != account?.id }.associateBy { it.id }
    if (current.isEmpty()) return emptyList()
    val stale = linkedMapOf<String, ProfilePerson>()
    fun compare(id: String, username: String?, name: String, avatarId: Int?, asset: String?) {
        val person = current[id] ?: return
        if (id in stale) return
        if (username != null && person.username != null && username != person.username ||
            person.displayName != null && name != person.displayName ||
            person.avatarId != null && avatarId != person.avatarId || asset != person.avatarAsset) stale[id] = person
    }
    fun member(value: Member) = compare(value.id, value.username, value.displayName, value.avatarId, value.avatarAsset)
    fun message(value: ChatMessage) = value.authors().forEach { compare(it.id, null, it.name, it.avatarId, it.avatarAsset) }
    selectedSpace?.members?.forEach(::member)
    (channelGrants + pendingChannelInvitations + pendingSpaceInvitations).forEach(::member)
    directConversations.forEach { val peer = it.peer; compare(peer.id, peer.username, peer.displayName, peer.avatarId, peer.avatarAsset) }
    people?.forEach { compare(it.id, it.username, it.displayName, it.avatarId, it.avatarAsset) }
    blocks.forEach { compare(it.id, it.username, it.displayName, it.avatarId, it.avatarAsset) }
    messages.forEach(::message)
    pinnedMessages.forEach(::message)
    cached.forEach { history -> history.messages.forEach(::message); history.pinnedMessages.forEach(::message) }
    return stale.values.toList()
}

/**
 * Every retained copy of [person] by account ID: the signed-in account, space and channel
 * members and invitations, DM peers (and the open DM's title), people suggestions, blocked
 * accounts, message authors including pins, thread participants, forwarded originals and reply
 * references, the pending send, reply targets, typing, and feedback reporters. Names, usernames
 * and pictures change; message text, cursors and selection never do. Pure; see
 * [withAccountProfile] for your own account.
 */
fun AppUiState.withPerson(person: ProfilePerson, keepSignedUrls: Boolean = false): AppUiState {
    val id = person.id
    fun picture(asset: String?, url: String?) = person.picture(asset, url, keepSignedUrls)
    fun member(value: Member) = if (value.id != id) value else {
        val (asset, url) = picture(value.avatarAsset, value.avatarUrl)
        value.copy(username = person.username ?: value.username, displayName = person.displayName ?: value.displayName,
            avatarId = person.avatarId ?: value.avatarId, avatarAsset = asset, avatarUrl = url, admin = person.admin ?: value.admin).retaining(value)
    }
    val directs = directConversations.mapRetaining { conversation ->
        val peer = conversation.peer
        if (peer.id != id) conversation else {
            val (asset, url) = picture(peer.avatarAsset, peer.avatarUrl)
            conversation.copy(peer = peer.copy(username = person.username ?: peer.username, displayName = person.displayName ?: peer.displayName,
                avatarId = person.avatarId ?: peer.avatarId, avatarAsset = asset, avatarUrl = url, admin = person.admin ?: peer.admin)).retaining(conversation)
        }
    }
    // A DM is titled with its peer's display name.
    val open = selectedChannel
    val name = person.displayName
    val channel = if (open != null && open.direct && name != null && selectedDirect?.peer?.id == id) open.copy(name = name).retaining(open) else open
    return copy(
        account = account?.let { own ->
            if (own.id != id) own else {
                val (asset, url) = picture(own.avatarAsset, own.avatarUrl)
                own.copy(username = person.username ?: own.username, displayName = person.displayName ?: own.displayName,
                    avatarId = person.avatarId ?: own.avatarId, avatarAsset = asset, avatarUrl = url).retaining(own)
            }
        },
        selectedSpace = selectedSpace?.let { detail ->
            val members = detail.members.mapRetaining(::member)
            if (members === detail.members) detail else detail.copy(members = members)
        },
        channelGrants = channelGrants.mapRetaining(::member),
        pendingChannelInvitations = pendingChannelInvitations.mapRetaining(::member),
        pendingSpaceInvitations = pendingSpaceInvitations.mapRetaining(::member),
        directConversations = directs,
        selectedChannel = channel,
        people = people?.mapRetaining { value ->
            if (value.id != id) value else {
                val (asset, url) = picture(value.avatarAsset, value.avatarUrl)
                value.copy(username = person.username ?: value.username, displayName = person.displayName ?: value.displayName,
                    avatarId = person.avatarId ?: value.avatarId, avatarAsset = asset, avatarUrl = url, admin = person.admin ?: value.admin).retaining(value)
            }
        },
        blocks = blocks.mapRetaining { value ->
            if (value.id != id) value else {
                val (asset, url) = picture(value.avatarAsset, value.avatarUrl)
                value.copy(username = person.username ?: value.username, displayName = person.displayName ?: value.displayName,
                    avatarId = person.avatarId ?: value.avatarId, avatarAsset = asset, avatarUrl = url).retaining(value)
            }
        },
        messages = messages.mapRetaining { it.withPerson(person, keepSignedUrls) },
        pinnedMessages = pinnedMessages.mapRetaining { it.withPerson(person, keepSignedUrls) },
        pendingMessage = pendingMessage?.let { pending ->
            val author = pending.author?.withPerson(person, keepSignedUrls)
            if (author === pending.author) pending else pending.copy(author = author)
        },
        pinIntents = pinIntents.mapValuesRetaining { intent ->
            intent.copy(message = intent.message.withPerson(person, keepSignedUrls), pin = intent.pin?.let { it.copy(author = it.author.withPerson(person, keepSignedUrls)) }).retaining(intent)
        },
        replyTargets = replyTargets.mapValuesRetaining { reply ->
            val author = reply.author.withPerson(person, keepSignedUrls)
            if (author === reply.author) reply else reply.copy(author = author)
        },
        typingAuthors = typingAuthors.mapRetaining { it.withPerson(person, keepSignedUrls) },
        feedback = if (feedback.reports.none { it.reporter.id == id }) feedback else feedback.copy(reports = feedback.reports.map { report ->
            if (report.reporter.id != id) report else report.copy(reporter = report.reporter.withPerson(person, keepSignedUrls))
        }),
        visit = visit?.let { visiting ->
            val reporter = visiting.reporter
            if (reporter != null && reporter.id == id) visiting.copy(reporter = reporter.withPerson(person, keepSignedUrls)) else visiting
        },
    )
}

/**
 * The view model resolves your own profile ([AppUiState.withAccountProfile]) on every state
 * emission, so copies that end up equal stay the same instances: equality checks stay cheap,
 * and lists in which nobody changed are not rebuilt.
 */
private fun <T> T.retaining(previous: T): T = if (this == previous) previous else this

/** [transform] applied to each item, or this same list when it returned every item unchanged. */
internal inline fun <T> List<T>.mapRetaining(transform: (T) -> T): List<T> {
    var changed: ArrayList<T>? = null
    for (index in indices) {
        val item = this[index]
        val next = transform(item)
        if (changed == null && next !== item) changed = ArrayList<T>(size).also { list -> for (before in 0 until index) list.add(this[before]) }
        changed?.add(next)
    }
    return changed ?: this
}

private inline fun <K, V> Map<K, V>.mapValuesRetaining(transform: (V) -> V): Map<K, V> {
    val next = mapValues { (_, value) -> transform(value) }
    return if (next.all { (key, value) -> value === getValue(key) }) this else next
}

/** [person]'s name and picture on this author; other authors are returned unchanged. */
fun ChatAuthor.withPerson(person: ProfilePerson, keepSignedUrls: Boolean = false): ChatAuthor {
    if (id != person.id) return this
    val (asset, url) = person.picture(avatarAsset, avatarUrl, keepSignedUrls)
    return copy(name = person.displayName ?: name, avatarId = person.avatarId ?: avatarId, avatarAsset = asset, avatarUrl = url, admin = person.admin ?: admin).retaining(this)
}

/** The message's author, pinner, thread participants, reply reference and forwarded original; this message when none is [person]. */
fun ChatMessage.withPerson(person: ProfilePerson, keepSignedUrls: Boolean = false): ChatMessage {
    if (authors().none { it.id == person.id }) return this
    return copy(
        author = author.withPerson(person, keepSignedUrls),
        pin = pin?.let { it.copy(author = it.author.withPerson(person, keepSignedUrls)) },
        thread = thread?.let { summary -> summary.copy(participants = summary.participants.map { it.withPerson(person, keepSignedUrls) }) },
        forward = forward?.let { it.copy(message = it.message?.withPerson(person, keepSignedUrls)) },
        replyTo = replyTo?.let { it.copy(author = it.author.withPerson(person, keepSignedUrls)) },
    ).retaining(this)
}

fun Reactor.withPerson(person: ProfilePerson): Reactor {
    if (id != person.id) return this
    val (asset, url) = person.picture(avatarAsset, avatarUrl, keepSignedUrl = false)
    return copy(username = person.username ?: username, displayName = person.displayName ?: displayName,
        avatarId = person.avatarId ?: avatarId, avatarAsset = asset, avatarUrl = url)
}

/** Who reacted, with [person]'s current name and picture. */
fun ReactorList.withPerson(person: ProfilePerson): ReactorList =
    if (reactions.none { group -> group.authors.any { it.id == person.id } }) this
    else copy(reactions = reactions.map { group -> group.copy(authors = group.authors.map { it.withPerson(person) }) })

fun FeedbackReporter.withPerson(person: ProfilePerson, keepSignedUrls: Boolean = false): FeedbackReporter {
    if (id != person.id) return this
    val (asset, url) = person.picture(avatarAsset, avatarUrl, keepSignedUrls)
    return copy(username = person.username ?: username, displayName = person.displayName ?: displayName,
        avatarId = person.avatarId ?: avatarId, avatarAsset = asset, avatarUrl = url)
}

/** A loaded or cached conversation page with [person]'s current name and picture. */
fun ChatHistory.withPerson(person: ProfilePerson, keepSignedUrls: Boolean = false): ChatHistory = copy(
    messages = messages.map { it.withPerson(person, keepSignedUrls) },
    pinnedMessages = pinnedMessages.map { it.withPerson(person, keepSignedUrls) },
)

/**
 * `space.updated`: that space's name and icon in the rail, invitations, the open space (header,
 * Manage space), a visit's title and the Feedback inbox's reports. A space you don't have changes nothing.
 */
fun AppUiState.withSpace(changed: Space): AppUiState {
    fun merge(space: Space) = if (space.id != changed.id) space
        else space.copy(name = changed.name, iconAsset = changed.iconAsset, iconUrl = changed.iconUrl,
            canEditAppearance = changed.canEditAppearance ?: space.canEditAppearance)
    return copy(
        spaces = spaces.map(::merge),
        invitations = invitations.map(::merge),
        notificationInvitation = notificationInvitation?.let(::merge),
        selectedSpace = selectedSpace?.let { detail -> if (detail.space.id == changed.id) detail.copy(space = merge(detail.space)) else detail },
        visit = visit?.let { open -> if (open.spaceId == changed.id) open.copy(spaceName = changed.name) else open },
        feedback = if (feedback.reports.none { it.space?.id == changed.id }) feedback else feedback.copy(reports = feedback.reports.map { report ->
            report.space?.takeIf { it.id == changed.id }?.let { report.copy(space = it.copy(name = changed.name)) } ?: report
        }),
    )
}
