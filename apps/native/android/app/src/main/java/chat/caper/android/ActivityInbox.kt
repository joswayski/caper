package chat.caper.android

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
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
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
@Composable internal fun ActivityInboxEntry(viewModel: CaperViewModel, onOpened: () -> Unit) {
    val inbox by viewModel.activityInbox.collectAsState()
    val state by viewModel.state.collectAsState()
    val epoch = viewModel.accountEpoch
    var shown by remember { mutableStateOf(false) }
    var reacting by remember(epoch) { mutableStateOf<ActivityItem?>(null) }
    var picking by remember(epoch) { mutableStateOf(emptySet<String>()) }
    TextButton({ shown = true }, Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        Text("Notifications", Modifier.weight(1f), color = Text, fontSize = 13.sp)
        if (inbox.unreadCount > 0) Text("${inbox.unreadCount}", color = TerracottaBright)
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
                            label = { Text(filter.label) }, enabled = inbox.reading == null)
                    }
                }
                Row(Modifier.toggleable(inbox.unreadOnly, enabled = inbox.reading == null, role = Role.Switch) {
                    viewModel.filterActivity(inbox.filter, it)
                }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.width(40.dp).height(48.dp), contentAlignment = Alignment.Center) {
                        Switch(inbox.unreadOnly, null, Modifier.wrapContentSize(unbounded = true).scale(0.75f),
                            enabled = inbox.reading == null, colors = caperSwitchColors())
                    }
                    Text("Unread only", color = Text)
                    Text("(${inbox.unreadCount})", color = TextMuted)
                }
                inbox.error?.let {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(it, Modifier.weight(1f), color = ErrorText, fontSize = 13.sp)
                        TextButton({ viewModel.loadActivity(poll = true) }, enabled = !inbox.loading && inbox.reading == null) { Text("Retry") }
                    }
                }
                if (inbox.loading && inbox.visibleItems.isEmpty()) LinearProgressIndicator(Modifier.fillMaxWidth())
                if (!inbox.loading && inbox.visibleItems.isEmpty() && inbox.error == null) Text("No notifications match these filters.", color = TextMuted)
                LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(inbox.visibleItems, key = { it.id }) { item ->
                        Column(Modifier.fillMaxWidth().background(SurfaceRaised, MaterialTheme.shapes.small).padding(12.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text("${if (item.unread) "● " else ""}${item.actorName}",
                                    Modifier.weight(1f).clickable(enabled = inbox.reading == null) {
                                        viewModel.readActivity(item, open = true) { shown = false; onOpened() }
                                    }, fontWeight = if (item.unread) FontWeight.Bold else FontWeight.Medium, color = Text, fontSize = 13.sp)
                                Text(feedbackAge(item.createdAt), color = TextMuted, fontSize = 11.sp)
                                if (item.unread) IconButton({ viewModel.readActivity(item) }, enabled = inbox.reading == null) {
                                    Icon(painterResource(R.drawable.lucide_check), "Mark notification from ${item.actorName} read", Modifier.size(16.dp), tint = TextMuted)
                                }
                            }
                            Column(Modifier.fillMaxWidth().clickable(enabled = inbox.reading == null) {
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
                                Text(listOfNotNull(item.spaceName, item.channelName).joinToString(" · "), color = TextMuted, fontSize = 12.sp)
                            }
                            val direct = state.directConversations.firstOrNull { it.id == item.conversationId }
                            val writable = item.canReply && direct?.incoming != true && direct?.blocked != true && direct?.peer?.id !in state.blockedIds
                            if (writable) Row {
                                TextButton({ viewModel.toggleActivityReply(item) }, Modifier.heightIn(min = 48.dp)) { Text(if (item.id in inbox.expanded) "Close reply" else "Reply", color = TextMuted) }
                                TextButton({ reacting = item }, Modifier.heightIn(min = 48.dp)) { Text("React", color = TextMuted) }
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
                        TextButton({ viewModel.loadActivity(more = true) }, enabled = !inbox.loading && inbox.reading == null) { Text("Load more") }
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
