package chat.caper.android.data

import chat.caper.android.model.NotificationSettings
import chat.caper.android.model.ProfileEvent
import java.time.Instant
import java.time.ZoneId
import java.util.Locale
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.*
import org.junit.Test

class DoNotDisturbTest {
    private val json = Json { ignoreUnknownKeys = true }
    private val newYork = ZoneId.of("America/New_York")
    // Tuesday 2026-11-24, 10:00 in New York.
    private val now = Instant.parse("2026-11-24T15:00:00Z")
    private fun until(preset: DndPreset, at: String) = preset.pausedUntil(Instant.parse(at), newYork)
    private fun label(pausedUntil: String?, at: Instant = now, hour24: Boolean = false) =
        dndEndLabel(pausedUntil, at, newYork, Locale.US, hour24)

    @Test fun `presets end after their duration in whole UTC seconds, or never`() {
        assertEquals("2026-11-24T15:30:00Z", DndPreset.Minutes30.pausedUntil(now, newYork))
        assertEquals("2026-11-24T16:00:00Z", DndPreset.Hour1.pausedUntil(now, newYork))
        assertEquals("2026-11-24T23:00:00Z", DndPreset.Hours8.pausedUntil(now, newYork))
        assertEquals("forever", DndPreset.Forever.pausedUntil(now, newYork))
        assertEquals("2026-11-24T15:30:00Z", until(DndPreset.Minutes30, "2026-11-24T15:00:00.750Z"))
        assertEquals(
            listOf("30 minutes", "1 hour", "8 hours", "Until tomorrow morning", "Until I turn it off"),
            DndPreset.entries.map { it.label },
        )
    }

    @Test fun `tomorrow morning is 8 AM on the next day, or today between midnight and 4-59`() {
        // 10:00 → tomorrow 08:00 EST.
        assertEquals("2026-11-25T13:00:00Z", DndPreset.TomorrowMorning.pausedUntil(now, newYork))
        // 23:59 the night before → the next calendar day, which is "tomorrow" still.
        assertEquals("2026-11-25T13:00:00Z", until(DndPreset.TomorrowMorning, "2026-11-25T04:59:00Z"))
        // 00:00 and 04:59:59 → 08:00 today.
        assertEquals("2026-11-25T13:00:00Z", until(DndPreset.TomorrowMorning, "2026-11-25T05:00:00Z"))
        assertEquals("2026-11-25T13:00:00Z", until(DndPreset.TomorrowMorning, "2026-11-25T09:59:59Z"))
        // 05:00 → 08:00 the next day.
        assertEquals("2026-11-26T13:00:00Z", until(DndPreset.TomorrowMorning, "2026-11-25T10:00:00Z"))
        // Across a month end, and into daylight saving time (08:00 EDT is 12:00 UTC).
        assertEquals("2026-12-01T13:00:00Z", until(DndPreset.TomorrowMorning, "2026-12-01T03:00:00Z"))
        assertEquals("2026-03-08T12:00:00Z", until(DndPreset.TomorrowMorning, "2026-03-08T03:00:00Z"))
    }

    @Test fun `end labels name today, tomorrow, a weekday or forever`() {
        assertEquals("Until 3:45 PM", label("2026-11-24T20:45:00Z"))
        assertEquals("Until 15:45", label("2026-11-24T20:45:00Z", hour24 = true))
        assertEquals("Until tomorrow at 8:00 AM", label("2026-11-25T13:00:00Z"))
        assertEquals("Until Thu 8:00 AM", label("2026-11-26T13:00:00Z"))
        assertEquals("Until you turn it off", label("forever"))
        // Past local midnight is tomorrow, even an hour away.
        assertEquals("Until tomorrow at 12:30 AM", label("2026-11-25T05:30:00Z", at = Instant.parse("2026-11-25T04:30:00Z")))
        assertNull(label(null))
        assertNull(label("2026-11-24T15:00:00Z"))
        assertNull(label("not a time"))
    }

    @Test fun `pausedUntil decodes as off, forever or a time, and expired pauses read as off`() {
        fun decode(text: String) = json.decodeFromString<NotificationSettings>(text).normalized(now)
        assertNull(decode("{}").pausedUntil)
        assertNull(decode("""{"pausedUntil":null}""").pausedUntil)
        assertEquals("forever", decode("""{"pausedUntil":"forever"}""").pausedUntil)
        assertEquals("2026-11-24T21:30:00Z", decode("""{"pausedUntil":"2026-11-24T21:30:00Z"}""").pausedUntil)
        assertNull(decode("""{"pausedUntil":"2026-11-24T14:59:59Z"}""").pausedUntil)
        assertNull(decode("""{"pausedUntil":"soon"}""").pausedUntil)
        assertTrue(dndActive("forever", now))
        assertTrue(dndActive("2026-11-24T15:00:01Z", now))
        assertFalse(dndActive("2026-11-24T15:00:00Z", now))
        assertFalse(dndActive(null, now))
        assertEquals(Instant.parse("2026-11-24T21:30:00Z"), dndEnd("2026-11-24T21:30:00Z"))
        assertNull(dndEnd("forever"))
    }

    @Test fun `a failed Do Not Disturb save reverts only itself`() {
        val edits = NotificationEdits(NotificationSettings(level = LEVEL_MENTIONS))
        val ticket = edits.begin(PAUSE_KEY) { it.copy(pausedUntil = "forever") }
        assertEquals("forever", edits.shown.pausedUntil)
        // A refresh mid-save keeps the value being saved.
        edits.loaded(NotificationSettings(level = LEVEL_ALL))
        assertEquals("forever", edits.shown.pausedUntil)
        assertEquals(LEVEL_ALL, edits.shown.level)
        edits.failed(PAUSE_KEY, ticket)
        assertNull(edits.shown.pausedUntil)
        assertEquals(LEVEL_ALL, edits.shown.level)
    }

    @Test fun `the profiles feed carries notifications updated`() {
        fun event(text: String) = Json.parseToJsonElement(text).jsonObject
        val on = profileEvent(event("""{"type":"notifications.updated","settings":{"level":"mentions","mobile":"always","voiceStarted":false,"pausedUntil":"forever"}}"""))
        assertEquals(
            ProfileEvent.NotificationsChanged(NotificationSettings(level = LEVEL_MENTIONS, mobile = MOBILE_ALWAYS, voiceStarted = false, pausedUntil = "forever")),
            on,
        )
        val off = profileEvent(event("""{"type":"notifications.updated","settings":{"level":"all","mobile":"whenInactive","voiceStarted":true,"pausedUntil":null}}"""))
        assertNull((off as ProfileEvent.NotificationsChanged).settings.pausedUntil)
        // An expired time reads as off; overrides never ride along.
        val expired = profileEvent(event("""{"type":"notifications.updated","settings":{"pausedUntil":"2020-01-01T00:00:00Z","overrides":[{"spaceId":"space0000001","level":"nothing"}]}}"""))
        assertEquals(NotificationSettings(), (expired as ProfileEvent.NotificationsChanged).settings)
        assertNull(profileEvent(event("""{"type":"notifications.updated"}""")))
        assertNull(profileEvent(event("""{"type":"notifications.updated","settings":"forever"}""")))
    }

    @Test fun `Do Not Disturb saves send only pausedUntil`() = runTest {
        val server = MockWebServer()
        try {
            server.enqueue(MockResponse().setBody("""{"level":"all","mobile":"whenInactive","voiceStarted":true,"pausedUntil":"forever"}"""))
            server.enqueue(MockResponse().setBody("""{"level":"all","mobile":"whenInactive","voiceStarted":true,"pausedUntil":"2099-01-01T08:00:00Z"}"""))
            server.enqueue(MockResponse().setBody("""{"level":"all","mobile":"whenInactive","voiceStarted":true,"pausedUntil":null}"""))
            val api = CaperApi(baseUrl = server.url("/").toString())
            assertEquals("forever", api.setDoNotDisturb("account-secret", "forever").pausedUntil)
            assertEquals("2099-01-01T08:00:00Z", api.setDoNotDisturb("account-secret", "2099-01-01T08:00:00Z").pausedUntil)
            assertNull(api.setDoNotDisturb("account-secret", null).pausedUntil)
            val requests = List(3) { server.takeRequest() }
            assertTrue(requests.all { it.method == "PUT" && it.path == "/api/notifications/settings" && it.headers["Authorization"] == "Bearer account-secret" })
            assertEquals(
                listOf("""{"pausedUntil":"forever"}""", """{"pausedUntil":"2099-01-01T08:00:00Z"}""", """{"pausedUntil":null}"""),
                requests.map { it.body.readUtf8() },
            )
        } finally { server.shutdown() }
    }
}
