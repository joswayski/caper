package chat.caper.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.isOutOfBounds
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.zIndex
import chat.caper.android.model.Account
import chat.caper.android.model.AppUiState
import chat.caper.android.model.Channel
import chat.caper.android.model.Participant
import chat.caper.android.model.SpaceDetail
import chat.caper.android.model.ownPicture
import chat.caper.android.model.voiceJoinUnavailableLabel
import chat.caper.android.model.voiceRootKey
import chat.caper.android.ui.*
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceState
import java.util.Locale
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive

// Voice "Live now" (design H, revised): live channels grow into cards in place in the channel list
// (your call's row becomes your call card; another live room's row gains its faces, names and Join),
// voice with no row in the list sits in a card above Channels, and on phones a strip under the chat
// header shows your call. Joining never changes the chat being read; the conversation header carries
// no voice, and nothing says who is speaking except the green rings on faces.

/** Your call card's green-tinted surface and border. */
internal val LiveCallSurface = Color(0xFF1A2A22)
internal val LiveCallBorder = Color(0xFF3A6B4B)
/** Another live room's card. */
internal val LiveRoomSurface = Color(0xFF18231F)
internal val LiveRoomBorder = Color(0xFF28362F)
/** The phone strip under the chat header. */
internal val VoiceStripSurface = Color(0xFF1A2622)
internal val VoiceStripBorder = Color(0xFF26352F)
/** Connecting and reconnecting, as the old dock showed them. */
internal val VoiceAmber = Color(0xFFD9AB5C)
/** A quiet row's dim headphones join. */
internal val QuietJoin = Color(0xFF687174)
/** Leave's destructive tint (web's #FF7182). */
internal val LeaveRed = Color(0xFFFF7182)

private val CardShape = RoundedCornerShape(10.dp)
/** A face in your call card: a 36dp avatar inside a 44dp touch target that also holds its speaking ring. */
private val CallFaceSize = 36.dp
private val CallFaceSlot = 44.dp

/** Your call card shows everyone up to this many; beyond it, the first ones and a "+N" tile. */
internal const val CALL_FACES_MAX = 10
/** The phone strip shows everyone up to this many; beyond it, the first four and a "+N" chip. */
internal const val STRIP_FACES_MAX = 5

/** A call is under way (joining, connected or reconnecting), not idle or failed. */
internal fun inVoice(voice: VoiceState): Boolean = voice.phase != VoiceState.Phase.IDLE && voice.phase != VoiceState.Phase.FAILED

/** A channel with people in its voice. */
internal data class LiveRoom(val channel: Channel, val people: List<Participant>)

/** What a channel row shows for voice: nothing more (quiet), another live room's card, or your call's card. */
internal enum class RowVoice { QUIET, ROOM, CALL }

/**
 * Your call's channel when it is a row of [detail]'s list (a joined channel of that space), whose row
 * then grows into your call card; otherwise null, and the card sits above the channels.
 */
internal fun callRowChannelId(detail: SpaceDetail?, voice: VoiceState): String? {
    val channelId = voice.channelId?.takeIf { inVoice(voice) } ?: return null
    if (detail == null || voice.spaceId != null && voice.spaceId != detail.space.id) return null
    return channelId.takeIf { id -> detail.channels.any { it.id == id && it.joined } }
}

/** A row's voice: your call's ([callRowId]), a joined channel with people in its voice, or quiet. */
internal fun rowVoice(channel: Channel, demo: Boolean, rosters: Map<String, List<Participant>>, callRowId: String?): RowVoice = when {
    !channel.joined -> RowVoice.QUIET
    channel.id == callRowId -> RowVoice.CALL
    rosters[voiceRootKey(demo, channel.id)].orEmpty().isNotEmpty() -> RowVoice.ROOM
    else -> RowVoice.QUIET
}

/**
 * The channel rows listed, in list order: the [shown] ones (Channels expanded, joined or browsed,
 * matching the search) and every row with voice, which collapsing or searching never hides.
 */
internal fun listedChannels(channels: List<Channel>, shown: (Channel) -> Boolean, voice: (Channel) -> RowVoice): List<Channel> =
    channels.filter { voice(it) != RowVoice.QUIET || shown(it) }

/**
 * A feedback visit's live space channel, which a team member joins like a member. It has no row in
 * the list, so its room card sits above the channels. Null while it is your call ([callChannelId]),
 * while nobody is there, and for a joined channel of the open space (its row has the card).
 */
internal fun visitedLiveRoom(state: AppUiState, callChannelId: String?): LiveRoom? {
    val visit = state.visit?.takeIf { !it.direct && it.spaceId != null && it.channelId != callChannelId } ?: return null
    val channel = state.selectedChannel?.takeIf { it.id == visit.channelId && !it.direct } ?: return null
    if (state.selectedSpace?.channels?.any { it.id == channel.id && it.joined } == true) return null
    return state.voiceRosters[channel.id]?.takeIf { it.isNotEmpty() }?.let { LiveRoom(channel, it) }
}

/** A live room's one-line names: "Maya", "Maya and Alex", "Maya, Alex and Sam", "Maya, Alex and 3 more". */
internal fun liveRoomNames(names: List<String>): String = when {
    names.size > 3 -> "${names[0]}, ${names[1]} and ${names.size - 2} more"
    names.size == 3 -> "${names[0]}, ${names[1]} and ${names[2]}"
    names.size == 2 -> "${names[0]} and ${names[1]}"
    else -> names.firstOrNull().orEmpty()
}

/** Your call's faces: you first, then everyone else in join order. Speaking never reorders them. */
internal fun callFaceOrder(people: List<Participant>, selfId: String?): List<Participant> {
    val self = people.firstOrNull { it.id == selfId } ?: return people
    return listOf(self) + people.filter { it.id != selfId }
}

/**
 * How many faces your call card shows for [total] people when two rows hold [capacity]: everyone
 * up to ten, otherwise one fewer than fits so a "+N" tile holds the rest (the first nine and
 * "+N" at the usual widths).
 */
internal fun callFacesShown(total: Int, capacity: Int): Int {
    val slots = capacity.coerceIn(1, CALL_FACES_MAX)
    return if (total <= slots) total else slots - 1
}

/**
 * How many of your call card's faces (same people, same order) the phone strip shows for [total]
 * people: everyone up to five, so the strip and the card agree, otherwise four and a "+N" chip.
 */
internal fun stripFacesShown(total: Int): Int = if (total <= STRIP_FACES_MAX) total else STRIP_FACES_MAX - 1

/** "1 person", "12 people". */
internal fun peopleCount(count: Int): String = "$count ${if (count == 1) "person" else "people"}"

/** The full list's title: "In #design · 12 people". */
internal fun voiceListTitle(channelName: String, count: Int): String = "In #$channelName · ${peopleCount(count)}"

/** Your call's status: "You’re in · N" once connected, otherwise how far the connection is. */
internal fun callStatusLabel(voice: VoiceState): String = when (voice.phase) {
    VoiceState.Phase.CONNECTING -> "Connecting…"
    VoiceState.Phase.RECONNECTING -> "Reconnecting…"
    else -> "You’re in · ${voice.participants.size}"
}

/** What the full list says about someone, the most telling first. */
internal enum class VoiceListStatus { Speaking, LocallyMuted, Deafened, Muted }

internal fun voiceListStatus(self: Boolean, speaking: Boolean, locallyMuted: Boolean, deafened: Boolean, muted: Boolean): VoiceListStatus? = when {
    speaking -> VoiceListStatus.Speaking
    locallyMuted && !self -> VoiceListStatus.LocallyMuted
    deafened -> VoiceListStatus.Deafened
    muted -> VoiceListStatus.Muted
    else -> null
}

internal fun voiceListStatusLabel(status: VoiceListStatus, name: String): String = when (status) {
    VoiceListStatus.Speaking -> "Speaking"
    VoiceListStatus.LocallyMuted -> "You muted $name"
    VoiceListStatus.Deafened -> "Deafened"
    VoiceListStatus.Muted -> "Muted"
}

/** A channel's one-tap voice action: Join, Switch (you are in a call elsewhere) or Joining…, and whether it can be pressed now. */
internal data class VoiceJoinAction(
    val switch: Boolean,
    /** A join to this channel is waiting for permission or connecting. */
    val joining: Boolean,
    val enabled: Boolean,
    /** Why Join can't be pressed yet (web's tooltip), or null. */
    val unavailableLabel: String?,
) {
    fun description(channelName: String): String = when {
        joining -> "Joining voice in #$channelName"
        switch -> "Switch voice to #$channelName"
        else -> "Join voice in #$channelName"
    }

    /** The accessible name, which says why Join is unavailable while it is. */
    fun label(channelName: String): String = if (joining || enabled) description(channelName) else unavailableLabel ?: description(channelName)
}

/** The same rules as before the redesign: one join at a time, never mid-connection, and only where voice is available. */
internal fun voiceJoinAction(state: AppUiState, voice: VoiceState, pending: VoiceJoinIntent?, channel: Channel): VoiceJoinAction {
    val active = voice.channelId.takeIf { inVoice(voice) }
    val available = state.voiceAvailable(channel)
    val denied = channel.id in state.deniedVoiceChannels
    val joining = (active == channel.id && voice.phase == VoiceState.Phase.CONNECTING) || pending?.channelId == channel.id
    val ready = pending == null && voice.phase != VoiceState.Phase.CONNECTING && voice.phase != VoiceState.Phase.RECONNECTING &&
        !denied && available == true
    return VoiceJoinAction(
        switch = active != null && active != channel.id, joining = joining, enabled = ready && !joining,
        unavailableLabel = if (denied) null else voiceJoinUnavailableLabel(available),
    )
}

/** Your call's clock: the channel's shared one while you are still connecting, then the call's own. */
internal fun callSessionStartedAt(state: AppUiState, voice: VoiceState): Long? {
    val channelId = voice.channelId ?: return voice.sessionStartedAt
    val demo = state.selectedSpace?.space?.let { it.demo && it.id == voice.spaceId } == true
    return if (voice.phase == VoiceState.Phase.CONNECTING) state.voiceSessionStartedAt[voiceRootKey(demo, channelId)] ?: voice.sessionStartedAt
    else voice.sessionStartedAt
}

/** A call participant's name, yours from your saved profile at once. */
private fun callName(person: Participant, voice: VoiceState, account: Account?): String =
    if (person.id == voice.selfId) account?.displayName ?: person.name else person.name

/** Your own face follows your saved picture at once; others' come with the roster. */
private fun callPicture(person: Participant, voice: VoiceState, account: Account?): Pair<String?, String?> =
    if (person.id == voice.selfId && account != null) ownPicture(account, person.avatarAsset, person.avatarUrl)
    else person.avatarAsset to person.avatarUrl

internal fun formatVoiceSessionDuration(startedAt: Long, now: Long): String {
    val seconds = ((now - startedAt).coerceAtLeast(0L) / 1_000L)
    val hours = seconds / 3_600
    val minutes = (seconds % 3_600) / 60
    val remainder = seconds % 60
    return if (hours == 0L) "%02d:%02d".format(Locale.ROOT, minutes, remainder)
    else "%d:%02d:%02d".format(Locale.ROOT, hours, minutes, remainder)
}

@Composable internal fun VoiceSessionTimer(startedAt: Long, modifier: Modifier = Modifier, color: Color = VoiceSessionGreen) {
    var now by remember(startedAt) { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(startedAt) {
        while (isActive) {
            now = System.currentTimeMillis()
            delay(1_000L - now.mod(1_000L))
        }
    }
    val duration = formatVoiceSessionDuration(startedAt, now)
    Text(
        duration,
        modifier.clearAndSetSemantics {
            contentDescription = "Voice session duration"
            stateDescription = duration
        },
        color = color,
        fontSize = 11.sp,
        maxLines = 1,
        // Satoshi's tabular figures, as on web, so nothing shifts each second.
        style = TabularNumbers,
    )
}

/** Web prepares the join on touch-down, before the tap completes. */
@Composable internal fun Modifier.prepareVoiceOnPress(active: Boolean, prepare: () -> Unit): Modifier {
    val latest by rememberUpdatedState(prepare)
    return pointerInput(active) {
        awaitEachGesture {
            awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
            if (active) latest()
        }
    }
}

/** A live channel row's end: a small green waveform and how many are in voice. It is not a control. */
@Composable internal fun LiveVoiceCount(count: Int, modifier: Modifier = Modifier) {
    Row(modifier.padding(horizontal = 6.dp).clearAndSetSemantics { contentDescription = "$count in voice" }, verticalAlignment = Alignment.CenterVertically) {
        Icon(painterResource(R.drawable.lucide_audio_lines), null, Modifier.size(14.dp), tint = VoiceSessionGreen)
        Spacer(Modifier.width(4.dp))
        Text("$count", color = VoiceSessionGreen, fontSize = 11.sp, fontWeight = FontWeight.Bold, maxLines = 1, style = TabularNumbers)
    }
}

/** A quiet channel row's one-tap join: dim headphones in a 44dp target, always shown on touch. */
@Composable internal fun RowVoiceJoin(channelName: String, action: VoiceJoinAction, join: () -> Unit, prepare: () -> Unit) {
    IconButton(join, Modifier.size(44.dp).prepareVoiceOnPress(action.enabled, prepare).semantics {
        contentDescription = action.label(channelName)
        if (action.joining) stateDescription = "Busy"
    }, enabled = action.enabled) {
        Icon(painterResource(R.drawable.lucide_headphones), null, Modifier.size(18.dp),
            tint = when { action.joining -> VoiceSessionGreen; action.enabled -> QuietJoin; else -> QuietJoin.copy(alpha = 0.5f) })
    }
}

/** A disabled control still keeps its taps from the card or strip around it, which would otherwise act on them. */
private fun Modifier.keepsTaps(): Modifier = pointerInput(Unit) { detectTapGestures {} }

/** A live room's Join (eggshell) or Switch (neutral) button; Joining… and unavailable step back to neutral chrome. */
@Composable internal fun VoiceJoinButton(channelName: String, action: VoiceJoinAction, join: () -> Unit, prepare: () -> Unit, modifier: Modifier = Modifier) {
    val label = when { action.joining -> "Joining…"; action.switch -> "Switch"; else -> "Join" }
    val colors = if (action.switch) ButtonDefaults.buttonColors(
        containerColor = SurfaceRaised, contentColor = Text, disabledContainerColor = SurfaceRaised, disabledContentColor = TextMuted,
    ) else primaryButtonColors()
    Button(
        join,
        modifier.keepsTaps().heightIn(min = 40.dp).prepareVoiceOnPress(action.enabled, prepare).semantics {
            contentDescription = action.label(channelName)
            if (action.joining) stateDescription = "Busy"
        },
        enabled = action.enabled,
        shape = MaterialTheme.shapes.small,
        colors = colors,
        border = if (action.switch) BorderStroke(1.dp, Border) else null,
        contentPadding = PaddingValues(horizontal = 14.dp),
    ) { Text(label, fontSize = 12.sp, fontWeight = FontWeight.Bold, maxLines = 1, softWrap = false) }
}

/**
 * Voice with no row in the channel list, above Channels and without a heading: your call when its
 * channel is not a row here (another space, a visited channel), then a feedback visit's live room.
 * The open space's live rows show their cards in place instead ([RowCallCard], [RowRoomCard]).
 * Absent otherwise.
 */
@Composable internal fun LiveNowSection(
    state: AppUiState,
    voice: VoiceState,
    pendingVoiceJoin: VoiceJoinIntent?,
    /** Phones open the full list as a bottom sheet; wider layouts as a popover beside the sidebar. */
    narrow: Boolean,
    joinVoice: (Channel) -> Unit,
    prepareVoice: (Channel) -> Unit,
    openChannel: (Channel) -> Unit,
    openCall: () -> Unit,
    leaveVoice: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val detail = state.selectedSpace
    val inCall = inVoice(voice)
    val call = inCall && callRowChannelId(detail, voice) == null
    val room = if (BuildConfig.ENABLE_NATIVE_VOICE) visitedLiveRoom(state, voice.channelId.takeIf { inCall }) else null
    if (!call && room == null) return
    Column(modifier.fillMaxWidth().padding(top = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (call) LiveCallCard(
            voice, state.account,
            otherSpaceName = voice.spaceName?.takeIf { voice.spaceId != null && voice.spaceId != detail?.space?.id },
            sessionStartedAt = callSessionStartedAt(state, voice), narrow = narrow, open = openCall, leave = leaveVoice,
        )
        room?.let { room ->
            key(room.channel.id) {
                LiveRoomCard(
                    room, voiceJoinAction(state, voice, pendingVoiceJoin, room.channel),
                    state.voiceSessionStartedAt[voiceRootKey(detail?.space?.let { it.demo && it.id == room.channel.spaceId } == true, room.channel.id)],
                    open = { openChannel(room.channel) }, join = { joinVoice(room.channel) }, prepare = { prepareVoice(room.channel) },
                )
            }
        }
    }
}

/** Above the channels, another live room: its name and timer, then who is there and Join or Switch. A tap anywhere but Join opens its chat. */
@Composable internal fun LiveRoomCard(
    room: LiveRoom,
    action: VoiceJoinAction,
    sessionStartedAt: Long?,
    open: () -> Unit,
    join: () -> Unit,
    prepare: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val channel = room.channel
    RoomCard(room, action, bodyStartedAt = null, open, join, prepare, rowHeading = false, modifier) {
        Row(Modifier.fillMaxWidth().padding(start = 10.dp, end = 10.dp, top = 2.dp).heightIn(min = 40.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(painterResource(if (channel.private) R.drawable.lucide_lock_keyhole else R.drawable.lucide_hash), null, Modifier.size(15.dp), tint = TextMuted)
            Spacer(Modifier.width(4.dp))
            Text(channel.name, Modifier.weight(1f), color = Text, fontSize = 13.sp, fontWeight = FontWeight.SemiBold,
                maxLines = 1, overflow = TextOverflow.Ellipsis)
            sessionStartedAt?.let { VoiceSessionTimer(it, Modifier.padding(start = 8.dp), color = TextMuted) }
        }
    }
}

/**
 * In the channel list, a live room's [row] (with its waveform count) above who is there, the room's
 * timer and Join or Switch. A tap anywhere but Join and the row's own controls opens its chat.
 */
@Composable internal fun RowRoomCard(
    room: LiveRoom,
    action: VoiceJoinAction,
    sessionStartedAt: Long?,
    open: () -> Unit,
    join: () -> Unit,
    prepare: () -> Unit,
    modifier: Modifier = Modifier,
    row: @Composable () -> Unit,
) = RoomCard(room, action, sessionStartedAt, open, join, prepare, rowHeading = true, modifier, row)

/**
 * A live room: [heading], up to three faces and "+N", one line of names, the timer when
 * [bodyStartedAt] is known, and Join or Switch. Names give way first. No speaking rings: there is
 * no audio from rooms you are not in. Under a listed row only the body arrives; above the channels
 * the whole card does.
 */
@Composable private fun RoomCard(
    room: LiveRoom,
    action: VoiceJoinAction,
    bodyStartedAt: Long?,
    open: () -> Unit,
    join: () -> Unit,
    prepare: () -> Unit,
    rowHeading: Boolean,
    modifier: Modifier,
    heading: @Composable () -> Unit,
) {
    Column(
        modifier.then(if (rowHeading) Modifier else Modifier.entrance(MOTION_BASE, dy = 4.dp)).fillMaxWidth().clip(CardShape)
            .background(LiveRoomSurface).border(1.dp, LiveRoomBorder, CardShape)
            .clickable(onClickLabel = "Open chat", onClick = open).padding(bottom = 8.dp),
    ) {
        heading()
        Row(
            Modifier.then(if (rowHeading) Modifier.entrance(MOTION_BASE, dy = 4.dp) else Modifier).fillMaxWidth().padding(horizontal = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Row(Modifier.clearAndSetSemantics {}, verticalAlignment = Alignment.CenterVertically) {
                VoiceFaces(room.people, 3, 22.dp, LiveRoomSurface)
                if (room.people.size > 3) Box(
                    Modifier.padding(start = 4.dp).height(22.dp).clip(RoundedCornerShape(11.dp)).background(LiveRoomBorder).padding(horizontal = 6.dp),
                    contentAlignment = Alignment.Center,
                ) { Text("+${room.people.size - 3}", color = Text, fontSize = 10.sp, fontWeight = FontWeight.Bold, maxLines = 1, style = TabularNumbers) }
            }
            Spacer(Modifier.width(6.dp))
            Text(liveRoomNames(room.people.map { it.name }), Modifier.weight(1f), color = TextMuted, fontSize = 12.sp,
                maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis)
            bodyStartedAt?.let { VoiceSessionTimer(it, Modifier.padding(start = 8.dp), color = TextMuted) }
            Spacer(Modifier.width(8.dp))
            VoiceJoinButton(room.channel.name, action, join, prepare)
        }
    }
}

/** Opens the full list, with [openId]'s audio controls already open. */
private class VoiceListRequest(val openId: String?)

/** Your call's status ([callStatusLabel]): green once connected, amber until then, and its timer once connected. */
@Composable internal fun CallStatus(voice: VoiceState, sessionStartedAt: Long?, modifier: Modifier = Modifier) {
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        Text(callStatusLabel(voice), color = if (voice.phase == VoiceState.Phase.CONNECTED) VoiceSessionGreen else VoiceAmber, fontSize = 11.sp,
            fontWeight = FontWeight.Bold, maxLines = 1, softWrap = false, style = TabularNumbers)
        if (voice.phase != VoiceState.Phase.CONNECTING) sessionStartedAt?.let { VoiceSessionTimer(it, Modifier.padding(start = 6.dp)) }
    }
}

/**
 * Above the channels, your call when its channel has no row in the list: a waveform, its channel
 * (and its space when that is not the one being viewed), status and timer, then the faces and
 * controls of [RowCallCard]. A tap anywhere but the controls opens its chat, in its own space.
 */
@Composable internal fun LiveCallCard(
    voice: VoiceState,
    account: Account?,
    /** The call's space, named when it is not the space being viewed. */
    otherSpaceName: String?,
    sessionStartedAt: Long?,
    narrow: Boolean,
    open: () -> Unit,
    leave: () -> Unit,
    modifier: Modifier = Modifier,
) {
    CallCard(voice, account, narrow, open, leave, rowHeading = false, modifier) {
        Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 12.dp, top = 4.dp).heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(painterResource(R.drawable.lucide_audio_lines), null, Modifier.size(16.dp),
                tint = if (voice.phase == VoiceState.Phase.CONNECTED) VoiceSessionGreen else VoiceAmber)
            Spacer(Modifier.width(6.dp))
            Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                Text(voice.channelName.orEmpty(), Modifier.weight(1f, fill = false), color = Text, fontSize = 13.sp, fontWeight = FontWeight.Bold,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
                otherSpaceName?.let {
                    Spacer(Modifier.width(5.dp))
                    Text(it, Modifier.weight(1f, fill = false), color = TextMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            CallStatus(voice, sessionStartedAt, Modifier.padding(start = 6.dp))
        }
    }
}

/**
 * In the channel list, your call's channel [row] (its status and timer at the end, see [CallStatus])
 * grown into your call card: everyone's faces (at most two rows) and Mute, Deafen and Leave (Cancel
 * while joining). A tap anywhere but those and the row's menu opens the chat, as the row does; a
 * long press on a face or "+N" (or its accessibility action) opens the full list.
 */
@Composable internal fun RowCallCard(
    voice: VoiceState,
    account: Account?,
    narrow: Boolean,
    open: () -> Unit,
    leave: () -> Unit,
    modifier: Modifier = Modifier,
    row: @Composable () -> Unit,
) = CallCard(voice, account, narrow, open, leave, rowHeading = true, modifier, row)

/**
 * Your call: [heading], the faces and the controls. Nothing in it comes or goes with speaking; the
 * green rings on faces show who talks. Under a listed row only the body arrives; above the channels
 * the whole card does.
 */
@Composable private fun CallCard(
    voice: VoiceState,
    account: Account?,
    narrow: Boolean,
    open: () -> Unit,
    leave: () -> Unit,
    rowHeading: Boolean,
    modifier: Modifier,
    heading: @Composable () -> Unit,
) {
    val context = LocalContext.current
    val connecting = voice.phase == VoiceState.Phase.CONNECTING
    val people = callFaceOrder(voice.participants, voice.selfId)
    var list by remember { mutableStateOf<VoiceListRequest?>(null) }
    var cardSize by remember { mutableStateOf(IntSize.Zero) }
    Box(modifier.then(if (rowHeading) Modifier else Modifier.entrance(MOTION_BASE, dy = 4.dp)).fillMaxWidth().onSizeChanged { cardSize = it }) {
        Column(
            Modifier.fillMaxWidth().clip(CardShape).background(LiveCallSurface).border(1.dp, LiveCallBorder, CardShape)
                .clickable(onClickLabel = "Open chat", onClick = open).padding(bottom = 12.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            heading()
            Column(
                Modifier.then(if (rowHeading) Modifier.entrance(MOTION_BASE, dy = 4.dp) else Modifier).fillMaxWidth().padding(horizontal = 12.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                if (people.isNotEmpty()) CallFaces(people, voice, account, open) { list = VoiceListRequest(it) }
                Row(Modifier.fillMaxWidth().padding(top = 2.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    CallMuteToggle(voice) { CaperEffects.toggle(voice.muted); VoiceCallService.toggleMute(context) }
                    CallToggle(
                        voice.deafened, if (voice.deafened) R.drawable.lucide_volume_x else R.drawable.lucide_headphones,
                        if (voice.deafened) "Undeafen audio" else "Deafen audio", voice.monitoring, "Stop mic test to change deafen",
                    ) { CaperEffects.toggle(voice.deafened); VoiceCallService.toggleDeafen(context) }
                    Spacer(Modifier.weight(1f))
                    TextButton(
                        leave,
                        Modifier.heightIn(min = 44.dp).semantics { contentDescription = if (connecting) "Cancel joining voice" else "Leave voice" },
                        shape = MaterialTheme.shapes.small,
                        colors = ButtonDefaults.textButtonColors(contentColor = LeaveRed),
                        contentPadding = PaddingValues(horizontal = 10.dp),
                    ) {
                        Icon(painterResource(R.drawable.lucide_phone_off), null, Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(if (connecting) "Cancel" else "Leave", fontSize = 12.sp, fontWeight = FontWeight.Bold, maxLines = 1, softWrap = false)
                    }
                }
            }
        }
        list?.let { request ->
            if (narrow) VoiceCallListSheet(voice, account, request.openId) { list = null }
            else {
                val density = LocalDensity.current
                // A popover beside the sidebar, its top level with the card's; it scrolls when tall.
                DropdownMenu(
                    true, { list = null }, Modifier.width(320.dp),
                    offset = with(density) { DpOffset(cardSize.width.toDp() + 24.dp, -cardSize.height.toDp()) },
                    containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border),
                ) { key(request) { VoiceCallList(voice, account, request.openId, { list = null }) } }
            }
        }
    }
}

/**
 * A long press here calls [onLongPress] (with the long-press haptic) and keeps the rest of that press
 * from the card around it. A shorter press is left to the card, which opens the chat; a scroll, a
 * swipe or moving off cancels it.
 */
@Composable private fun Modifier.longPressOnly(onLongPress: () -> Unit): Modifier {
    val latest by rememberUpdatedState(onLongPress)
    val haptics = LocalHapticFeedback.current
    return pointerInput(Unit) {
        awaitEachGesture {
            // The Final pass comes after the card (whose tap consumes the down), any scroller and
            // swipeCancelsPresses have seen each change: only a later consumed change cancels.
            awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Final)
            val ended = withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) {
                while (true) {
                    val event = awaitPointerEvent(PointerEventPass.Final)
                    if (event.changes.any { !it.pressed || it.isConsumed || it.isOutOfBounds(size, extendedTouchPadding) }) break
                }
            }
            if (ended != null) return@awaitEachGesture
            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
            latest()
            // The card sees the rest of this press consumed, so it never taps.
            do {
                val event = awaitPointerEvent()
                event.changes.forEach { it.consume() }
            } while (event.changes.any { it.pressed })
        }
    }
}

/**
 * Faces in join order (you first), wrapping onto at most two rows; past that a "+N" tile holds the
 * rest. A tap is the card's ([open]); a long press on a face opens the full list with that person's
 * audio open ([showList], none for yours), and on "+N" with none open.
 */
@Composable private fun CallFaces(people: List<Participant>, voice: VoiceState, account: Account?, open: () -> Unit, showList: (String?) -> Unit) {
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val perRow = (maxWidth / CallFaceSlot).toInt().coerceAtLeast(1)
        val shown = callFacesShown(people.size, perRow * 2)
        val folded = people.drop(shown)
        val tiles: List<Participant?> = people.take(shown) + if (folded.isEmpty()) emptyList() else listOf(null)
        Column {
            tiles.chunked(perRow).forEach { row ->
                Row {
                    row.forEach { person ->
                        if (person == null) MoreFacesTile(folded.size, folded.any { it.id in voice.speakingParticipants }, open) { showList(null) }
                        else key(person.id) { CallFace(person, voice, account, open) { showList(person.id.takeIf { it != voice.selfId }) } }
                    }
                }
            }
        }
    }
}

@Composable private fun CallFace(person: Participant, voice: VoiceState, account: Account?, open: () -> Unit, showList: () -> Unit) {
    val self = person.id == voice.selfId
    val name = callName(person, voice, account)
    val (asset, url) = callPicture(person, voice, account)
    val speaking = person.id in voice.speakingParticipants
    val muted = if (self) voice.muted else person.muted
    val deafened = if (self) voice.deafened else person.deafened
    val status = voiceListStatus(self, speaking, locallyMuted = !self && person.id in voice.locallyMutedParticipants, deafened = deafened, muted = muted)
    val listLabel = if (self) "Show everyone in voice" else "Audio controls for ${person.name}"
    Box(
        Modifier.size(CallFaceSlot).longPressOnly(showList)
            .semantics(mergeDescendants = true) {
                contentDescription = if (self) "$name (you)" else name
                status?.let { stateDescription = voiceListStatusLabel(it, person.name) }
                onClick("Open chat") { open(); true }
                customActions = listOf(CustomAccessibilityAction(listLabel) { showList(); true })
            },
        contentAlignment = Alignment.Center,
    ) {
        Avatar(name, CallFaceSize, avatarId = person.avatarId, speaking = speaking, avatarAsset = asset, avatarUrl = url, voiceRing = true)
        if (deafened || muted) Box(
            Modifier.align(Alignment.BottomEnd).padding(end = 2.dp, bottom = 2.dp).size(16.dp).background(LiveCallSurface, CircleShape).padding(2.dp),
        ) {
            Icon(painterResource(if (deafened) R.drawable.lucide_headphone_off else R.drawable.lucide_mic_off), null, Modifier.fillMaxSize(), tint = ErrorText)
        }
    }
}

/** The "+N" tile; it rings when someone folded into it is speaking. */
@Composable private fun MoreFacesTile(count: Int, speaking: Boolean, open: () -> Unit, showList: () -> Unit) {
    Box(
        Modifier.size(CallFaceSlot).longPressOnly(showList)
            .semantics(mergeDescendants = true) {
                contentDescription = "$count more in voice"
                if (speaking) stateDescription = "Speaking"
                onClick("Open chat") { open(); true }
                customActions = listOf(CustomAccessibilityAction("Show everyone in voice") { showList(); true })
            },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(CallFaceSize)
                .then(if (speaking) Modifier.drawBehind {
                    val stroke = 2.dp.toPx()
                    drawCircle(VoiceSessionGreen, radius = size.minDimension / 2 + 2.dp.toPx() + stroke / 2, style = Stroke(stroke))
                } else Modifier)
                .clip(CircleShape).background(LiveRoomBorder),
            contentAlignment = Alignment.Center,
        ) { Text("+$count", color = Text, fontSize = 12.sp, fontWeight = FontWeight.Bold, maxLines = 1, style = TabularNumbers) }
    }
}

/** Mute with a pressed state, in your call card and the phone strip; it matches the account bar's. */
@Composable internal fun CallMuteToggle(voice: VoiceState, onToggle: () -> Unit) {
    CallToggle(
        voice.muted, if (voice.muted) R.drawable.lucide_mic_off else R.drawable.lucide_mic,
        if (voice.muted) "Unmute microphone" else "Mute microphone", voice.monitoring, "Stop mic test to change mute", onToggle,
    )
}

/**
 * A 44dp call toggle: pressed reads as a soft red fill. Like the account bar's, it waits while the
 * Audio test holds the microphone, and meanwhile a tap on it does nothing (not the card's or strip's action).
 */
@Composable internal fun CallToggle(pressed: Boolean, icon: Int, label: String, monitoring: Boolean, monitoringLabel: String, onToggle: () -> Unit) {
    val shape = MaterialTheme.shapes.small
    IconButton(
        onToggle,
        Modifier.keepsTaps().size(44.dp)
            .background(if (pressed) ErrorText.copy(alpha = 0.16f) else Color.Transparent, shape)
            .border(1.dp, if (pressed) ErrorText.copy(alpha = 0.45f) else LiveCallBorder, shape)
            .semantics { if (monitoring) stateDescription = monitoringLabel },
        enabled = !monitoring,
    ) {
        Icon(painterResource(icon), label, Modifier.size(18.dp), tint = (if (pressed) ErrorText else Text).copy(alpha = if (monitoring) 0.4f else 1f))
    }
}

/** Phones: the full list as a bottom sheet. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable private fun VoiceCallListSheet(voice: VoiceState, account: Account?, openId: String?, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = SurfaceRaised, contentColor = Text) {
        VoiceCallList(voice, account, openId, onDismiss,
            Modifier.fillMaxWidth().swipeCancelsPresses().verticalScroll(rememberScrollState()).padding(bottom = 24.dp))
    }
}

/**
 * Everyone in your call ("In #design · 12 people"): name, status and, for others while connected,
 * their local audio (User volume 0–200% and Mute for me), which only changes what you hear.
 */
@Composable internal fun VoiceCallList(voice: VoiceState, account: Account?, openId: String?, close: () -> Unit, modifier: Modifier = Modifier) {
    val people = callFaceOrder(voice.participants, voice.selfId)
    val title = voiceListTitle(voice.channelName.orEmpty(), people.size)
    var open by remember(openId) { mutableStateOf(openId) }
    Column(modifier.semantics { paneTitle = title }) {
        Row(Modifier.fillMaxWidth().padding(start = 18.dp, end = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(title, Modifier.weight(1f).semantics { heading() }, color = Text, fontSize = 15.sp, fontWeight = FontWeight.Bold,
                maxLines = 1, overflow = TextOverflow.Ellipsis)
            TextButton(close, Modifier.heightIn(min = 44.dp), shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Close") }
        }
        HorizontalDivider(color = Border)
        people.forEach { person ->
            key(person.id) { VoiceListRow(voice, person, account, open == person.id) { open = if (open == person.id) null else person.id } }
        }
    }
}

@Composable private fun VoiceListRow(voice: VoiceState, person: Participant, account: Account?, expanded: Boolean, toggle: () -> Unit) {
    val context = LocalContext.current
    val self = person.id == voice.selfId
    val name = callName(person, voice, account)
    val (asset, url) = callPicture(person, voice, account)
    val speaking = person.id in voice.speakingParticipants
    val locallyMuted = !self && person.id in voice.locallyMutedParticipants
    val status = voiceListStatus(self, speaking, locallyMuted, if (self) voice.deafened else person.deafened, if (self) voice.muted else person.muted)
    // Only a connected call has someone else's audio to adjust; your own row has none.
    val controls = !self && voice.phase == VoiceState.Phase.CONNECTED
    Column(Modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(start = 18.dp, end = 8.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(name, 34.dp, avatarId = person.avatarId, speaking = speaking, avatarAsset = asset, avatarUrl = url, voiceRing = true)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(if (self) "$name (you)" else name, color = Text, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                status?.let {
                    Text(voiceListStatusLabel(it, person.name), color = when (it) {
                        VoiceListStatus.Speaking -> VoiceSessionGreen
                        VoiceListStatus.LocallyMuted -> TerracottaBright
                        else -> TextMuted
                    }, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            if (controls) TextButton(
                toggle,
                Modifier.heightIn(min = 44.dp).semantics {
                    contentDescription = "Audio controls for ${person.name}"
                    stateDescription = if (expanded) "Expanded" else "Collapsed"
                },
                shape = MaterialTheme.shapes.small,
                colors = ButtonDefaults.textButtonColors(contentColor = TextMuted),
                contentPadding = PaddingValues(horizontal = 8.dp),
            ) {
                Text("Audio", fontSize = 11.sp)
                Spacer(Modifier.width(4.dp))
                DisclosureChevron(expanded, Modifier.size(14.dp))
            }
        }
        if (controls && expanded) Column(
            Modifier.fillMaxWidth().padding(start = 64.dp, end = 18.dp, bottom = 12.dp).semantics { contentDescription = "${person.name} local audio settings" },
        ) {
            val volume = voice.participantVolumes[person.id] ?: 100
            // SurfaceRaised also names surfaceVariant, so the list inherits TextMuted.
            // Keep control labels primary while the percentage and hint stay muted.
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text("User volume", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = Text)
                Text("$volume%", fontSize = 11.sp, color = TextMuted, style = TabularNumbers)
            }
            Slider(volume.toFloat(), { CaperEffects.slider(it / 200f); VoiceCallService.setParticipantVolume(context, person.id, it.toInt()) },
                Modifier.semantics { contentDescription = "${person.name} volume" }, valueRange = 0f..200f)
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text("Mute for me", Modifier.weight(1f), fontSize = 12.sp, color = Text)
                Switch(locallyMuted, { CaperEffects.toggle(!it); VoiceCallService.toggleParticipantMute(context, person.id) },
                    modifier = Modifier.semantics { contentDescription = "Mute ${person.name} for me" }, colors = caperSwitchColors())
            }
            Text("Only changes what you hear.", color = TextMuted, fontSize = 10.sp)
        }
    }
}

/** Phones' strip under the chat header: 56dp, its own surface and a bottom rule. */
@Composable private fun VoiceStripFrame(modifier: Modifier, onClick: (() -> Unit)?, onClickLabel: String?, content: @Composable RowScope.() -> Unit) {
    Row(
        modifier.entrance(MOTION_BASE, dy = (-4).dp).fillMaxWidth().height(56.dp).background(VoiceStripSurface)
            .drawBehind {
                val stroke = 1.dp.toPx()
                drawLine(VoiceStripBorder, Offset(0f, size.height - stroke / 2), Offset(size.width, size.height - stroke / 2), strokeWidth = stroke)
            }
            .then(if (onClick != null) Modifier.clickable(role = Role.Button, onClickLabel = onClickLabel, onClick = onClick) else Modifier)
            .padding(start = 18.dp, end = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        content = content,
    )
}

/**
 * Phones, in a call, under any conversation's header: the call's channel, your call card's faces
 * ([VoiceStripFaces]), "Connecting…"/"Reconnecting…" until connected, the timer and Mute. Tapping
 * anything but Mute opens Browse, where the call card is.
 */
@Composable internal fun VoiceCallStrip(
    voice: VoiceState, account: Account?, sessionStartedAt: Long?, openBrowse: () -> Unit, modifier: Modifier = Modifier,
    /** Mute's action; the call service's by default. */
    toggleMute: (() -> Unit)? = null,
) {
    val context = LocalContext.current
    val count = voice.participants.size
    val phase = when (voice.phase) {
        VoiceState.Phase.CONNECTING -> "Connecting…"
        VoiceState.Phase.RECONNECTING -> "Reconnecting…"
        else -> null
    }
    val channel = "#${voice.channelName.orEmpty()}"
    VoiceStripFrame(modifier, openBrowse, "Show your call") {
        Row(
            Modifier.weight(1f).clearAndSetSemantics {
                contentDescription = listOfNotNull("In voice in $channel" + if (count > 0) ", ${peopleCount(count)}" else "", phase).joinToString(". ")
            },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(painterResource(R.drawable.lucide_audio_lines), null, Modifier.size(16.dp), tint = if (phase == null) VoiceSessionGreen else VoiceAmber)
            Text(channel, Modifier.weight(1f, fill = false), color = Text, fontSize = 13.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (count > 0) VoiceStripFaces(voice, account)
            phase?.let { Text(it, Modifier.weight(1f, fill = false), color = VoiceAmber, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
        sessionStartedAt?.takeIf { voice.phase != VoiceState.Phase.CONNECTING }?.let { VoiceSessionTimer(it) }
        CallMuteToggle(voice, toggleMute ?: { CaperEffects.toggle(voice.muted); VoiceCallService.toggleMute(context) })
    }
}

private val StripFaceSize = 22.dp

/**
 * The strip's faces: your call card's people in its order (you, then join order), overlapping, up
 * to five, otherwise the first four and a "+N" chip ([stripFacesShown]). A speaker's ring draws
 * above its neighbours; the chip rings when someone folded into it speaks.
 */
@Composable internal fun VoiceStripFaces(voice: VoiceState, account: Account?, modifier: Modifier = Modifier) {
    val people = callFaceOrder(voice.participants, voice.selfId)
    val shown = people.take(stripFacesShown(people.size))
    val folded = people.drop(shown.size)
    if (shown.isEmpty()) return
    val step = StripFaceSize - 6.dp
    // 4dp each side keeps room for an end face's ring.
    Row(modifier.padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(width = step * (shown.size - 1) + StripFaceSize, height = StripFaceSize)) {
            shown.forEachIndexed { index, person ->
                key(person.id) {
                    val name = callName(person, voice, account)
                    val (asset, url) = callPicture(person, voice, account)
                    val speaking = person.id in voice.speakingParticipants
                    // The strip's one summary replaces these; on their own (in tests) they name each face.
                    Avatar(name, StripFaceSize, Modifier.offset(x = step * index).zIndex(if (speaking) 1f else 0f)
                        .semantics { contentDescription = if (person.id == voice.selfId) "$name (you)" else name },
                        avatarId = person.avatarId, speaking = speaking, avatarAsset = asset, avatarUrl = url,
                        outline = BorderStroke(1.5.dp, VoiceStripSurface), voiceRing = true)
                }
            }
        }
        if (folded.isNotEmpty()) {
            val chip = RoundedCornerShape(11.dp)
            Box(
                Modifier.padding(start = 4.dp).height(StripFaceSize).clip(chip).background(LiveRoomBorder)
                    .then(if (folded.any { it.id in voice.speakingParticipants }) Modifier.border(1.5.dp, VoiceSessionGreen, chip) else Modifier)
                    .padding(horizontal = 6.dp),
                contentAlignment = Alignment.Center,
            ) { Text("+${folded.size}", color = Text, fontSize = 10.sp, fontWeight = FontWeight.Bold, maxLines = 1, style = TabularNumbers) }
        }
    }
}

/** Phones, not in a call, reading a channel with people in its voice: how many are talking, their faces and Join. */
@Composable internal fun VoiceJoinStrip(channelName: String, people: List<Participant>, action: VoiceJoinAction, join: () -> Unit, prepare: () -> Unit, modifier: Modifier = Modifier) {
    VoiceStripFrame(modifier, onClick = null, onClickLabel = null) {
        Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(painterResource(R.drawable.lucide_audio_lines), null, Modifier.size(16.dp), tint = VoiceSessionGreen)
            Text("${people.size} talking", color = Text, fontSize = 13.sp, fontWeight = FontWeight.Bold, maxLines = 1, softWrap = false, style = TabularNumbers)
            VoiceFaces(people, 3, 24.dp, VoiceStripSurface)
        }
        VoiceJoinButton(channelName, action, join, prepare)
    }
}
