package chat.caper.android

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
import chat.caper.android.data.linkRanges
import chat.caper.android.model.AppUiState
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.MessageMention
import chat.caper.android.ui.Terracotta
import java.util.Locale

// The @mentions contract shared with the API and every client: an `@` at the
// start of the text, after whitespace or after `(`, `[` or `{`, followed by a
// run of ASCII word characters. Runs longer than 32 characters are not mentions.

internal const val MENTION_NAME_LIMIT = 32

internal data class MentionToken(val start: Int, val end: Int, val query: String)

/** A person or special name the composer can suggest after `@` (people also fill username fields). */
internal data class MentionCandidate(
    val username: String,
    /** The display name for people; the muted description for `@everyone`/`@here`/team names. */
    val label: String,
    val special: Boolean = false,
    val avatarId: Int? = null,
    /** Already has a pending invitation to the space or channel being managed; the row says so. */
    val invited: Boolean = false,
    /** The person's custom picture, over the [avatarId] mascot. */
    val avatarAsset: String? = null,
    val avatarUrl: String? = null,
)

internal val mentionSpecials = listOf(
    MentionCandidate("everyone", "Everyone in this channel", special = true),
    MentionCandidate("here", "Everyone online in this channel", special = true),
)

/**
 * The names that ask the Caper team to look (all reserved usernames), in the server's order.
 * Any of them in a message with a `feedback` mention entry is a pill.
 */
internal val FEEDBACK_NAMES = listOf(
    "feedback", "caper", "capers", "caperchat", "caper_feedback", "caper_support", "support", "team", "staff",
    "admin", "admins", "administrator", "administrators", "mod", "mods", "moderator", "moderators",
)

/** The muted description of the team suggestion. */
internal const val FEEDBACK_SUGGESTION_LABEL = "Notify the Caper team"

/**
 * The one team suggestion for a non-empty [query]: the first team name that starts with it,
 * inserted as named (`@ad` → `@admin`, `@cap` → `@caper`, `@fe` → `@feedback`). Offered in
 * every conversation (channels, threads, DMs and your notes); a bare `@` lists `@feedback` first instead (see [mentionSuggestions]).
 */
internal fun feedbackSuggestion(query: String): MentionCandidate? {
    val needle = query.lowercase(Locale.ROOT)
    if (needle.isEmpty()) return null
    return FEEDBACK_NAMES.firstOrNull { it.startsWith(needle) }?.let { MentionCandidate(it, FEEDBACK_SUGGESTION_LABEL, special = true) }
}

/** The team suggestion, whose description gets its own line in the popup. */
internal val MentionCandidate.teamSuggestion: Boolean get() = special && label == FEEDBACK_SUGGESTION_LABEL && username in FEEDBACK_NAMES

internal fun Char.isMentionNameChar(): Boolean =
    this in 'a'..'z' || this in 'A'..'Z' || this in '0'..'9' || this == '_'

private fun Char.startsMention(): Boolean = isWhitespace() || this in "([{"

/** The `@name` token ending at a collapsed caret, outside IME composition. */
internal fun mentionToken(value: TextFieldValue): MentionToken? {
    if (!value.selection.collapsed || value.composition != null) return null
    val text = value.text
    val caret = value.selection.start
    if (caret < text.length && (text[caret] == '@' || text[caret].isMentionNameChar())) return null
    var at = caret - 1
    while (at >= 0 && text[at].isMentionNameChar()) at--
    if (at < 0 || text[at] != '@' || caret - at - 1 > MENTION_NAME_LIMIT) return null
    if (at > 0 && !text[at - 1].startsMention()) return null
    return MentionToken(at, caret, text.substring(at + 1, caret))
}

/**
 * Who the composer may suggest: the space's members (excluding you) plus the
 * specials in a space channel; in a DM, everyone from `GET /api/people`,
 * falling back to the other participant until that list loads. A visit has no
 * member list: the reporter and people the loaded messages name. [feedback]
 * offers the team suggestion (signed-in accounts, every conversation).
 */
internal data class MentionSource(val people: List<MentionCandidate>, val specials: Boolean, val feedback: Boolean = false)

internal fun mentionSource(state: AppUiState): MentionSource {
    val channel = state.selectedChannel ?: return MentionSource(emptyList(), false)
    val self = state.account?.id ?: state.chatAuthorId
    val feedback = state.account != null
    state.visit?.let { return MentionSource(visitMentionPeople(state, self), false, feedback) }
    if (channel.direct) {
        val people = state.people?.filter { it.id != self }?.map {
            MentionCandidate(it.username, it.displayName, avatarId = it.avatarId, avatarAsset = it.avatarAsset, avatarUrl = it.avatarUrl)
        } ?: listOfNotNull(state.directConversations.firstOrNull { it.id == (state.selectedDirectId ?: channel.id) }?.peer
                ?.takeIf { it.id != self }?.let { MentionCandidate(it.username, it.displayName, avatarId = it.avatarId, avatarAsset = it.avatarAsset, avatarUrl = it.avatarUrl) })
        return MentionSource(people, false, feedback)
    }
    // Members not loaded yet (or another space's): only the specials.
    val members = state.selectedSpace?.takeIf { detail -> detail.space.id == channel.spaceId || detail.channels.any { it.id == channel.id } }?.members.orEmpty()
    return MentionSource(members.filter { it.id != self }.map {
        MentionCandidate(it.username, it.displayName, avatarId = it.avatarId, avatarAsset = it.avatarAsset, avatarUrl = it.avatarUrl)
    }, true, feedback)
}

/**
 * A visit's people, without a member list: the reporter, then accounts the loaded messages
 * @mention, then authors whose usernames you already know (`/api/people`, DM peers). Never you.
 */
internal fun visitMentionPeople(state: AppUiState, self: String?): List<MentionCandidate> {
    val authors = state.messages.map { it.author }.filter { !it.isGuest }.associateBy { it.id }
    val known = state.people.orEmpty().associateBy { it.id }
    val peers = state.directConversations.associate { it.peer.id to it.peer }
    val people = linkedMapOf<String, MentionCandidate>()
    fun add(id: String?, username: String?, name: String?, avatarId: Int?, avatarAsset: String? = null, avatarUrl: String? = null) {
        if (username.isNullOrEmpty() || id == self || username.lowercase(Locale.ROOT) in FEEDBACK_NAMES) return
        val key = id ?: "@${username.lowercase(Locale.ROOT)}"
        if (key !in people) people[key] = MentionCandidate(username, name ?: username, avatarId = avatarId, avatarAsset = avatarAsset, avatarUrl = avatarUrl)
    }
    state.visit?.reporter?.let { add(it.id, it.username, it.displayName, it.avatarId, it.avatarAsset, it.avatarUrl) }
    state.messages.flatMap { it.content.mentions }.filter { it.type == "user" }.forEach { mention ->
        val author = mention.id?.let(authors::get)
        add(mention.id, mention.username, author?.name, author?.avatarId, author?.avatarAsset, author?.avatarUrl)
    }
    authors.values.forEach { author ->
        val username = known[author.id]?.username ?: peers[author.id]?.username
        add(author.id, username, author.name, author.avatarId, author.avatarAsset, author.avatarUrl)
    }
    return people.values.toList()
}

/**
 * Members ranked by [rankPeople], followed by matching specials (`@everyone`, `@here`,
 * then one team name once something is typed, see [feedbackSuggestion]). A bare `@` puts
 * `@feedback` first instead, so the team is easy to find. Specials keep their slots within [limit].
 */
internal fun mentionSuggestions(source: MentionSource, query: String, limit: Int = 6): List<MentionCandidate> {
    val needle = query.lowercase(Locale.ROOT)
    val first = if (source.feedback && needle.isEmpty()) listOf(MentionCandidate(FEEDBACK_NAMES.first(), FEEDBACK_SUGGESTION_LABEL, special = true)) else emptyList()
    val specials = ((if (source.specials) mentionSpecials.filter { it.username.startsWith(needle) } else emptyList()) +
        listOfNotNull(if (source.feedback) feedbackSuggestion(needle) else null)).take((limit - first.size).coerceAtLeast(0))
    return first + rankPeople(source.people, needle).take((limit - first.size - specials.size).coerceAtLeast(0)) + specials
}

/**
 * People matching [query] case-insensitively, ranked exact username, username prefix,
 * display-name (or display-name word) prefix, then username substring, ties by username.
 * An empty query keeps everyone (a bare `@`); username fields never ask with one.
 */
internal fun rankPeople(people: List<MentionCandidate>, query: String): List<MentionCandidate> {
    val needle = query.lowercase(Locale.ROOT)
    return people.mapNotNull { person ->
        val username = person.username.lowercase(Locale.ROOT)
        val display = person.label.lowercase(Locale.ROOT)
        val rank = when {
            needle.isEmpty() -> 1
            username == needle -> 0
            username.startsWith(needle) -> 1
            display.startsWith(needle) || display.split(' ').any { it.startsWith(needle) } -> 2
            needle in username -> 3
            else -> return@mapNotNull null
        }
        Triple(rank, username, person)
    }.sortedWith(compareBy({ it.first }, { it.second })).map { it.third }
}

/** Replaces the token with `@username ` and leaves the caret after the space. */
internal fun insertMention(value: TextFieldValue, token: MentionToken, username: String, limit: Int = 4000): TextFieldValue? {
    val inserted = "@$username "
    val text = value.text.substring(0, token.start) + inserted + value.text.substring(token.end)
    if (text.codePointCount(0, text.length) > limit) return null
    return TextFieldValue(text, TextRange(token.start + inserted.length))
}

/** A grammatical mention in message text; [name] is lowercase. */
internal data class MentionSpan(val start: Int, val end: Int, val name: String)

internal fun mentionSpans(text: String): List<MentionSpan> {
    val spans = mutableListOf<MentionSpan>()
    var index = 0
    while (index < text.length) {
        if (text[index] != '@' || index > 0 && !text[index - 1].startsMention()) { index++; continue }
        var end = index + 1
        while (end < text.length && text[end].isMentionNameChar()) end++
        if (end - index - 1 in 1..MENTION_NAME_LIMIT) spans += MentionSpan(index, end, text.substring(index + 1, end).lowercase(Locale.ROOT))
        // The character after a run is never a start, so resume there.
        index = end
    }
    return spans
}

/** Complete locally known mentions; unresolved/partial names stay plain while typing. */
internal fun draftMentionSpans(text: String, source: MentionSource): List<MentionSpan> {
    val people = source.people.map { it.username.lowercase(Locale.ROOT) }.toSet()
    return mentionSpans(text).filter { span ->
        when (span.name) {
            "everyone", "here" -> source.specials
            in FEEDBACK_NAMES -> source.feedback
            else -> span.name in people
        }
    }
}

/** Background only: selection, IME offsets and the submitted text are unchanged. */
internal class MentionVisualTransformation(private val source: MentionSource) : VisualTransformation {
    override fun filter(text: AnnotatedString): TransformedText {
        val styled = AnnotatedString.Builder(text)
        val mentions = draftMentionSpans(text.text, source)
        for (span in mentions) {
            styled.addStyle(SpanStyle(background = Terracotta.copy(alpha = 0.24f)), span.start, span.end)
        }
        for (link in linkRanges(text.text, mentions.map { it.start until it.end })) {
            styled.addStyle(SpanStyle(background = Terracotta.copy(alpha = 0.24f)), link.start, link.end)
        }
        return TransformedText(styled.toAnnotatedString(), OffsetMapping.Identity)
    }
}

/**
 * Tokens the server resolved: specials with their entry (every team name in [FEEDBACK_NAMES]
 * with a `feedback` entry), users by username. Unknown types are ignored.
 */
internal fun highlightedMentions(text: String, mentions: List<MessageMention>): List<MentionSpan> {
    if (mentions.isEmpty()) return emptyList()
    val everyone = mentions.any { it.type == "everyone" }
    val here = mentions.any { it.type == "here" }
    val feedback = mentions.any { it.type == "feedback" }
    val users = mentions.filter { it.type == "user" }.mapNotNull { it.username?.lowercase(Locale.ROOT) }.toSet()
    return mentionSpans(text).filter { span ->
        when (span.name) {
            "everyone" -> everyone
            "here" -> here
            in FEEDBACK_NAMES -> feedback
            else -> span.name in users
        }
    }
}

/** The `user` entry behind a highlighted pill; null for `@everyone`/`@here` and team names, which open nothing. */
internal fun mentionedUser(span: MentionSpan, mentions: List<MessageMention>): MessageMention? =
    if (span.name == "everyone" || span.name == "here" || span.name in FEEDBACK_NAMES) null
    else mentions.firstOrNull { it.type == "user" && it.username?.lowercase(Locale.ROOT) == span.name }

/** The mention card for a tagged person, from data already loaded (no network call). */
internal data class MentionCard(
    val id: String?,
    val username: String?,
    /** Null for someone you share nothing with: the title is then `@username`, without a second line. */
    val displayName: String?,
    val avatarId: Int? = null,
    val self: Boolean = false,
    /** The person's custom picture, over the [avatarId] mascot. */
    val avatarAsset: String? = null,
    val avatarUrl: String? = null,
    val admin: Boolean = false,
    val team: Boolean = false,
) {
    val canBlock: Boolean get() = !admin && !team
    val title: String get() = displayName ?: username?.let { "@$it" } ?: "Profile"
    val subtitle: String? get() = if (displayName != null) username?.let { "@$it" } else null
}

/** Matches by id: loaded space members, then `/api/people`, then DM peers (then your own account), else unknown. */
internal fun mentionCard(id: String?, username: String?, state: AppUiState): MentionCard {
    val selfId = state.account?.id ?: state.chatAuthorId
    val self = if (id != null) id == selfId else username != null && state.account?.username?.equals(username, ignoreCase = true) == true
    if (id != null) {
        val peer = state.directConversations.firstOrNull { it.peer.id == id }?.peer
        val admin = peer?.admin == true || state.people.orEmpty().any { it.id == id && it.admin } || state.selectedSpace?.members.orEmpty().any { it.id == id && it.admin }
        val team = peer?.team == true
        state.selectedSpace?.members?.firstOrNull { it.id == id }?.let { return MentionCard(id, it.username, it.displayName, it.avatarId, self, it.avatarAsset, it.avatarUrl, admin, team) }
        state.people?.firstOrNull { it.id == id }?.let { return MentionCard(id, it.username, it.displayName, it.avatarId, self, it.avatarAsset, it.avatarUrl, admin, team) }
        peer?.let { return MentionCard(id, it.username, it.displayName, it.avatarId, self, it.avatarAsset, it.avatarUrl, admin, team) }
    }
    val account = state.account
    if (self && account?.username != null && account.displayName != null) {
        return MentionCard(account.id, account.username, account.displayName, account.avatarId, true, account.avatarAsset, account.avatarUrl)
    }
    return MentionCard(id, username, null, null, self)
}

/**
 * An open card follows live profile changes (`profile.updated`) of someone this client knows by
 * ID: their current name, username and picture. Anyone else keeps the card as it opened.
 */
internal fun refreshedCard(card: MentionCard, state: AppUiState): MentionCard {
    val id = card.id ?: return card
    val known = mentionCard(id, card.username, state)
    return if (known.displayName == null) card else known.copy(self = card.self, admin = known.admin || card.admin, team = known.team || card.team)
}

/** A pinner has an account id and real name/avatar, but no username in pin metadata. */
internal fun authorCard(author: ChatAuthor, state: AppUiState): MentionCard {
    val known = mentionCard(author.id, null, state)
    val card = known.copy(admin = known.admin || author.admin)
    return if (card.username == null) card.copy(displayName = card.displayName ?: author.name, avatarId = author.avatarId, avatarAsset = author.avatarAsset, avatarUrl = author.avatarUrl)
    else card.copy(displayName = card.displayName ?: author.name)
}

/** You are mentioned by id, or by `@everyone`/`@here` in someone else's message. */
internal fun mentionsMe(mentions: List<MessageMention>, authorId: String, selfId: String?): Boolean {
    if (selfId == null) return false
    return mentions.any { it.type == "user" && it.id == selfId } ||
        authorId != selfId && mentions.any { it.type == "everyone" || it.type == "here" }
}
