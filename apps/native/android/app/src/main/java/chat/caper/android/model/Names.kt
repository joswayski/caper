package chat.caper.android.model

import chat.caper.android.data.CaperMobile

/**
 * The API's name-character rule (apps/api/src/names.rs), run by the shared Rust copy
 * (apps/native/client/src/names.rs): names (display and space names) keep visible text but not
 * control characters, Unicode Default_Ignorable_Code_Point characters (bidi controls, zero-width
 * characters, soft hyphen, Hangul fillers, tags…) or the braille blank. Emoji keep their parts: a
 * lone ZWJ/ZWNJ between visible characters, one variation selector after a visible character, and
 * tags inside a subdivision flag. Message text is not checked by this rule.
 */
fun hasHiddenNameCharacters(value: String): Boolean = CaperMobile.nameHasHidden(value)

/** Web's spaceNameError (apps/web/src/spaces/client.ts), on the name as typed (shared/names/channel-cases.json). */
internal fun spaceNameError(name: String): String? = ascii(CaperMobile.nameError(name, false))

/** The space name web sends: `name.trim()`, with JavaScript's white space. */
internal fun spaceName(name: String): String = CaperMobile.spaceName(name).let { name.substring(it[0], it[1]) }

/** Web's channelNameError; check the name without its trailing dash. */
internal fun channelNameError(name: String): String? = ascii(CaperMobile.nameError(name, true))

/** Web's normalizeChannelName: a channel name as typed, keeping a trailing dash. */
internal fun normalizeChannelName(value: String): String = ascii(CaperMobile.channelName(value)) ?: ""

private fun ascii(values: IntArray): String? = if (values.isEmpty()) null else String(CharArray(values.size) { values[it].toChar() })
