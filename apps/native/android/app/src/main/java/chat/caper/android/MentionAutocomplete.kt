package chat.caper.android

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import chat.caper.android.model.AppUiState
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.MessageMention
import java.util.Locale

// The @mentions contract shared with the API and every client: an `@` at the
// start of the text, after whitespace or after `(`, `[` or `{`, followed by a
// run of ASCII word characters. Runs longer than 32 characters are not mentions.

internal const val MENTION_NAME_LIMIT = 32

internal data class MentionToken(val start: Int, val end: Int, val query: String)

/** A person or special name the composer can suggest after `@`. */
internal data class MentionCandidate(
    val username: String,
    /** The display name for people; the muted description for `@everyone`/`@here`. */
    val label: String,
    val special: Boolean = false,
    val avatarId: Int? = null,
)

internal val mentionSpecials = listOf(
    MentionCandidate("everyone", "Everyone in this channel", special = true),
    MentionCandidate("here", "Everyone online in this channel", special = true),
)

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
 * falling back to the other participant until that list loads.
 */
internal data class MentionSource(val people: List<MentionCandidate>, val specials: Boolean)

internal fun mentionSource(state: AppUiState): MentionSource {
    val channel = state.selectedChannel ?: return MentionSource(emptyList(), false)
    val self = state.account?.id ?: state.chatAuthorId
    if (channel.direct) {
        val people = state.people?.filter { it.id != self }?.map { MentionCandidate(it.username, it.displayName, avatarId = it.avatarId) }
            ?: listOfNotNull(state.directConversations.firstOrNull { it.id == (state.selectedDirectId ?: channel.id) }?.peer
                ?.takeIf { it.id != self }?.let { MentionCandidate(it.username, it.displayName) })
        return MentionSource(people, false)
    }
    // Members not loaded yet (or another space's): only the specials.
    val members = state.selectedSpace?.takeIf { detail -> detail.space.id == channel.spaceId || detail.channels.any { it.id == channel.id } }?.members.orEmpty()
    return MentionSource(members.filter { it.id != self }.map { MentionCandidate(it.username, it.displayName, avatarId = it.avatarId) }, true)
}

/**
 * Members ranked exact username, username prefix, display-name word prefix,
 * then username substring (ties by username), followed by matching specials.
 * Specials keep their slots within [limit].
 */
internal fun mentionSuggestions(source: MentionSource, query: String, limit: Int = 6): List<MentionCandidate> {
    val needle = query.lowercase(Locale.ROOT)
    val specials = if (source.specials) mentionSpecials.filter { it.username.startsWith(needle) }.take(limit) else emptyList()
    val people = source.people.mapNotNull { person ->
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
    return people.take((limit - specials.size).coerceAtLeast(0)) + specials
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

/** Tokens the server resolved: specials with their entry, users by username. Unknown types are ignored. */
internal fun highlightedMentions(text: String, mentions: List<MessageMention>): List<MentionSpan> {
    if (mentions.isEmpty()) return emptyList()
    val everyone = mentions.any { it.type == "everyone" }
    val here = mentions.any { it.type == "here" }
    val users = mentions.filter { it.type == "user" }.mapNotNull { it.username?.lowercase(Locale.ROOT) }.toSet()
    return mentionSpans(text).filter { span ->
        when (span.name) {
            "everyone" -> everyone
            "here" -> here
            else -> span.name in users
        }
    }
}

/** The `user` entry behind a highlighted pill; null for `@everyone`/`@here`, which open nothing. */
internal fun mentionedUser(span: MentionSpan, mentions: List<MessageMention>): MessageMention? =
    if (span.name == "everyone" || span.name == "here") null
    else mentions.firstOrNull { it.type == "user" && it.username?.lowercase(Locale.ROOT) == span.name }

/** The mention card for a tagged person, from data already loaded (no network call). */
internal data class MentionCard(
    val id: String?,
    val username: String?,
    /** Null for someone you share nothing with: the title is then `@username`, without a second line. */
    val displayName: String?,
    val avatarId: Int? = null,
    val self: Boolean = false,
) {
    val title: String get() = displayName ?: username?.let { "@$it" } ?: "Profile"
    val subtitle: String? get() = if (displayName != null) username?.let { "@$it" } else null
}

/** Matches by id: loaded space members, then `/api/people`, then DM peers (then your own account), else unknown. */
internal fun mentionCard(id: String?, username: String?, state: AppUiState): MentionCard {
    val selfId = state.account?.id ?: state.chatAuthorId
    val self = if (id != null) id == selfId else username != null && state.account?.username?.equals(username, ignoreCase = true) == true
    if (id != null) {
        state.selectedSpace?.members?.firstOrNull { it.id == id }?.let { return MentionCard(id, it.username, it.displayName, it.avatarId, self) }
        state.people?.firstOrNull { it.id == id }?.let { return MentionCard(id, it.username, it.displayName, it.avatarId, self) }
        state.directConversations.firstOrNull { it.peer.id == id }?.peer?.let { return MentionCard(id, it.username, it.displayName, null, self) }
    }
    val account = state.account
    if (self && account?.username != null && account.displayName != null) {
        return MentionCard(account.id, account.username, account.displayName, account.avatarId, true)
    }
    return MentionCard(id, username, null, null, self)
}

/** A pinner has an account id and real name/avatar, but no username in pin metadata. */
internal fun authorCard(author: ChatAuthor, state: AppUiState): MentionCard {
    val card = mentionCard(author.id, null, state)
    return card.copy(displayName = card.displayName ?: author.name, avatarId = if (card.username == null) author.avatarId else card.avatarId)
}

/** You are mentioned by id, or by `@everyone`/`@here` in someone else's message. */
internal fun mentionsMe(mentions: List<MessageMention>, authorId: String, selfId: String?): Boolean {
    if (selfId == null) return false
    return mentions.any { it.type == "user" && it.id == selfId } ||
        authorId != selfId && mentions.any { it.type == "everyone" || it.type == "here" }
}
