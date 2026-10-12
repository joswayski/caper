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
/** The app's own scheme, `caper://invite/<code>[/][?query][#fragment]` (scheme and host in any case). */
private val appInvitePattern = Regex("^(?i:caper)://(?i:invite)/([A-Za-z0-9]{10})/?(?:[?#]\\S*)?$")

internal fun isInviteCode(value: String): Boolean = bareInviteCode.matches(value)

/**
 * The invite code in what someone pasted or opened: a full link on any host
 * (`https://caper.chat/invite/<code>`, with an optional trailing slash, query or fragment), the
 * same without a scheme, the app link `caper://invite/<code>`, or a bare 10-character code. Null
 * when it is none of those.
 */
fun inviteCode(input: String): String? {
    val text = input.trim()
    if (bareInviteCode.matches(text)) return text
    return (inviteUrlPattern.matchEntire(text) ?: appInvitePattern.matchEntire(text))?.groupValues?.get(1)
}

/** `<origin>/invite/<code>`, for the configured API origin. */
fun inviteLinkUrl(origin: String, code: String): String = "${origin.trimEnd('/')}/invite/$code"

/** "Expire after" units: [seconds] per unit of the number field, null for "Never" (no number). */
enum class InviteExpiryUnit(val label: String, val seconds: Long?) {
    Minutes("minutes", 60), Hours("hours", 3_600), Days("days", 86_400), Never("Never", null),
}

/** Web's default expiry, 7 days. */
const val DEFAULT_INVITE_EXPIRY_NUMBER = "7"
val DEFAULT_INVITE_EXPIRY_UNIT = InviteExpiryUnit.Days

/** The API's range for `maxAge`: 1 minute to 365 days, in seconds. */
const val MIN_INVITE_MAX_AGE = 60L
const val MAX_INVITE_MAX_AGE = 31_536_000L
/** The API's largest `maxUses`. */
const val MAX_INVITE_MAX_USES = 10_000L

const val INVALID_INVITE_EXPIRY = "Expiry must be between 1 minute and 365 days."
const val INVALID_INVITE_MAX_USES = "Max uses must be a whole number from 1 to 10,000."

/** A parsed "Expire after" or "Max uses" field: what to send (null never expires / has no limit), or its inline error. */
sealed interface InviteOption {
    data class Valid(val value: Int?) : InviteOption
    data class Invalid(val error: String) : InviteOption
}

private val wholeNumber = Regex("^[0-9]+$")

/** [text] as a whole number of ASCII digits (surrounding space ignored), or null; too many digits is null too. */
private fun wholeNumberOrNull(text: String): Long? = text.trim().takeIf { wholeNumber.matches(it) }?.toLongOrNull()

/**
 * The "Expire after" fields as `maxAge` seconds: [number] × [unit], from 1 minute to 365 days.
 * "Never" sends null whatever [number] holds.
 */
fun inviteExpirySeconds(number: String, unit: InviteExpiryUnit): InviteOption {
    val perUnit = unit.seconds ?: return InviteOption.Valid(null)
    val seconds = wholeNumberOrNull(number)?.takeIf { it <= MAX_INVITE_MAX_AGE }?.let { it * perUnit }
    return if (seconds != null && seconds in MIN_INVITE_MAX_AGE..MAX_INVITE_MAX_AGE) InviteOption.Valid(seconds.toInt())
    else InviteOption.Invalid(INVALID_INVITE_EXPIRY)
}

/** The "Max uses" field as `maxUses`: empty has no limit (null), otherwise a whole number from 1 to 10,000. */
fun inviteMaxUses(text: String): InviteOption {
    if (text.isBlank()) return InviteOption.Valid(null)
    val uses = wholeNumberOrNull(text)
    return if (uses != null && uses in 1..MAX_INVITE_MAX_USES) InviteOption.Valid(uses.toInt())
    else InviteOption.Invalid(INVALID_INVITE_MAX_USES)
}

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
