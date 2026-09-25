package chat.caper.android

import chat.caper.android.voice.JoinTiming
import chat.caper.android.voice.VoiceDiagnostics
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.*
import org.junit.Test

class ConnectionDetailsTest {
    private val stats = VoiceDiagnostics(1_234_567, 31_600, 987_654, 28_400, 3, 12, 41, "relay")

    @Test fun `rows use web labels and formats`() {
        val rows = connectionDetailRows(stats).toMap()
        assertEquals("1.23 MB", rows["Received"])
        assertEquals("32 kbps", rows["Live receive"])
        assertEquals("0.99 MB", rows["Sent"])
        assertEquals("28 kbps", rows["Live send"])
        assertEquals("TURN relay", rows["Route"])
        assertNull("Unmeasured join phases are omitted", rows["Joined"])
        assertEquals("Direct", connectionDetailRows(stats.copy(route = "direct")).toMap()["Route"])
        assertEquals("Not observed yet", connectionDetailRows(stats.copy(route = "unknown")).toMap()["Route"])
    }

    @Test fun `join timing rows appear only with measured phases`() {
        val timed = stats.copy(timing = JoinTiming(joinedMs = 812, sessionMs = 240, transportMs = 180, iceMs = 150, rosterMs = 60))
        val rows = connectionDetailRows(timed)
        assertEquals(listOf("Joined", "Session + publish", "Transport + state", "Connectivity checks", "Roster", "Received"), rows.take(6).map { it.first })
        val values = rows.toMap()
        assertEquals("Joined in 812 ms", values["Joined"])
        assertEquals("180 ms (ICE 150 ms)", values["Transport + state"])
        assertEquals("Not observed yet", values["Connectivity checks"])
        assertEquals("180 ms", connectionDetailRows(timed.copy(timing = timed.timing!!.copy(iceMs = null))).toMap()["Transport + state"])
        assertEquals("3 sent · 3 answered", connectionDetailRows(timed.copy(checks = "3 sent · 3 answered")).toMap()["Connectivity checks"])
        for (unmeasured in listOf("Microphone", "Signaling + live updates", "Hearing others")) assertNull(values[unmeasured])
    }

    @Test fun `copied JSON uses web field names`() {
        val copied = Json.parseToJsonElement(connectionDetailsJson(stats.copy(timing = JoinTiming(812, 240, 180, null, 60)))).jsonObject
        assertEquals("Joined in 812 ms", copied["join"]!!.jsonPrimitive.content)
        assertEquals("240", copied["sessionMs"]!!.jsonPrimitive.content)
        assertEquals("1234567", copied["receivedBytes"]!!.jsonPrimitive.content)
        assertEquals("relay", copied["route"]!!.jsonPrimitive.content)
        assertFalse(copied.containsKey("iceMs"))
        assertFalse(copied.containsKey("checks"))
    }
}
