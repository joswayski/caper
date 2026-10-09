package chat.caper.android

/**
 * Unsent composer text of each conversation by channel or DM ID, so switching
 * conversations and coming back restores it. Signing out clears every draft.
 */
internal class ConversationDrafts {
    private val drafts = mutableMapOf<String, String>()

    operator fun get(conversation: String): String = drafts[conversation].orEmpty()

    operator fun set(conversation: String, text: String) {
        if (text.isEmpty()) drafts.remove(conversation) else drafts[conversation] = text
    }

    fun clear() = drafts.clear()
}
