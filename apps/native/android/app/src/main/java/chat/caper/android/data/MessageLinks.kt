package chat.caper.android.data

/**
 * Message links (`https://caper.chat/m/<channel>/<message>`, `caper://m/<channel>/<message>`), through
 * shared Rust (shared/messages/message_links.rs): (channel, message), or null for anything else.
 * Only caper.chat hosts count. Every client runs shared/messages/message-link-cases.json.
 */
fun messageLink(input: String): Pair<String, String>? =
    CaperMobile.messageLink(input).takeIf { it.isNotEmpty() }
        ?.let { value -> String(CharArray(value.size) { value[it].toChar() }) }
        ?.split('/')?.takeIf { it.size == 2 }?.let { it[0] to it[1] }

/** The link Copy link puts on the clipboard. */
fun messageLinkUrl(channel: String, message: String): String = "https://caper.chat/m/$channel/$message"
