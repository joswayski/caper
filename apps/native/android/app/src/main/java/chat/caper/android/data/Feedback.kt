package chat.caper.android.data

import chat.caper.android.model.ChatContent
import chat.caper.android.model.FeedbackReport
import chat.caper.android.model.FeedbackReports
import chat.caper.android.model.VisitUi
import java.time.Duration
import java.time.Instant
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

// `@feedback` (or `@caper`) asks the Caper team to look at a conversation. The message stays
// where it was written; feedback team members open that conversation without joining it
// (a "visit") and reply in place, with an ADMIN badge.

/** The gold badge after a feedback team member's name (`author.admin`). */
internal const val ADMIN_BADGE = "ADMIN"

/** The small muted line under a message that asked the Caper team to look. */
internal const val CAPER_TEAM_NOTIFIED = "Caper team has been notified 🫡"

/** The server reported this message to the Caper team (`content.mentions` has a `feedback` entry). */
internal fun notifiesCaperTeam(content: ChatContent): Boolean = content.mentions.any { it.type == "feedback" }

/** Where a report was written: "#general · Studio", or "DM · Alice & Bob" for a DM or personal notes. */
internal fun feedbackLocation(report: FeedbackReport): String =
    if (report.channel.direct || report.space == null) "DM · ${report.channel.name}"
    else "#${report.channel.name} · ${report.space.name}"

/** The report's text, or "Sent N files" for a file-only message. */
internal fun feedbackPreview(report: FeedbackReport): String = when {
    report.text.isNotBlank() -> report.text.trim()
    report.attachments == 1 -> "Sent 1 file"
    report.attachments > 1 -> "Sent ${report.attachments} files"
    else -> ""
}

/** The report's quiet status: "Answered" once the team replied in its thread, else "Waiting". */
internal fun feedbackStatus(report: FeedbackReport): String = if (report.answered) "Answered" else "Waiting"

/** The empty Caper DM's line, in place of "Only you and … can read this conversation." */
internal const val CAPER_DM_EMPTY_NOTE = "Messages here go to the Caper team. They’ll reply in a thread on each one."

/** RFC 3339 with `Z` or an offset (Postgres sends `+00:00`); null when unreadable. */
internal fun parseTimestamp(value: String): Instant? =
    runCatching { OffsetDateTime.parse(value).toInstant() }.getOrNull() ?: runCatching { Instant.parse(value) }.getOrNull()

/** A compact relative time: "now", "5m", "3h", "2d", then the date ("Oct 4", or "Oct 4, 2025" in another year). */
internal fun feedbackAge(createdAt: String, now: Instant = Instant.now(), zone: ZoneId = ZoneId.systemDefault(), locale: Locale = Locale.getDefault()): String {
    val time = parseTimestamp(createdAt) ?: return ""
    val elapsed = Duration.between(time, now)
    return when {
        elapsed.toMinutes() < 1 -> "now"
        elapsed.toHours() < 1 -> "${elapsed.toMinutes()}m"
        elapsed.toDays() < 1 -> "${elapsed.toHours()}h"
        elapsed.toDays() < 7 -> "${elapsed.toDays()}d"
        else -> {
            val date = time.atZone(zone)
            val pattern = if (date.year == now.atZone(zone).year) "MMM d" else "MMM d, yyyy"
            DateTimeFormatter.ofPattern(pattern, locale).format(date)
        }
    }
}

/** A visit's header: "#plans · Studio" for a channel, "Alice & Bob" for a DM. */
internal fun visitTitle(visit: VisitUi): String = when {
    visit.direct -> visit.channelName
    visit.spaceName.isNullOrEmpty() -> "#${visit.channelName}"
    else -> "#${visit.channelName} · ${visit.spaceName}"
}

/**
 * What to open for a report or a `feedback` notification: the conversation [channelId] (a DM
 * when [spaceId] is null) around [messageId], with the thread [threadRootId] open.
 */
internal data class FeedbackTarget(
    val channelId: String,
    val spaceId: String?,
    val messageId: String,
    val threadRootId: String?,
    val channelName: String? = null,
    val spaceName: String? = null,
    val reporter: chat.caper.android.model.FeedbackReporter? = null,
) {
    /** The thread to open: the message's root, or the message itself when it is top-level. */
    val thread: String get() = threadRootId ?: messageId

    /** The root to load the message's context in, null for a top-level message (its own thread). */
    val replyRoot: String? get() = threadRootId?.takeIf { it != messageId }
}

private val externalIdPattern = Regex("^[A-Za-z0-9]{12}$")
private val messageIdPattern = Regex("^[A-Za-z0-9]{15}$")

internal fun feedbackTarget(report: FeedbackReport): FeedbackTarget? {
    if (!externalIdPattern.matches(report.channel.id) || !messageIdPattern.matches(report.messageId) ||
        !messageIdPattern.matches(report.threadRootId)) return null
    val space = report.space?.takeIf { !report.channel.direct && externalIdPattern.matches(it.id) }
    return FeedbackTarget(report.channel.id, space?.id, report.messageId, report.threadRootId,
        report.channel.name, space?.name, report.reporter)
}

/**
 * A notification tap that opens the visit view: kind `feedback` with a well-formed message and
 * conversation (a DM's [conversationId], or a channel's [spaceId] and [channelId]). Other
 * kinds, and malformed taps, open the conversation the usual way (null).
 */
internal fun feedbackTargetFromNotification(
    kind: String?, conversationId: String?, spaceId: String?, channelId: String?, messageId: String?, threadRootId: String?,
): FeedbackTarget? {
    if (kind != "feedback" || messageId == null || !messageIdPattern.matches(messageId)) return null
    val root = threadRootId?.takeIf { messageIdPattern.matches(it) }
    if (threadRootId != null && root == null) return null
    return when {
        conversationId != null -> if (externalIdPattern.matches(conversationId) && spaceId == null && channelId == null)
            FeedbackTarget(conversationId, null, messageId, root) else null
        spaceId != null && channelId != null && externalIdPattern.matches(spaceId) && externalIdPattern.matches(channelId) ->
            FeedbackTarget(channelId, spaceId, messageId, root)
        else -> null
    }
}

/** The list without [report]'s thread (archive and restore move whole threads). */
internal fun withoutThread(reports: List<FeedbackReport>, report: FeedbackReport): List<FeedbackReport> =
    reports.filterNot { it.channel.id == report.channel.id && it.threadRootId == report.threadRootId }

/** A row's archive action, as read aloud: archive from the inbox, restore from the archive. */
internal fun archiveActionLabel(report: FeedbackReport, archived: Boolean): String =
    if (archived) "Restore feedback from ${report.reporter.name} to the inbox" else "Archive feedback from ${report.reporter.name}"

/** Appends a next page (or replaces the list for the first page), keeping one row per report. */
internal fun mergeFeedbackPage(current: List<FeedbackReport>, page: FeedbackReports, first: Boolean): List<FeedbackReport> =
    (if (first) page.reports else current + page.reports).distinctBy { it.id }

/** The inbox cursor is the server's decimal report ID. */
internal fun validFeedbackCursor(cursor: String?): String? = cursor?.takeIf { Regex("^[1-9][0-9]{0,18}$").matches(it) }
