package chat.caper.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.ApiException
import chat.caper.android.data.withFreshUrls
import chat.caper.android.model.*
import chat.caper.android.ui.Border
import chat.caper.android.ui.EmojiImage
import chat.caper.android.ui.SurfaceRaised
import chat.caper.android.ui.TextMuted
import chat.caper.android.ui.Terracotta
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import java.math.BigInteger
import java.util.UUID

@OptIn(ExperimentalLayoutApi::class)
@Composable private fun SharedOriginal(
    message: ChatMessage, state: AppUiState? = null, onAttachmentFailed: (ChatAttachment, Int?) -> Unit = { _, _ -> },
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Avatar(message.author.name, 24.dp, avatarId = message.author.avatarId)
            Text(message.author.name, fontWeight = FontWeight.Bold, fontSize = 13.sp)
            if (message.editedAt != null) Text("edited", color = TextMuted, fontSize = 10.sp)
        }
        // A file-only original has empty text: show just its files, in their processing state.
        if (message.content.text.isNotEmpty()) Text(message.content.text, fontSize = 14.sp)
        MessageAttachments(
            message.content.attachments.map { it.withFreshUrls(state?.freshAttachmentUrls?.get(it.id)) }, false, onAttachmentFailed,
            state?.attachmentProgress.orEmpty(),
        )
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            message.reactions.forEach { reaction -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                EmojiImage(reaction.emoji, reaction.emoji, Modifier.size(18.dp))
                Text(reaction.authorIds.size.toString(), fontSize = 12.sp)
            } }
        }
    }
}

@Composable internal fun ForwardCard(
    message: ChatMessage, state: AppUiState? = null, onAttachmentFailed: (ChatAttachment, Int?) -> Unit = { _, _ -> }, open: () -> Unit,
) {
    val forward = message.forward ?: return
    Surface(Modifier.padding(start = 62.dp, end = 18.dp, bottom = 8.dp).fillMaxWidth(), shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Forwarded · live", color = TextMuted, fontSize = 11.sp)
            val original = forward.message
            if (original == null) Text("Original conversation unavailable.", color = TextMuted)
            else {
                SharedOriginal(original, state, onAttachmentFailed)
                TextButton(open) { Text("${original.thread?.replyCount?.let { "$it ${if (it == 1) "reply" else "replies"} · " }.orEmpty()}View conversation") }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun ForwardPickerSheet(message: ChatMessage, viewModel: CaperViewModel, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    var destinations by remember { mutableStateOf<List<ForwardDestination>?>(null) }
    var selected by remember { mutableStateOf<ForwardDestination?>(null) }
    var search by remember { mutableStateOf("") }
    var note by remember { mutableStateOf("") }
    var key by remember { mutableStateOf<UUID?>(null) }
    var sending by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    LaunchedEffect(attempt) {
        error = null
        try { destinations = viewModel.forwardDestinations().sortedBy { "${it.spaceName} ${it.name}" } }
        catch (reason: Throwable) { if (reason is CancellationException) throw reason; error = reason.message }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = SurfaceRaised) {
        Column(Modifier.fillMaxWidth().heightIn(max = 660.dp).verticalScroll(rememberScrollState()).padding(18.dp).imePadding(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Forward message", fontWeight = FontWeight.Bold, fontSize = 16.sp)
            Text("Shares this conversation live, including future edits, reactions and replies. People in the destination can read and forward it.", color = TextMuted, fontSize = 12.sp)
            SharedOriginal(message.forward?.message ?: message)
            OutlinedTextField(search, { search = it }, label = { Text("Find a channel or DM") }, singleLine = true, enabled = key == null, modifier = Modifier.fillMaxWidth())
            val visible = destinations?.filter { "${it.spaceName} ${it.name}".contains(search, ignoreCase = true) }
            visible?.forEach { destination ->
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    RadioButton(selected?.id == destination.id, onClick = { selected = destination }, enabled = key == null)
                    TextButton({ selected = destination }, enabled = key == null) { Column(horizontalAlignment = Alignment.Start) {
                        Text("${if (destination.direct) "" else "# "}${destination.name}")
                        Text(destination.spaceName, color = TextMuted, fontSize = 11.sp)
                    } }
                }
            }
            if (destinations == null && error == null) Text("Loading destinations…", color = TextMuted)
            if (visible?.isEmpty() == true) Text("No matching destinations. Join a channel or start a DM.", color = TextMuted)
            OutlinedTextField(note, { note = it }, label = { Text("Add a note (optional)") }, enabled = key == null, modifier = Modifier.fillMaxWidth())
            error?.let { Text(it, color = Terracotta); if (destinations == null) TextButton({ attempt++ }) { Text("Retry loading") } }
            Button({
                val destination = selected ?: return@Button
                if (sending) return@Button
                val intent = key ?: UUID.randomUUID().also { key = it }
                sending = true; error = null
                scope.launch {
                    try { viewModel.forward(message, destination.id, intent, note.trim()); onDismiss() }
                    catch (reason: Throwable) {
                        if (reason is CancellationException) throw reason
                        val rejected = reason is ApiException && reason.status in listOf(400, 401, 403, 404, 409, 422)
                        if (rejected) key = null
                        error = "${if (rejected) "Not sent." else "Not confirmed. Retry checks the same forward."} ${reason.message.orEmpty()}"
                    } finally { sending = false }
                }
            }, enabled = selected != null && !sending && note.codePointCount(0, note.length) <= 4000) { Text(if (sending) "Forwarding…" else if (key != null) "Retry forward" else "Forward") }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun ForwardConversationSheet(message: ChatMessage, viewModel: CaperViewModel, onDismiss: () -> Unit) {
    var conversation by remember { mutableStateOf<ForwardConversation?>(null) }
    var oldest by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    var pages by remember { mutableIntStateOf(1) }
    LaunchedEffect(message.forward?.seq, attempt, pages) {
        loading = true; error = null
        try {
            var fresh = viewModel.forwardedConversation(message)
            var loadedPages = 1
            while (fresh.hasMore && fresh.messages.isNotEmpty() && (loadedPages < pages || oldest?.let { BigInteger(fresh.messages.first().seq) > BigInteger(it) } == true)) {
                val earlier = viewModel.forwardedConversation(message, fresh.messages.first().seq)
                if (earlier.messages.isEmpty()) break
                fresh = fresh.copy(messages = earlier.messages + fresh.messages, hasMore = earlier.hasMore); loadedPages++
            }
            oldest = fresh.messages.firstOrNull()?.seq; conversation = fresh
        } catch (reason: Throwable) { if (reason is CancellationException) throw reason; conversation = null; error = reason.message }
        finally { loading = false }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = SurfaceRaised) {
        Column(Modifier.fillMaxWidth().heightIn(max = 660.dp).verticalScroll(rememberScrollState()).padding(18.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text("Forwarded conversation", fontWeight = FontWeight.Bold, fontSize = 16.sp)
            Text("Live · Read-only original. Replies to the forward stay in the destination.", color = TextMuted, fontSize = 12.sp)
            conversation?.root?.let { SharedOriginal(it) }
            if (conversation?.hasMore == true) TextButton({ pages++ }, enabled = !loading) { Text("Load older replies") }
            conversation?.messages?.forEach { reply -> key(reply.id) { SharedOriginal(reply) } }
            if (conversation?.root == null && !loading && error == null) Text("Original conversation unavailable.", color = TextMuted)
            if (conversation?.messages?.isEmpty() == true && conversation?.root != null) Text("No replies yet.", color = TextMuted)
            if (loading) Text("Updating conversation…", color = TextMuted)
            error?.let { Text(it, color = Terracotta); TextButton({ attempt++ }) { Text("Retry") } }
        }
    }
}
