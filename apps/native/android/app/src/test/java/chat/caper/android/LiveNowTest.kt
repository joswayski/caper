package chat.caper.android

import chat.caper.android.model.AppUiState
import chat.caper.android.model.Channel
import chat.caper.android.model.Participant
import chat.caper.android.model.Space
import chat.caper.android.model.SpaceDetail
import chat.caper.android.model.VisitUi
import chat.caper.android.voice.VoiceState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LiveNowTest {
    private fun person(id: String, name: String = id) = Participant(id, name, muted = false, deafened = false, tracks = emptyList())
    private fun channel(id: String, name: String = id, joined: Boolean = true) = Channel(id, "space", name, private = false, joined = joined)

    @Test fun `room names list up to three people, then two and how many more`() {
        assertEquals("Maya", liveRoomNames(listOf("Maya")))
        assertEquals("Maya and Alex", liveRoomNames(listOf("Maya", "Alex")))
        assertEquals("Maya, Alex and Sam", liveRoomNames(listOf("Maya", "Alex", "Sam")))
        assertEquals("Maya, Alex and 2 more", liveRoomNames(listOf("Maya", "Alex", "Sam", "Jo")))
        assertEquals("Maya, Alex and 5 more", liveRoomNames(listOf("Maya", "Alex", "Sam", "Jo", "Lee", "Kim", "Ana")))
    }

    @Test fun `your call's faces put you first and otherwise keep join order`() {
        val people = listOf(person("a"), person("me"), person("b"), person("c"))
        assertEquals(listOf("me", "a", "b", "c"), callFaceOrder(people, "me").map { it.id })
        // Not in the roster yet, or no self ID: the roster's own order.
        assertEquals(listOf("a", "me", "b", "c"), callFaceOrder(people, "other").map { it.id })
        assertEquals(listOf("a", "me", "b", "c"), callFaceOrder(people, null).map { it.id })
    }

    @Test fun `up to ten faces show, beyond that nine and a +N tile, fewer when two rows hold less`() {
        // Two rows of five or more: the spec's ten / nine + N.
        assertEquals(10, callFacesShown(10, 10))
        assertEquals(9, callFacesShown(11, 10))
        assertEquals(9, callFacesShown(40, 12))
        assertEquals(3, callFacesShown(3, 12))
        // A narrow card holding four a row keeps to two rows: seven faces and "+N".
        assertEquals(8, callFacesShown(8, 8))
        assertEquals(7, callFacesShown(10, 8))
    }

    @Test fun `live rooms leave out your call and quiet channels, busiest first then by name`() {
        val detail = SpaceDetail(
            Space("space", "Studio"),
            listOf(channel("general"), channel("design"), channel("planning"), channel("random"), channel("lobby", joined = false)),
            members = emptyList(),
        )
        val rosters = mapOf(
            "design" to listOf(person("a"), person("b")),
            "random" to listOf(person("c"), person("d")),
            "planning" to listOf(person("e"), person("f"), person("g")),
            "general" to listOf(person("h")),
            "lobby" to listOf(person("i")),
        )
        assertEquals(listOf("planning", "design", "random", "general"), liveRooms(detail, rosters, null).map { it.channel.id })
        assertEquals(listOf("design", "random", "general"), liveRooms(detail, rosters, "planning").map { it.channel.id })
        assertEquals(emptyList<LiveRoom>(), liveRooms(detail, emptyMap(), null))
    }

    @Test fun `a feedback visit's live space channel joins the open space's rooms`() {
        val detail = SpaceDetail(Space("space", "Studio"), listOf(channel("design")), members = emptyList())
        val visited = Channel("report", "other", "support", private = false)
        val visit = VisitUi(channelId = "report", spaceId = "other", spaceName = "Other", channelName = "support", direct = false)
        val rosters = mapOf("design" to listOf(person("a")), "report" to listOf(person("b"), person("c")))
        val state = AppUiState(selectedSpace = detail, selectedChannel = visited, visit = visit, voiceRosters = rosters)
        assertEquals(listOf("report", "design"), liveNowRooms(state, null).map { it.channel.id })
        // Not while it is your call, or when nobody is there; a DM visit has no voice.
        assertEquals(listOf("design"), liveNowRooms(state, "report").map { it.channel.id })
        assertEquals(listOf("design"), liveNowRooms(state.copy(voiceRosters = rosters - "report"), null).map { it.channel.id })
        assertEquals(listOf("design"), liveNowRooms(state.copy(visit = visit.copy(direct = true)), null).map { it.channel.id })
        assertEquals(listOf("design"), liveNowRooms(state.copy(visit = null), null).map { it.channel.id })
    }

    @Test fun `the full list's title counts people`() {
        assertEquals("In #design · 12 people", voiceListTitle("design", 12))
        assertEquals("In #design · 1 person", voiceListTitle("design", 1))
    }

    @Test fun `the full list says speaking first, then your local mute, deafened and muted`() {
        assertEquals(VoiceListStatus.Speaking, voiceListStatus(self = false, speaking = true, locallyMuted = true, deafened = false, muted = false))
        assertEquals(VoiceListStatus.LocallyMuted, voiceListStatus(self = false, speaking = false, locallyMuted = true, deafened = true, muted = true))
        assertEquals(VoiceListStatus.Deafened, voiceListStatus(self = false, speaking = false, locallyMuted = false, deafened = true, muted = true))
        assertEquals(VoiceListStatus.Muted, voiceListStatus(self = false, speaking = false, locallyMuted = false, deafened = false, muted = true))
        // You never mute yourself for yourself.
        assertNull(voiceListStatus(self = true, speaking = false, locallyMuted = true, deafened = false, muted = false))
        assertEquals("You muted Maya", voiceListStatusLabel(VoiceListStatus.LocallyMuted, "Maya"))
        assertEquals("Speaking", voiceListStatusLabel(VoiceListStatus.Speaking, "Maya"))
    }

    @Test fun `join reads Join, Switch from another call, or Joining while this join is under way`() {
        val design = channel("design")
        val state = AppUiState(voiceAvailability = mapOf("design" to true))
        val idle = VoiceState()
        val join = voiceJoinAction(state, idle, null, design)
        assertTrue(join.enabled)
        assertFalse(join.switch)
        assertEquals("Join voice in #design", join.label("design"))

        val elsewhere = VoiceState(phase = VoiceState.Phase.CONNECTED, channelId = "general")
        val switch = voiceJoinAction(state, elsewhere, null, design)
        assertTrue(switch.enabled && switch.switch)
        assertEquals("Switch voice to #design", switch.label("design"))

        val pending = VoiceJoinIntent("design", "space", "design", "Studio", "Me", "me", 0, demo = false, controlEpoch = 1)
        val joining = voiceJoinAction(state, idle, pending, design)
        assertTrue(joining.joining)
        assertFalse(joining.enabled)
        assertEquals("Joining voice in #design", joining.label("design"))

        // Another join waiting for permission holds every other Join.
        val waiting = voiceJoinAction(state, idle, pending.copy(channelId = "general"), design)
        assertFalse(waiting.enabled || waiting.joining)

        val connecting = voiceJoinAction(state, VoiceState(phase = VoiceState.Phase.CONNECTING, channelId = "design"), null, design)
        assertTrue(connecting.joining)
        assertFalse(connecting.enabled)
    }

    @Test fun `unavailable voice says why Join is disabled, denied channels do not`() {
        val design = channel("design")
        val checking = voiceJoinAction(AppUiState(), VoiceState(), null, design)
        assertFalse(checking.enabled)
        assertEquals("Checking voice availability…", checking.label("design"))
        val off = voiceJoinAction(AppUiState(voiceAvailability = mapOf("design" to false)), VoiceState(), null, design)
        assertEquals("Joining is not available at this time.", off.label("design"))
        val denied = voiceJoinAction(AppUiState(voiceAvailability = mapOf("design" to true), deniedVoiceChannels = setOf("design")), VoiceState(), null, design)
        assertFalse(denied.enabled)
        assertEquals("Join voice in #design", denied.label("design"))
    }

    @Test fun `in voice means joining, connected or reconnecting`() {
        assertFalse(inVoice(VoiceState()))
        assertFalse(inVoice(VoiceState(phase = VoiceState.Phase.FAILED)))
        assertTrue(inVoice(VoiceState(phase = VoiceState.Phase.CONNECTING)))
        assertTrue(inVoice(VoiceState(phase = VoiceState.Phase.CONNECTED)))
        assertTrue(inVoice(VoiceState(phase = VoiceState.Phase.RECONNECTING)))
    }

    @Test fun `your call's clock is the channel's while connecting, then the call's own`() {
        val state = AppUiState(voiceSessionStartedAt = mapOf("design" to 1_000L))
        assertEquals(1_000L, callSessionStartedAt(state, VoiceState(phase = VoiceState.Phase.CONNECTING, channelId = "design", sessionStartedAt = 5_000L)))
        assertEquals(5_000L, callSessionStartedAt(state, VoiceState(phase = VoiceState.Phase.CONNECTED, channelId = "design", sessionStartedAt = 5_000L)))
        assertEquals(7_000L, callSessionStartedAt(AppUiState(), VoiceState(phase = VoiceState.Phase.CONNECTING, channelId = "design", sessionStartedAt = 7_000L)))
    }
}
