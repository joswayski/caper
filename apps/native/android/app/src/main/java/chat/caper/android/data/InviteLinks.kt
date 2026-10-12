package chat.caper.android.data

import chat.caper.android.model.InviteLink
import java.time.Duration
import java.time.Instant
import kotlin.math.ceil
import kotlin.math.roundToLong

/** Web's copy for a link that does not exist, was revoked, expired or is used up. */
const val INVALID_INVITE_LINK = "This invite link is invalid or has expired."

private val bareInviteCode = Regex("^[A-Za-z0-9]{10}$")
/** `[scheme://]host/invite/<code>[/][?query][#fragment]`, on any host. */
private val inviteUrlPattern = Regex("^(?:[A-Za-z][A-Za-z0-9+.-]*://)?[^/?#\\s]+/invite/([A-Za-z0-9]{10})/?(?:[?#]\\S*)?$")

internal fun isInviteCode(value: String): Boolean = bareInviteCode.matches(value)

/**
 * The invite code in what someone pasted: a full link on any host (`https://caper.chat/invite/<code>`,
 * with an optional trailing slash, query or fragment), the same without a scheme, or a bare
 * 10-character code. Null when it is none of those.
 */
fun inviteCode(input: String): String? {
    val text = input.trim()
    if (bareInviteCode.matches(text)) return text
    return inviteUrlPattern.matchEntire(text)?.groupValues?.get(1)
}

/** `<origin>/invite/<code>`, for the configured API origin. */
fun inviteLinkUrl(origin: String, code: String): String = "${origin.trimEnd('/')}/invite/$code"

/** "Expire after" choices, in seconds; null never expires. */
val INVITE_MAX_AGES: List<Pair<Int?, String>> = listOf(
    3_600 to "1 hour", 86_400 to "1 day", 604_800 to "7 days", 2_592_000 to "30 days", null to "Never",
)
const val DEFAULT_INVITE_MAX_AGE = 604_800

/** "Max uses" choices; null has no limit (the default). */
val INVITE_MAX_USES: List<Pair<Int?, String>> = listOf<Pair<Int?, String>>(null to "No limit") +
    listOf(1, 5, 10, 25, 50, 100, 500, 1_000, 5_000, 10_000).map { it to plural(it.toLong(), "use") }

/** "10,000" */
private fun grouped(count: Long) = String.format(java.util.Locale.US, "%,d", count)

private fun plural(count: Long, one: String, many: String = "${one}s") = "${grouped(count)} ${if (count == 1L) one else many}"

/** "3 of 10 uses", or "2 uses" without a limit. */
fun inviteUsesLabel(uses: Int, maxUses: Int?): String =
    if (maxUses != null) "${grouped(uses.toLong())} of ${plural(maxUses.toLong(), "use")}" else plural(uses.toLong(), "use")

/** "Expires in 6 days", "Never expires", or "Expired". Null when [expiresAt] can't be read. */
fun inviteExpiryLabel(expiresAt: String?, now: Instant): String? {
    if (expiresAt == null) return "Never expires"
    val end = runCatching { Instant.parse(expiresAt) }.getOrNull() ?: return null
    val seconds = Duration.between(now, end).seconds
    if (seconds <= 0) return "Expired"
    val minutes = ceil(seconds / 60.0).toLong()
    if (minutes < 60) return "Expires in ${plural(minutes, "minute")}"
    val hours = (minutes / 60.0).roundToLong()
    if (hours < 24) return "Expires in ${plural(hours, "hour")}"
    return "Expires in ${plural((hours / 24.0).roundToLong(), "day")}"
}

/** A link row's meta line: "#feedback · 3 of 10 uses · Expires in 6 days". */
fun inviteLinkMeta(link: InviteLink, now: Instant): String = listOfNotNull(
    link.channel?.let { "#${it.name}" },
    inviteUsesLabel(link.uses, link.maxUses),
    inviteExpiryLabel(link.expiresAt, now),
).joinToString(" · ")

/** "1 member" or "12 members". */
fun inviteMemberCount(count: Int): String = plural(count.toLong(), "member")
