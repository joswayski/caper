package chat.caper.android.data

import chat.caper.android.model.AppUiState
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatMessage
import chat.caper.android.model.DirectConversation
import java.math.BigInteger

/** The main "Direct messages" list: accepted and outgoing conversations, never incoming requests. */
internal fun mainDirects(conversations: List<DirectConversation>): List<DirectConversation> = conversations.filter { !it.incoming }

/** Incoming message requests, in server order. */
internal fun messageRequests(conversations: List<DirectConversation>): List<DirectConversation> = conversations.filter { it.incoming }

/** The unread dot: requests never count, whatever their sequences say. */
internal fun directUnread(conversation: DirectConversation): Boolean = !conversation.incoming &&
    runCatching { BigInteger(conversation.lastSeq) > BigInteger(conversation.readSeq) }.getOrDefault(false)

/** The gold badge after a Caper admin's name: `ADMIN BOT` for the Caper account's automatic replies. */
internal fun adminBadgeLabel(bot: Boolean): String = if (bot) "ADMIN BOT" else "ADMIN"

/** The small muted tag after a DM row's name: `Feedback` for a conversation with the Caper team. */
internal fun directRowTag(conversation: DirectConversation): String? = if (conversation.feedback) "Feedback" else null

/** The empty feedback conversation's note, in place of "Only you and … can read this conversation." */
internal const val FEEDBACK_EMPTY_NOTE = "The Caper team reads and replies here. Mention @feedback anywhere to send a message or its thread here."

/** The line under "No messages yet." in an empty channel or DM (your notes have their own text). */
internal fun emptyConversationNote(state: AppUiState): String {
    val name = state.selectedChannel?.name.orEmpty()
    return when {
        state.selectedChannel?.direct != true -> "Start the conversation in #$name."
        state.feedbackOpen -> FEEDBACK_EMPTY_NOTE
        else -> "Only you and $name can read this conversation."
    }
}

/** Message actions offer Block/Unblock for another signed-in account's message, never a guest's,
 * and never in a feedback conversation (the Caper team reads and replies there). */
internal fun offersAuthorBlock(state: AppUiState, author: ChatAuthor): Boolean =
    state.account != null && !state.feedbackOpen && !author.isGuest && author.id != (state.chatAuthorId ?: state.account.id)

/** Account ids whose messages collapse for you: never yourself or a guest. */
internal fun collapsesFor(message: ChatMessage, blocked: Set<String>, selfId: String?): Boolean =
    !message.author.isGuest && message.author.id != selfId && message.author.id in blocked

/** One timeline row: a visible message, or a run of consecutive blocked messages. */
internal sealed interface TimelineEntry {
    val first: ChatMessage
    val last: ChatMessage
    data class Shown(val message: ChatMessage) : TimelineEntry {
        override val first get() = message
        override val last get() = message
    }
    /** [key] is the run's first message id; [revealed] runs list their messages under a Hide row. */
    data class BlockedRun(val key: String, val messages: List<ChatMessage>, val revealed: Boolean) : TimelineEntry {
        override val first get() = messages.first()
        override val last get() = messages.last()
    }
}

/** Collapses each run of consecutive blocked-author messages into one row. */
internal fun groupBlocked(messages: List<ChatMessage>, blocked: Set<String>, selfId: String?, revealed: Set<String>): List<TimelineEntry> {
    if (blocked.isEmpty()) return messages.map { TimelineEntry.Shown(it) }
    val entries = mutableListOf<TimelineEntry>()
    var run = mutableListOf<ChatMessage>()
    fun flush() {
        if (run.isEmpty()) return
        val key = run.first().id
        entries += TimelineEntry.BlockedRun(key, run, key in revealed)
        run = mutableListOf()
    }
    messages.forEach { message ->
        if (collapsesFor(message, blocked, selfId)) run += message
        else { flush(); entries += TimelineEntry.Shown(message) }
    }
    flush()
    return entries
}

internal fun blockedRunLabel(count: Int): String = if (count == 1) "1 blocked message" else "$count blocked messages"

internal const val DM_NOT_ACCEPTED = "dm_not_accepted"
internal const val DM_BLOCKED = "dm_blocked"

/** Copy for the DM privacy and block errors; null for anything else. */
internal fun directMessageError(code: String?): String? = when (code) {
    DM_NOT_ACCEPTED -> "This person isn't accepting direct messages."
    DM_BLOCKED -> "You blocked this person. Unblock them to message them."
    else -> null
}

/** The DM privacy choices, in display order: value, label, optional explanation. */
internal val directPrivacyOptions = listOf(
    Triple("anyone", "Anyone", "People outside your spaces send a message request first."),
    Triple("spaces", "People in my spaces", null),
    Triple("nobody", "No one new", "Conversations you already have stay open."),
)
