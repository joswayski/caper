package chat.caper.android.data

import chat.caper.android.model.NotificationOverride
import chat.caper.android.model.NotificationSettings
import java.time.Duration
import java.time.Instant
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit
import java.util.Locale
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

internal const val LEVEL_ALL = "all"
internal const val LEVEL_MENTIONS = "mentions"
internal const val LEVEL_NOTHING = "nothing"
internal const val MOBILE_WHEN_INACTIVE = "whenInactive"
internal const val MOBILE_ALWAYS = "always"
internal const val MUTED_FOREVER = "forever"

/** Setting keys: the account level, the phone choice, and one per override scope. */
internal const val ACCOUNT_LEVEL_KEY = "level"
internal const val MOBILE_KEY = "mobile"
internal const val VOICE_KEY = "voiceStarted"
internal fun spaceKey(spaceId: String) = "space:$spaceId"
internal fun channelKey(channelId: String) = "channel:$channelId"
internal fun directKey(conversationId: String) = "dm:$conversationId"

/** The scope an override applies to: a DM, a channel, or a space. */
internal val NotificationOverride.key: String
    get() = conversationId?.let(::directKey) ?: channelId?.let(::channelKey) ?: spaceKey(spaceId.orEmpty())

internal const val NOTIFICATION_SAVE_ERROR = "Couldn’t save. Try again."

/** "Notify me about", in display order. */
internal val accountLevelOptions = listOf(
    LEVEL_ALL to "All messages",
    LEVEL_MENTIONS to "Only @mentions and DMs",
    LEVEL_NOTHING to "Nothing",
)

/** "Send to this phone", in display order. */
internal val mobileOptions = listOf(
    MOBILE_WHEN_INACTIVE to "When I'm not active elsewhere",
    MOBILE_ALWAYS to "Always",
)

/** Space and channel levels after Default, in display order. */
internal val overrideLevelOptions = listOf(
    LEVEL_ALL to "All messages",
    LEVEL_MENTIONS to "Only @mentions",
    LEVEL_NOTHING to "Nothing",
)

internal fun overrideLevelLabel(level: String): String = overrideLevelOptions.firstOrNull { it.first == level }?.second ?: "Only @mentions"

/** Default names what it inherits, for example `Default (Only @mentions)`. */
internal fun defaultLevelLabel(inherited: String) = "Default (${overrideLevelLabel(inherited)})"

/** A level from a newer server that this client does not know reads as `mentions`. */
internal fun knownLevel(level: String?): String? = when (level) {
    null, LEVEL_ALL, LEVEL_MENTIONS, LEVEL_NOTHING -> level
    else -> LEVEL_MENTIONS
}

/** An expired or unreadable [NotificationSettings.pausedUntil] reads as off (null). */
internal fun NotificationSettings.normalized(now: Instant = Instant.now()) = copy(
    level = knownLevel(level) ?: LEVEL_ALL,
    mobile = if (mobile == MOBILE_ALWAYS) MOBILE_ALWAYS else MOBILE_WHEN_INACTIVE,
    pausedUntil = pausedUntil?.takeIf { dndActive(it, now) },
    overrides = overrides.filter { it.conversationId != null || it.channelId != null || it.spaceId != null }
        .map { it.normalized() }.distinctBy { it.key },
)

internal fun NotificationOverride.normalized() = copy(level = knownLevel(level))

internal fun NotificationSettings.override(key: String): NotificationOverride? = overrides.firstOrNull { it.key == key }

/** What Default means for a channel in [spaceId] (its space's level, else the account's), or for a space (null). */
internal fun NotificationSettings.inheritedLevel(spaceId: String? = null): String =
    spaceId?.let { override(spaceKey(it))?.level } ?: level

internal fun NotificationSettings.spaceMuted(spaceId: String, now: Instant) = muteActive(override(spaceKey(spaceId))?.mutedUntil, now)

/** A channel is muted by its own mute or by its space's. */
internal fun NotificationSettings.channelMuted(spaceId: String, channelId: String, now: Instant) =
    spaceMuted(spaceId, now) || muteActive(override(channelKey(channelId))?.mutedUntil, now)

internal fun NotificationSettings.directMuted(conversationId: String, now: Instant) = muteActive(override(directKey(conversationId))?.mutedUntil, now)

/** The first moment after [now] when a mute ends, so muted rows can brighten again. */
internal fun NotificationSettings.nextMuteEnd(now: Instant): Instant? =
    overrides.mapNotNull { override -> override.mutedUntil?.let(::muteEnd)?.takeIf { it.isAfter(now) } }.minOrNull()

private fun muteEnd(mutedUntil: String): Instant? = runCatching { OffsetDateTime.parse(mutedUntil).toInstant() }.getOrNull()

/** `forever`, or a time still ahead. Expired or unreadable values are not mutes. */
internal fun muteActive(mutedUntil: String?, now: Instant): Boolean = when (mutedUntil) {
    null -> false
    MUTED_FOREVER -> true
    else -> muteEnd(mutedUntil)?.isAfter(now) == true
}

/** `Muted until 5:00 PM` (with the date when it isn't today), `Muted` for forever, or null when not muted. */
internal fun muteLabel(mutedUntil: String?, now: Instant, zone: ZoneId = ZoneId.systemDefault(), locale: Locale = Locale.getDefault()): String? {
    if (!muteActive(mutedUntil, now)) return null
    val end = mutedUntil?.let(::muteEnd)?.atZone(zone) ?: return "Muted"
    val today = now.atZone(zone).toLocalDate()
    val pattern = when {
        end.toLocalDate() == today -> "h:mm a"
        end.year == today.year -> "MMM d, h:mm a"
        else -> "MMM d, yyyy, h:mm a"
    }
    return "Muted until ${DateTimeFormatter.ofPattern(pattern, locale).format(end)}"
}

/** The mute choices, in display order. */
internal enum class MutePreset(val label: String, private val duration: Duration?) {
    Minutes15("For 15 minutes", Duration.ofMinutes(15)),
    Hour1("For 1 hour", Duration.ofHours(1)),
    Hours8("For 8 hours", Duration.ofHours(8)),
    Hours24("For 24 hours", Duration.ofHours(24)),
    Forever("Until I turn it back on", null);

    /** Whole seconds in UTC (`2026-10-08T05:00:00Z`), or `forever`. */
    fun mutedUntil(now: Instant): String = duration?.let { now.plus(it).truncatedTo(ChronoUnit.SECONDS).toString() } ?: MUTED_FOREVER
}

// --- Do Not Disturb ---------------------------------------------------------------------------

/** Do Not Disturb's setting key in [NotificationEdits]. */
internal const val PAUSE_KEY = "pausedUntil"
internal const val PAUSED_FOREVER = "forever"
internal const val DND_SAVE_ERROR = "That change wasn’t saved. Try again."

/** On while [pausedUntil] is `forever` or a time still ahead. Expired or unreadable values are off. */
internal fun dndActive(pausedUntil: String?, now: Instant): Boolean = muteActive(pausedUntil, now)

/** When Do Not Disturb ends by itself: null when off or `forever`. */
internal fun dndEnd(pausedUntil: String?): Instant? = pausedUntil?.takeIf { it != PAUSED_FOREVER }?.let(::muteEnd)

/** The Do Not Disturb choices, identical on every client, in display order. */
internal enum class DndPreset(val label: String) {
    Minutes30("30 minutes"),
    Hour1("1 hour"),
    Hours8("8 hours"),
    TomorrowMorning("Until tomorrow morning"),
    Forever("Until I turn it off");

    /**
     * The `pausedUntil` to send: whole seconds in UTC, or `forever`. Tomorrow morning is 8:00
     * local time on the next calendar day, except between 00:00 and 04:59, when it is 8:00 today.
     */
    fun pausedUntil(now: Instant, zone: ZoneId = ZoneId.systemDefault()): String = when (this) {
        Minutes30 -> utcSeconds(now.plus(Duration.ofMinutes(30)))
        Hour1 -> utcSeconds(now.plus(Duration.ofHours(1)))
        Hours8 -> utcSeconds(now.plus(Duration.ofHours(8)))
        TomorrowMorning -> {
            val local = now.atZone(zone)
            val day = if (local.hour < 5) local.toLocalDate() else local.toLocalDate().plusDays(1)
            utcSeconds(day.atTime(8, 0).atZone(zone).toInstant())
        }
        Forever -> PAUSED_FOREVER
    }
}

private fun utcSeconds(instant: Instant) = instant.truncatedTo(ChronoUnit.SECONDS).toString()

/**
 * `Until 3:45 PM` (today), `Until tomorrow at 8:00 AM`, `Until Thu 8:00 AM` (later), or
 * `Until you turn it off`; null while Do Not Disturb is off. Times follow [hour24] (the
 * system's 12/24-hour setting) like message times.
 */
internal fun dndEndLabel(
    pausedUntil: String?, now: Instant, zone: ZoneId = ZoneId.systemDefault(),
    locale: Locale = Locale.getDefault(), hour24: Boolean = false,
): String? {
    if (!dndActive(pausedUntil, now)) return null
    val end = dndEnd(pausedUntil)?.atZone(zone) ?: return "Until you turn it off"
    val time = DateTimeFormatter.ofPattern(if (hour24) "HH:mm" else "h:mm a", locale).format(end)
    val today = now.atZone(zone).toLocalDate()
    return when (end.toLocalDate()) {
        today -> "Until $time"
        today.plusDays(1) -> "Until tomorrow at $time"
        else -> "Until ${DateTimeFormatter.ofPattern("EEE", locale).format(end)} $time"
    }
}

/** One override change. The API leaves the other field as it is, and null resets this one. */
sealed interface OverrideChange {
    data class VoiceStarted(val enabled: Boolean?) : OverrideChange
    data class Level(val level: String?) : OverrideChange
    data class Mute(val mutedUntil: String?) : OverrideChange
}

internal fun OverrideChange.body(): JsonObject = buildJsonObject {
    when (val change = this@body) {
        is OverrideChange.VoiceStarted -> put("voiceStarted", change.enabled)
        is OverrideChange.Level -> put("level", change.level)
        is OverrideChange.Mute -> put("mutedUntil", change.mutedUntil)
    }
}

internal fun NotificationOverride.applying(change: OverrideChange) = when (change) {
    is OverrideChange.VoiceStarted -> copy(voiceStarted = change.enabled)
    is OverrideChange.Level -> copy(level = change.level)
    is OverrideChange.Mute -> copy(mutedUntil = change.mutedUntil)
}

/** [key]'s override replaced by [value]. Like the server, an override with no level and no mute is dropped. */
internal fun NotificationSettings.withOverride(key: String, value: NotificationOverride?): NotificationSettings {
    val kept = value?.takeIf { it.level != null || it.mutedUntil != null || it.voiceStarted != null }
    val index = overrides.indexOfFirst { it.key == key }
    val next = overrides.toMutableList()
    if (index < 0) { if (kept != null) next.add(kept) }
    else if (kept == null) next.removeAt(index)
    else next[index] = kept
    return copy(overrides = next)
}

/** [key] (the account level, the phone choice or an override) taken from [source]. */
internal fun NotificationSettings.withSettingFrom(key: String, source: NotificationSettings): NotificationSettings = when (key) {
    ACCOUNT_LEVEL_KEY -> copy(level = source.level)
    MOBILE_KEY -> copy(mobile = source.mobile)
    VOICE_KEY -> copy(voiceStarted = source.voiceStarted)
    PAUSE_KEY -> copy(pausedUntil = source.pausedUntil)
    else -> withOverride(key, source.override(key))
}

/**
 * Optimistic notification edits. [shown] applies each edit at once and [confirmed] holds the
 * server's answers. Each setting permits one save at a time; a failed edit restores its
 * confirmed value without disturbing independent settings.
 */
internal class NotificationEdits(initial: NotificationSettings) {
    var confirmed = initial
        private set
    var shown = initial
        private set
    private val latest = mutableMapOf<String, Long>()
    private var tickets = 0L

    val saving: Set<String> get() = latest.keys.toSet()

    /** Rejects a repeated save until this setting settles. */
    fun begin(key: String, edit: (NotificationSettings) -> NotificationSettings): Long? {
        if (key in latest) return null
        shown = edit(shown)
        return (++tickets).also { latest[key] = it }
    }

    /** The server saved the edit; [answer] writes its value for [key]. */
    fun succeeded(key: String, ticket: Long?, answer: (NotificationSettings) -> NotificationSettings) {
        if (ticket == null) return
        if (latest[key] != ticket) return
        confirmed = answer(confirmed)
        settle(key, ticket)
    }

    fun failed(key: String, ticket: Long?) { if (ticket != null) settle(key, ticket) }

    /** A fresh load. Settings still saving keep the value being saved. */
    fun loaded(settings: NotificationSettings) {
        confirmed = settings
        shown = latest.keys.fold(settings) { result, key -> result.withSettingFrom(key, shown) }
    }

    private fun settle(key: String, ticket: Long) {
        if (latest[key] != ticket) return
        latest.remove(key)
        shown = shown.withSettingFrom(key, confirmed)
    }
}
