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

    @Test fun `the strip shows your call card's faces up to five, then the first four and +N`() {
        // Up to five the strip and the card (ten, or fewer when two rows hold less) show the same people.
        for (total in 0..5) {
            assertEquals(total, stripFacesShown(total))
            assertEquals(total, callFacesShown(total, CALL_FACES_MAX))
            assertEquals(total, callFacesShown(total, 8))
        }
        assertEquals(4, stripFacesShown(6))
        assertEquals(4, stripFacesShown(12))
    }

    @Test fun `your call's row is a joined channel of the listed space, otherwise its card goes above the channels`() {
        val detail = SpaceDetail(Space("space", "Studio"), listOf(channel("general"), channel("design"), channel("lobby", joined = false)), members = emptyList())
        val call = VoiceState(phase = VoiceState.Phase.CONNECTED, channelId = "design", spaceId = "space")
        assertEquals("design", callRowChannelId(detail, call))
        assertEquals("design", callRowChannelId(detail, call.copy(phase = VoiceState.Phase.CONNECTING)))
        assertEquals("design", callRowChannelId(detail, call.copy(phase = VoiceState.Phase.RECONNECTING)))
        // No call, no card.
        assertNull(callRowChannelId(detail, call.copy(phase = VoiceState.Phase.IDLE)))
        assertNull(callRowChannelId(detail, call.copy(phase = VoiceState.Phase.FAILED)))
        // Another space's call, a channel with no row here (a feedback visit) and a channel you left.
        assertNull(callRowChannelId(detail, call.copy(spaceId = "other")))
        assertNull(callRowChannelId(detail, call.copy(channelId = "report")))
        assertNull(callRowChannelId(detail, call.copy(channelId = "lobby")))
        assertNull(callRowChannelId(null, call))
    }

    @Test fun `rows show your call, other live rooms or nothing`() {
        val rosters = mapOf("design" to listOf(person("a")), "planning" to listOf(person("b")), "lobby" to listOf(person("c")))
        assertEquals(RowVoice.CALL, rowVoice(channel("design"), demo = false, rosters, callRowId = "design"))
        // Your call's row is its card from the moment you join, before anyone shows in the roster.
        assertEquals(RowVoice.CALL, rowVoice(channel("general"), demo = false, emptyMap(), callRowId = "general"))
        assertEquals(RowVoice.ROOM, rowVoice(channel("planning"), demo = false, rosters, callRowId = "design"))
        assertEquals(RowVoice.QUIET, rowVoice(channel("general"), demo = false, rosters, callRowId = "design"))
        assertEquals(RowVoice.QUIET, rowVoice(channel("lobby", joined = false), demo = false, rosters, callRowId = null))
        // The demo keys its one room by the demo root.
        assertEquals(RowVoice.ROOM, rowVoice(channel("general"), demo = true, mapOf("" to listOf(person("a"))), callRowId = null))
    }

    @Test fun `collapsing or searching hides quiet rows only, and live rows keep the list's order`() {
        val channels = listOf("general", "design", "planning", "random", "lobby").map { channel(it) }
        val kinds = mapOf("design" to RowVoice.ROOM, "planning" to RowVoice.ROOM, "random" to RowVoice.CALL)
        val voiceOf = { channel: Channel -> kinds[channel.id] ?: RowVoice.QUIET }
        fun listed(shown: (Channel) -> Boolean) = listedChannels(channels, shown, voiceOf).map { it.id }
        assertEquals(listOf("general", "design", "planning", "random", "lobby"), listed { true })
        // Collapsed: your call's card and the live rooms stay, in list order rather than busiest first.
        assertEquals(listOf("design", "planning", "random"), listed { false })
        // A search keeps its matches and every row with voice.
        assertEquals(listOf("design", "planning", "random", "lobby"), listed { it.name == "lobby" })
    }

    @Test fun `a feedback visit's live space channel has no row, so its card goes above the channels`() {
        val detail = SpaceDetail(Space("space", "Studio"), listOf(channel("design")), members = emptyList())
        val visited = Channel("report", "other", "support", private = false)
        val visit = VisitUi(channelId = "report", spaceId = "other", spaceName = "Other", channelName = "support", direct = false)
        val rosters = mapOf("design" to listOf(person("a")), "report" to listOf(person("b"), person("c")))
        val state = AppUiState(selectedSpace = detail, selectedChannel = visited, visit = visit, voiceRosters = rosters)
        assertEquals("report", visitedLiveRoom(state, null)?.channel?.id)
        assertEquals(listOf("b", "c"), visitedLiveRoom(state, null)?.people?.map { it.id })
        // Not while it is your call (your call card goes there instead), or when nobody is there; a DM visit has no voice.
        assertNull(visitedLiveRoom(state, "report"))
        assertNull(visitedLiveRoom(state.copy(voiceRosters = rosters - "report"), null))
        assertNull(visitedLiveRoom(state.copy(visit = visit.copy(direct = true)), null))
        assertNull(visitedLiveRoom(state.copy(visit = null), null))
        // A visited channel that is a joined row of the open space has its card in place.
        assertNull(visitedLiveRoom(state.copy(selectedSpace = detail.copy(channels = detail.channels + visited)), null))
    }

    @Test fun `the full list's title counts people`() {
        assertEquals("In #design · 12 people", voiceListTitle("design", 12))
        assertEquals("In #design · 1 person", voiceListTitle("design", 1))
    }

    @Test fun `your call's status counts everyone once connected, and says how far the connection is until then`() {
        val two = listOf(person("me"), person("a"))
        assertEquals("You’re in · 2", callStatusLabel(VoiceState(phase = VoiceState.Phase.CONNECTED, participants = two)))
        assertEquals("Connecting…", callStatusLabel(VoiceState(phase = VoiceState.Phase.CONNECTING, participants = two)))
        assertEquals("Reconnecting…", callStatusLabel(VoiceState(phase = VoiceState.Phase.RECONNECTING, participants = two)))
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
