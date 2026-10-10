package chat.caper.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.ApiException
import chat.caper.android.data.withFreshUrls
import chat.caper.android.data.friendlyError
import chat.caper.android.data.linkRanges
import chat.caper.android.model.*
import chat.caper.android.ui.Border
import chat.caper.android.ui.EmojiImage
import chat.caper.android.ui.ErrorText
import chat.caper.android.ui.SurfaceRaised
import chat.caper.android.ui.TextMuted
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
        if (message.content.text.isNotEmpty()) LinkedText(message.content.text, fontSize = 14.sp)
        MessageAttachments(
            message.content.attachments.map { it.withFreshUrls(state?.freshAttachmentUrls?.get(it.id)) }, false, onAttachmentFailed,
            state?.attachmentProgress.orEmpty(), caption = message.author.name,
        )
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            message.reactions.forEach { reaction -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                EmojiImage(reaction.emoji, reaction.emoji, Modifier.size(18.dp))
                Text(reaction.authorIds.size.toString(), fontSize = 12.sp)
            } }
        }
    }
}

/** The original's text with its `http(s)://` and `www.` links (`data/Links.kt`); like web, no mention pills here. */
@Composable private fun LinkedText(text: String, fontSize: TextUnit) {
    val links = remember(text) { linkRanges(text) }
    if (links.isEmpty()) return Text(text, fontSize = fontSize)
    val uriHandler = LocalUriHandler.current
    val annotated = remember(text, links, uriHandler) {
        buildAnnotatedString {
            append(text)
            links.forEach { link -> addLink(LinkAnnotation.Url(link.href, MessageLink) { openExternalLink(uriHandler, link.href) }, link.start, link.end) }
        }
    }
    Text(annotated, fontSize = fontSize)
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
                TextButton(open, shape = MaterialTheme.shapes.small) { Text("${original.thread?.replyCount?.let { "$it ${if (it == 1) "reply" else "replies"} · " }.orEmpty()}View conversation") }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun ForwardPickerSheet(message: ChatMessage, viewModel: CaperViewModel, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    var destinations by remember { mutableStateOf<List<ForwardDestination>?>(null) }
    var selected by remember { mutableStateOf(setOf<String>()) }
    var search by remember { mutableStateOf("") }
    var note by remember { mutableStateOf("") }
    var pending by remember { mutableStateOf<List<Pair<ForwardDestination, UUID>>?>(null) }
    var confirmed by remember { mutableIntStateOf(0) }
    var sending by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    LaunchedEffect(attempt) {
        error = null
        try { destinations = viewModel.forwardDestinations().sortedBy { "${it.spaceName} ${it.name}" } }
        catch (reason: Throwable) { if (reason is CancellationException) throw reason; error = friendlyError(reason, "Destinations are unavailable.") }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = SurfaceRaised) {
        Column(Modifier.fillMaxWidth().heightIn(max = 660.dp).verticalScroll(rememberScrollState()).padding(18.dp).imePadding(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Forward message", fontWeight = FontWeight.Bold, fontSize = 16.sp)
            Text("Shares this conversation live, including future edits, reactions and replies. People in the destination can read and forward it.", color = TextMuted, fontSize = 12.sp)
            SharedOriginal(message.forward?.message ?: message)
            OutlinedTextField(search, { search = it }, label = { Text("Find a space, channel or DM") }, singleLine = true, enabled = pending == null, modifier = Modifier.fillMaxWidth())
            val terms = search.trim().split(Regex("\\s+")).map { it.removePrefix("#") }
            val visible = destinations?.filter { destination -> terms.all { "${destination.spaceName} ${destination.name}".contains(it, ignoreCase = true) } }
            visible?.forEach { destination ->
                Row(Modifier.fillMaxWidth().clip(MaterialTheme.shapes.small).toggleable(value = destination.id in selected, enabled = pending == null, role = Role.Checkbox, onValueChange = { checked -> selected = if (checked) selected + destination.id else selected - destination.id }).padding(vertical = 8.dp, horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text("${if (destination.direct) "" else "# "}${destination.name}", fontWeight = FontWeight.Bold)
                        Text(destination.spaceName, color = TextMuted, fontSize = 11.sp)
                    }
                    Checkbox(destination.id in selected, onCheckedChange = null, enabled = pending == null)
                }
            }
            if (destinations == null && error == null) Text("Loading destinations…", color = TextMuted)
            if (visible?.isEmpty() == true) Text("No matching destinations. Join a channel or start a DM.", color = TextMuted)
            OutlinedTextField(note, { note = it }, label = { Text("Add a note (optional)") }, enabled = pending == null, modifier = Modifier.fillMaxWidth())
            error?.let { Text(it, color = ErrorText); if (destinations == null) TextButton({ attempt++ }, shape = MaterialTheme.shapes.small) { Text("Retry loading") } }
            Button({
                if (sending || selected.isEmpty()) return@Button
                val intent = pending ?: destinations.orEmpty().filter { it.id in selected }.map { it to UUID.randomUUID() }
                pending = intent
                val text = note.trim()
                sending = true; error = null
                scope.launch {
                    try {
                        for ((destination, key) in intent) {
                            viewModel.forward(message, destination.id, key, text)
                            confirmed++
                            selected = selected - destination.id
                            pending = pending?.drop(1)
                        }
                        onDismiss()
                    }
                    catch (reason: Throwable) {
                        if (reason is CancellationException) throw reason
                        val rejected = reason is ApiException && reason.status in listOf(400, 401, 403, 404, 409, 422)
                        if (rejected) pending = null
                        // Multi-destination progress with a readable reason, never raw exception text.
                        val done = if (confirmed > 0) "Forwarded to $confirmed ${if (confirmed == 1) "destination" else "destinations"}. " else ""
                        val status = when {
                            rejected && confirmed > 0 -> "Remaining forwards not sent."
                            rejected -> "Not sent."
                            confirmed > 0 -> "Remaining forwards not confirmed. Retry checks the same forwards."
                            else -> "Not confirmed. Retry checks the same forward."
                        }
                        error = "$done$status ${friendlyError(reason, "Try again.")}"
                    } finally { sending = false }
                }
            }, enabled = selected.isNotEmpty() && !sending && note.codePointCount(0, note.length) <= 4000, shape = MaterialTheme.shapes.small) { Text(if (sending) "Forwarding…" else if (pending != null) "Retry forwards (${selected.size})" else "Forward (${selected.size})") }
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
        } catch (reason: Throwable) { if (reason is CancellationException) throw reason; conversation = null; error = friendlyError(reason, "Conversation is unavailable.") }
        finally { loading = false }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = SurfaceRaised) {
        Column(Modifier.fillMaxWidth().heightIn(max = 660.dp).verticalScroll(rememberScrollState()).padding(18.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text("Forwarded conversation", fontWeight = FontWeight.Bold, fontSize = 16.sp)
            Text("Live · Read-only original. Replies to the forward stay in the destination.", color = TextMuted, fontSize = 12.sp)
            conversation?.root?.let { SharedOriginal(it) }
            if (conversation?.hasMore == true) TextButton({ pages++ }, enabled = !loading, shape = MaterialTheme.shapes.small) { Text("Load older replies") }
            conversation?.messages?.forEach { reply -> key(reply.id) { SharedOriginal(reply) } }
            if (conversation?.root == null && !loading && error == null) Text("Original conversation unavailable.", color = TextMuted)
            if (conversation?.messages?.isEmpty() == true && conversation?.root != null) Text("No replies yet.", color = TextMuted)
            if (loading) Text("Updating conversation…", color = TextMuted)
            error?.let { Text(it, color = ErrorText); TextButton({ attempt++ }, shape = MaterialTheme.shapes.small) { Text("Retry") } }
        }
    }
}
