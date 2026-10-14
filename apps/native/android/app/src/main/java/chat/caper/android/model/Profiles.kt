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
            avatarId = person.avatarId ?: value.avatarId, avatarAsset = asset, avatarUrl = url)
    }
    val directs = directConversations.map { conversation ->
        val peer = conversation.peer
        if (peer.id != id) conversation else {
            val (asset, url) = picture(peer.avatarAsset, peer.avatarUrl)
            conversation.copy(peer = peer.copy(username = person.username ?: peer.username, displayName = person.displayName ?: peer.displayName,
                avatarId = person.avatarId ?: peer.avatarId, avatarAsset = asset, avatarUrl = url))
        }
    }
    // A DM is titled with its peer's display name.
    val open = selectedChannel
    val name = person.displayName
    val channel = if (open != null && open.direct && name != null && selectedDirect?.peer?.id == id) open.copy(name = name) else open
    return copy(
        account = account?.let { own ->
            if (own.id != id) own else {
                val (asset, url) = picture(own.avatarAsset, own.avatarUrl)
                own.copy(username = person.username ?: own.username, displayName = person.displayName ?: own.displayName,
                    avatarId = person.avatarId ?: own.avatarId, avatarAsset = asset, avatarUrl = url)
            }
        },
        selectedSpace = selectedSpace?.let { detail -> detail.copy(members = detail.members.map(::member)) },
        channelGrants = channelGrants.map(::member),
        pendingChannelInvitations = pendingChannelInvitations.map(::member),
        pendingSpaceInvitations = pendingSpaceInvitations.map(::member),
        directConversations = directs,
        selectedChannel = channel,
        people = people?.map { value ->
            if (value.id != id) value else {
                val (asset, url) = picture(value.avatarAsset, value.avatarUrl)
                value.copy(username = person.username ?: value.username, displayName = person.displayName ?: value.displayName,
                    avatarId = person.avatarId ?: value.avatarId, avatarAsset = asset, avatarUrl = url)
            }
        },
        blocks = blocks.map { value ->
            if (value.id != id) value else {
                val (asset, url) = picture(value.avatarAsset, value.avatarUrl)
                value.copy(username = person.username ?: value.username, displayName = person.displayName ?: value.displayName,
                    avatarId = person.avatarId ?: value.avatarId, avatarAsset = asset, avatarUrl = url)
            }
        },
        messages = messages.map { it.withPerson(person, keepSignedUrls) },
        pinnedMessages = pinnedMessages.map { it.withPerson(person, keepSignedUrls) },
        pendingMessage = pendingMessage?.let { pending -> pending.copy(author = pending.author?.withPerson(person, keepSignedUrls)) },
        pinIntents = pinIntents.mapValues { (_, intent) ->
            intent.copy(message = intent.message.withPerson(person, keepSignedUrls), pin = intent.pin?.let { it.copy(author = it.author.withPerson(person, keepSignedUrls)) })
        },
        replyTargets = replyTargets.mapValues { (_, reply) -> reply.copy(author = reply.author.withPerson(person, keepSignedUrls)) },
        typingAuthors = typingAuthors.map { it.withPerson(person, keepSignedUrls) },
        feedback = if (feedback.reports.none { it.reporter.id == id }) feedback else feedback.copy(reports = feedback.reports.map { report ->
            if (report.reporter.id != id) report else report.copy(reporter = report.reporter.withPerson(person, keepSignedUrls))
        }),
        visit = visit?.let { visiting ->
            val reporter = visiting.reporter
            if (reporter != null && reporter.id == id) visiting.copy(reporter = reporter.withPerson(person, keepSignedUrls)) else visiting
        },
    )
}

/** [person]'s name and picture on this author; other authors are returned unchanged. */
fun ChatAuthor.withPerson(person: ProfilePerson, keepSignedUrls: Boolean = false): ChatAuthor {
    if (id != person.id) return this
    val (asset, url) = person.picture(avatarAsset, avatarUrl, keepSignedUrls)
    return copy(name = person.displayName ?: name, avatarId = person.avatarId ?: avatarId, avatarAsset = asset, avatarUrl = url)
}

/** The message's author, pinner, thread participants, reply reference and forwarded original. */
fun ChatMessage.withPerson(person: ProfilePerson, keepSignedUrls: Boolean = false): ChatMessage = copy(
    author = author.withPerson(person, keepSignedUrls),
    pin = pin?.let { it.copy(author = it.author.withPerson(person, keepSignedUrls)) },
    thread = thread?.let { summary -> summary.copy(participants = summary.participants.map { it.withPerson(person, keepSignedUrls) }) },
    forward = forward?.let { it.copy(message = it.message?.withPerson(person, keepSignedUrls)) },
    replyTo = replyTo?.let { it.copy(author = it.author.withPerson(person, keepSignedUrls)) },
)

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
fun ChatHistory.withPerson(person: ProfilePerson): ChatHistory = copy(
    messages = messages.map { it.withPerson(person) },
    pinnedMessages = pinnedMessages.map { it.withPerson(person) },
)

/**
 * `space.updated`: that space's name and icon in the rail, invitations, the open space (header,
 * Manage space), a visit's title and the Feedback inbox's reports. A space you don't have changes nothing.
 */
fun AppUiState.withSpace(changed: Space): AppUiState {
    fun merge(space: Space) = if (space.id != changed.id) space
        else space.copy(name = changed.name, iconAsset = changed.iconAsset, iconUrl = changed.iconUrl)
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
