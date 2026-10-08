package chat.caper.android.data

import chat.caper.android.model.NotificationOverride
import chat.caper.android.model.NotificationSettings
import java.time.Instant
import java.time.ZoneId
import java.util.Locale
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class NotificationsTest {
    private val json = Json { ignoreUnknownKeys = true }
    private val now = Instant.parse("2026-10-08T03:00:00Z")
    private val utc = ZoneId.of("UTC")
    private fun decode(text: String) = json.decodeFromString<NotificationSettings>(text).normalized()

    @Test fun `settings decode with every scope`() {
        val settings = decode("""{"level":"all","mobile":"whenInactive","overrides":[
            {"spaceId":"space0000001","level":"mentions","mutedUntil":null},
            {"spaceId":"space0000001","channelId":"channel00001","level":null,"mutedUntil":"2026-10-08T05:00:00Z"},
            {"conversationId":"direct000001","level":"nothing","mutedUntil":"forever"}]}""")
        assertEquals(LEVEL_ALL, settings.level)
        assertEquals(MOBILE_WHEN_INACTIVE, settings.mobile)
        assertEquals(listOf("space:space0000001", "channel:channel00001", "dm:direct000001"), settings.overrides.map { it.key })
        assertEquals(LEVEL_MENTIONS, settings.override(spaceKey("space0000001"))?.level)
        assertEquals(LEVEL_NOTHING, settings.override(directKey("direct000001"))?.level)
        assertFalse(settings.spaceMuted("space0000001", now))
        assertTrue(settings.channelMuted("space0000001", "channel00001", now))
        assertFalse(settings.channelMuted("space0000001", "channel00002", now))
        assertTrue(settings.directMuted("direct000001", now))
        assertEquals(Instant.parse("2026-10-08T05:00:00Z"), settings.nextMuteEnd(now))
        assertNull(settings.nextMuteEnd(Instant.parse("2026-10-08T05:00:00Z")))
    }

    @Test fun `defaults, future values and scopeless entries`() {
        assertEquals(NotificationSettings(), decode("{}"))
        val future = decode("""{"level":"priority","mobile":"never","overrides":[{"spaceId":"space0000001","level":"quiet"},{"level":"all"}]}""")
        assertEquals(LEVEL_MENTIONS, future.level)
        assertEquals(MOBILE_WHEN_INACTIVE, future.mobile)
        assertEquals(LEVEL_MENTIONS, future.overrides.single().level)
        assertEquals(MOBILE_ALWAYS, decode("""{"mobile":"always"}""").mobile)
    }

    @Test fun `a muted space mutes its channels and default names what is inherited`() {
        val settings = NotificationSettings(level = LEVEL_MENTIONS, overrides = listOf(NotificationOverride(spaceId = "space0000001", level = LEVEL_NOTHING, mutedUntil = "forever")))
        assertTrue(settings.channelMuted("space0000001", "channel00001", now))
        assertFalse(settings.channelMuted("space0000002", "channel00002", now))
        assertEquals(LEVEL_MENTIONS, settings.inheritedLevel())
        assertEquals(LEVEL_NOTHING, settings.inheritedLevel("space0000001"))
        assertEquals(LEVEL_MENTIONS, settings.inheritedLevel("space0000002"))
        assertEquals("Default (Only @mentions)", defaultLevelLabel(settings.inheritedLevel()))
        assertEquals("Default (Nothing)", defaultLevelLabel(settings.inheritedLevel("space0000001")))
        assertEquals("Default (All messages)", defaultLevelLabel(LEVEL_ALL))
        assertEquals(listOf("All messages", "Only @mentions and DMs", "Nothing"), accountLevelOptions.map { it.second })
        assertEquals(listOf("When I'm not active elsewhere", "Always"), mobileOptions.map { it.second })
    }

    @Test fun `expired and unreadable mutes are not mutes`() {
        assertTrue(muteActive("forever", now))
        assertTrue(muteActive("2026-10-08T03:00:01Z", now))
        assertTrue(muteActive("2026-10-08T05:00:00+02:00", now.minusSeconds(1)))
        assertFalse(muteActive("2026-10-08T03:00:00Z", now))
        assertFalse(muteActive("2026-10-07T05:00:00Z", now))
        assertFalse(muteActive("soon", now))
        assertFalse(muteActive(null, now))
    }

    @Test fun `mute labels show the local time, adding the date when it isn't today`() {
        assertEquals("Muted until 5:00 PM", muteLabel("2026-10-08T17:00:00Z", now, utc, Locale.US))
        // 03:00 UTC is still the 7th in New York, so the 8th needs its date there.
        assertEquals("Muted until Oct 8, 1:00 PM", muteLabel("2026-10-08T17:00:00Z", now, ZoneId.of("America/New_York"), Locale.US))
        assertEquals("Muted until Oct 9, 5:00 AM", muteLabel("2026-10-09T05:00:00Z", now, utc, Locale.US))
        assertEquals("Muted until Jan 2, 2027, 9:30 AM", muteLabel("2027-01-02T09:30:00Z", now, utc, Locale.US))
        assertEquals("Muted", muteLabel("forever", now, utc, Locale.US))
        assertNull(muteLabel("2026-10-08T02:00:00Z", now, utc, Locale.US))
        assertNull(muteLabel(null, now, utc, Locale.US))
    }

    @Test fun `mute presets send whole UTC seconds or forever`() {
        val at = Instant.parse("2026-10-08T04:59:30.700Z")
        assertEquals(listOf("For 15 minutes", "For 1 hour", "For 8 hours", "For 24 hours", "Until I turn it back on"), MutePreset.entries.map { it.label })
        assertEquals("2026-10-08T05:14:30Z", MutePreset.Minutes15.mutedUntil(at))
        assertEquals("2026-10-08T05:59:30Z", MutePreset.Hour1.mutedUntil(at))
        assertEquals("2026-10-08T12:59:30Z", MutePreset.Hours8.mutedUntil(at))
        assertEquals("2026-10-09T04:59:30Z", MutePreset.Hours24.mutedUntil(at))
        assertEquals("forever", MutePreset.Forever.mutedUntil(at))
    }

    @Test fun `a change sends only its own key and null resets it`() {
        assertEquals("""{"level":null}""", OverrideChange.Level(null).body().toString())
        assertEquals("""{"level":"nothing"}""", OverrideChange.Level(LEVEL_NOTHING).body().toString())
        assertEquals("""{"mutedUntil":"forever"}""", OverrideChange.Mute("forever").body().toString())
        assertEquals("""{"mutedUntil":null}""", OverrideChange.Mute(null).body().toString())
        val base = NotificationOverride(spaceId = "space0000001", channelId = "channel00001", level = LEVEL_ALL)
        assertEquals(base.copy(mutedUntil = "forever"), base.applying(OverrideChange.Mute("forever")))
        assertEquals(base.copy(level = null), base.applying(OverrideChange.Level(null)))
    }

    @Test fun `empty overrides are dropped and others replaced in place`() {
        val space = NotificationOverride(spaceId = "space0000001", level = LEVEL_MENTIONS)
        val direct = NotificationOverride(conversationId = "direct000001", mutedUntil = "forever")
        val settings = NotificationSettings(overrides = listOf(space, direct))
        assertEquals(listOf(space.copy(level = LEVEL_ALL), direct), settings.withOverride(space.key, space.copy(level = LEVEL_ALL)).overrides)
        assertEquals(listOf(direct), settings.withOverride(space.key, space.copy(level = null)).overrides)
        assertEquals(listOf(space), settings.withOverride(direct.key, null).overrides)
        val channel = NotificationOverride(spaceId = "space0000001", channelId = "channel00001", level = LEVEL_NOTHING)
        assertEquals(listOf(space, direct, channel), settings.withOverride(channel.key, channel).overrides)
        assertEquals(settings, settings.withOverride(channel.key, channel.copy(level = null)))
    }

    @Test fun `a failed edit reverts only its own setting`() {
        val space = NotificationOverride(spaceId = "space0000001", level = LEVEL_MENTIONS)
        val edits = NotificationEdits(NotificationSettings(overrides = listOf(space)))
        val mute = edits.begin(space.key) { it.withOverride(space.key, space.copy(mutedUntil = "forever")) }
        val level = edits.begin(ACCOUNT_LEVEL_KEY) { it.copy(level = LEVEL_NOTHING) }
        assertEquals("forever", edits.shown.override(space.key)?.mutedUntil)
        assertEquals(LEVEL_NOTHING, edits.shown.level)

        edits.failed(space.key, mute)
        assertEquals(space, edits.shown.override(space.key))
        assertEquals(LEVEL_NOTHING, edits.shown.level)

        edits.succeeded(ACCOUNT_LEVEL_KEY, level) { it.copy(level = LEVEL_NOTHING) }
        assertEquals(LEVEL_NOTHING, edits.shown.level)
        assertEquals(LEVEL_NOTHING, edits.confirmed.level)
    }

    @Test fun `a newer edit of the same setting outlives an older failure`() {
        val edits = NotificationEdits(NotificationSettings())
        val first = edits.begin(MOBILE_KEY) { it.copy(mobile = MOBILE_ALWAYS) }
        edits.begin(MOBILE_KEY) { it.copy(mobile = MOBILE_WHEN_INACTIVE) }
        val third = edits.begin(MOBILE_KEY) { it.copy(mobile = MOBILE_ALWAYS) }
        edits.failed(MOBILE_KEY, first)
        assertEquals(MOBILE_ALWAYS, edits.shown.mobile)
        edits.failed(MOBILE_KEY, third)
        assertEquals(MOBILE_WHEN_INACTIVE, edits.shown.mobile)
    }

    @Test fun `the saved answer wins and a reload keeps settings still saving`() {
        val key = directKey("direct000001")
        val edits = NotificationEdits(NotificationSettings())
        val ticket = edits.begin(key) { it.withOverride(key, NotificationOverride(conversationId = "direct000001", mutedUntil = "2026-10-08T05:00:00.250Z")) }
        edits.loaded(NotificationSettings(level = LEVEL_MENTIONS))
        assertEquals(LEVEL_MENTIONS, edits.shown.level)
        assertEquals("2026-10-08T05:00:00.250Z", edits.shown.override(key)?.mutedUntil)
        val saved = NotificationOverride(conversationId = "direct000001", mutedUntil = "2026-10-08T05:00:00Z")
        edits.succeeded(key, ticket) { it.withOverride(key, saved) }
        assertEquals(saved, edits.shown.override(key))
        edits.loaded(NotificationSettings())
        assertNull(edits.shown.override(key))
    }
}
