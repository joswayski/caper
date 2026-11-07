package chat.caper.android

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import chat.caper.android.data.ActivityFilter
import chat.caper.android.data.ActivityItem
import chat.caper.android.data.feedbackAge
import chat.caper.android.ui.*
import kotlinx.coroutines.delay

/** A modal inbox keeps the underlying chat draft and voice call mounted. */
@OptIn(ExperimentalLayoutApi::class)
@Composable internal fun ActivityInboxEntry(viewModel: CaperViewModel, onOpened: () -> Unit) {
    val inbox by viewModel.activityInbox.collectAsState()
    val state by viewModel.state.collectAsState()
    val epoch = viewModel.accountEpoch
    val own = state.chatAuthorId ?: state.account?.id
    var shown by remember { mutableStateOf(false) }
    var reacting by remember(epoch) { mutableStateOf<ActivityItem?>(null) }
    var picking by remember(epoch) { mutableStateOf(emptySet<String>()) }
    TextButton({ shown = true }, Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        Icon(painterResource(R.drawable.lucide_bell), null, Modifier.size(16.dp), tint = TextMuted)
        Spacer(Modifier.width(8.dp))
        Text("Notifications", Modifier.weight(1f), color = Text, fontSize = 13.sp)
        if (inbox.unreadCount > 0) Text(if (inbox.unreadCount > 99) "99+" else "${inbox.unreadCount}",
            Modifier.clearAndSetSemantics { contentDescription = "${inbox.unreadCount} unread notifications" }, color = TerracottaBright)
    }
    if (!shown) return
    DisposableEffect(viewModel) {
        viewModel.setActivityVisible(true)
        onDispose { viewModel.setActivityVisible(false) }
    }
    LaunchedEffect(Unit) {
        viewModel.loadActivity()
        while (true) { delay(15_000); if (!viewModel.activityInbox.value.loading) viewModel.loadActivity(poll = true) }
    }
    Dialog({ if (picking.isEmpty() && reacting == null) shown = false }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxWidth().fillMaxHeight(0.94f).padding(12.dp).widthIn(max = 620.dp),
            color = SurfaceConversation, shape = MaterialTheme.shapes.medium) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Notifications", Modifier.weight(1f), fontSize = 20.sp, fontWeight = FontWeight.Bold, color = Text)
                    TextButton({ shown = false }, enabled = picking.isEmpty() && reacting == null) { Text("Done") }
                }
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    ActivityFilter.entries.forEach { filter ->
                        FilterChip(inbox.filter == filter, { viewModel.filterActivity(filter, inbox.unreadOnly) },
                            label = { Text(filter.label) })
                    }
                }
                Row(Modifier.toggleable(inbox.unreadOnly, role = Role.Switch) {
                    viewModel.filterActivity(inbox.filter, it)
                }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.width(40.dp).height(48.dp), contentAlignment = Alignment.Center) {
                        Switch(inbox.unreadOnly, null, Modifier.wrapContentSize(unbounded = true).scale(0.75f),
                            colors = caperSwitchColors())
                    }
                    Text("Unread only", color = Text)
                    Text("(${inbox.unreadCount})", color = TextMuted)
                }
                inbox.error?.let {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(it, Modifier.weight(1f), color = ErrorText, fontSize = 13.sp)
                        TextButton({ viewModel.loadActivity(poll = true) }, enabled = !inbox.loading) { Text("Retry") }
                    }
                }
                if (inbox.loading && inbox.visibleItems.isEmpty()) LinearProgressIndicator(Modifier.fillMaxWidth())
                if (!inbox.loading && inbox.visibleItems.isEmpty() && inbox.error == null && inbox.cursor == null) Column(
                    Modifier.fillMaxWidth().padding(vertical = 24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Icon(painterResource(R.drawable.lucide_bell), null, tint = TextMuted)
                    Text(if (inbox.unreadOnly) "You’re all caught up" else "No notifications here yet", color = Text, fontWeight = FontWeight.Bold)
                    Text(if (inbox.unreadOnly) "No unread notifications in this view." else "Messages, mentions, replies and reactions will appear here.",
                        color = TextMuted, fontSize = 12.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                }
                // The previous filter's rows stay put until the new page replaces them: dimmed after a beat
                // (fast replies swap without a flash) and inert. Leaving the stale state is instant.
                val still = reducedMotion()
                val dim by animateFloatAsState(if (inbox.stale) 0.5f else 1f, when {
                    !inbox.stale -> snap()
                    still -> snap(150)
                    else -> tween(MOTION_FAST, 150, EaseOut)
                }, label = "stale")
                LazyColumn(Modifier.weight(1f).graphicsLayer { alpha = dim }, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(inbox.visibleItems, key = { it.id }) { item ->
                        Column(Modifier.fillMaxWidth().background(SurfaceRaised, MaterialTheme.shapes.small).padding(12.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Avatar(item.actorName, 28.dp, avatarId = item.avatarId)
                                Spacer(Modifier.width(10.dp))
                                Text("${if (item.unread) "● " else ""}${item.actorName}",
                                    Modifier.weight(1f).clickable(enabled = !inbox.stale && inbox.opening != item.id, role = Role.Button) {
                                        viewModel.readActivity(item, open = true) { shown = false; onOpened() }
                                    // The dot is unread state (web: aria-label "Unread"), not a glyph to read out.
                                    }.semantics { contentDescription = item.actorName; if (item.unread) stateDescription = "Unread" }, fontWeight = if (item.unread) FontWeight.Bold else FontWeight.Medium, color = Text, fontSize = 13.sp)
                                Text(feedbackAge(item.createdAt), color = TextMuted, fontSize = 11.sp)
                                if (item.unread) IconButton({ viewModel.readActivity(item) }, enabled = !inbox.stale) {
                                    Icon(painterResource(R.drawable.lucide_check), "Mark notification from ${item.actorName} read", Modifier.size(16.dp), tint = TextMuted)
                                }
                            }
                            Column(Modifier.fillMaxWidth().clickable(enabled = !inbox.stale && inbox.opening != item.id, role = Role.Button) {
                                viewModel.readActivity(item, open = true) { shown = false; onOpened() }
                            }, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                val description = when (item.kind) {
                                    "reaction" -> "Reacted ${item.emoji.orEmpty()} to your message"
                                    "direct.request" -> "Wants to send you messages"
                                    "space.invitation" -> "Invited you to a space"
                                    "channel.invitation" -> "Invited you to a private channel"
                                    else -> null
                                }
                                description?.let { Text(it, color = TextMuted, fontSize = 12.sp) }
                                Text(item.body, color = Text, fontSize = 13.sp, maxLines = 3)
                                Text(activityLocation(item), color = TextMuted, fontSize = 12.sp)
                            }
                            val direct = state.directConversations.firstOrNull { it.id == item.conversationId }
                            val writable = item.canReply && direct?.incoming != true && direct?.blocked != true && direct?.peer?.id !in state.blockedIds
                            val saving = item.id in inbox.reacting
                            if (writable) FlowRow(verticalArrangement = Arrangement.Center, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                                TextButton({ viewModel.toggleActivityReply(item) }, Modifier.heightIn(min = 48.dp), enabled = !inbox.stale) { Text(if (item.id in inbox.expanded) "Close reply" else "Reply", color = TextMuted) }
                                // After you react, the message's reactions show; yours toggle off with a tap (web).
                                inbox.reactions[item.id].orEmpty().forEach { reaction ->
                                    val mine = own != null && own in reaction.authorIds
                                    Box(Modifier.heightIn(min = 48.dp), contentAlignment = Alignment.Center) {
                                        ReactionChip(reaction, mine, canReact = !inbox.stale && !saving,
                                            toggle = { viewModel.reactToActivity(item, reaction.emoji, !mine) }, showReactors = {})
                                    }
                                }
                                TextButton({ reacting = item }, Modifier.heightIn(min = 48.dp), enabled = !inbox.stale && !saving) { Text("React", color = TextMuted) }
                            }
                            inbox.statuses[item.id]?.let {
                                Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 12.sp)
                            }
                            if (item.id in inbox.expanded) key(item.draftKey, epoch) {
                                Column(Modifier.entrance(MOTION_FAST, dy = 4.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                    Text(item.replyLabel, color = TextMuted, fontSize = 12.sp)
                                    ActivityReplyComposer(viewModel, item, writable) { picking = if (it) picking + item.id else picking - item.id }
                                }
                            }
                        }
                    }
                    if (inbox.cursor != null) item {
                        TextButton({ viewModel.loadActivity(more = true) }, enabled = !inbox.loading) { Text("Load more") }
                    }
                }
            }
        }
    }
    reacting?.let { item ->
        EmojiPicker({ reacting = null }) { emoji ->
            reacting = null
            if (viewModel.accountEpoch == epoch) viewModel.reactToActivity(item, emoji)
        }
    }
}

/** Web's location line: "#channel · Space", or "Direct message". */
internal fun activityLocation(item: ActivityItem): String =
    item.spaceName?.let { space -> listOfNotNull(item.channelName?.let { "#$it" }, space).joinToString(" · ") } ?: "Direct message"

/** The profile-card composer, backed by a target-keyed draft rather than an actor DM. */
@Composable private fun ActivityReplyComposer(viewModel: CaperViewModel, item: ActivityItem, writable: Boolean, pickerInteraction: (Boolean) -> Unit) {
    val drafts by viewModel.profileDrafts.state.collectAsState()
    val state by viewModel.state.collectAsState()
    val person = item.draftKey
    val epoch = viewModel.accountEpoch
    val current = { viewModel.accountEpoch == epoch }
    DisposableEffect(person, epoch) { onDispose { pickerInteraction(false) } }
    val stage: (VoiceRecording) -> Unit = { recording ->
        if (current()) viewModel.stageProfileVoice(person, null, item.actorName, recording.file, recording.durationMs, recording.waveform)
        else recording.file.delete()
    }
    ProfileMessageComposer(item.actorName, drafts[person] ?: ProfileMessageDraft(), writable, state.uploadsEnabled,
        placeholder = item.replyLabel, voiceKey = person, sendLabel = "Send reply: ${item.replyLabel}",
        pickerInteraction = pickerInteraction,
        text = { if (current()) viewModel.profileDrafts.text(person, it) },
        add = { if (current()) viewModel.addProfileAttachments(person, null, item.actorName, it) },
        keyboardAttachment = { if (current()) viewModel.addProfileAttachments(person, null, item.actorName, listOf(it)).singleOrNull() else null },
        paste = { if (current()) viewModel.pasteProfileAttachments(person, null, item.actorName) },
        remove = { if (current()) viewModel.removeProfileAttachment(person, it) },
        toggleSpoiler = { if (current()) viewModel.toggleProfileAttachmentSpoiler(person, it) },
        edit = { key, change, preview -> if (current()) viewModel.editProfileAttachment(person, key, change, preview) else preview?.recycle() },
        retry = { if (current()) viewModel.retryProfileAttachment(person, it) },
        send = { if (current()) viewModel.sendActivityReply(item) },
        loadMedia = viewModel::klipyPage, mediaAvailable = state.klipy != null,
        sendMedia = { gif, kind, query -> current() && viewModel.sendActivityReply(item, gif, kind, query) },
        stageVoice = stage,
        recordVoice = { recording ->
            if (current()) viewModel.sendProfileVoice(person, null, item.actorName, recording.file, recording.durationMs, recording.waveform) {}
            else recording.file.delete()
        },
        dictate = viewModel::dictate,
        retryVoice = { if (current()) viewModel.retryProfileVoice(person, null, item.actorName) {} },
        discardVoice = { if (current()) viewModel.discardProfileVoice(person) },
    )
}
