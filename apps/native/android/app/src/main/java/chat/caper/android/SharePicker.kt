package chat.caper.android

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.AttachmentPolicy
import chat.caper.android.data.ShareTarget
import chat.caper.android.data.Sharing
import chat.caper.android.data.friendlyError
import chat.caper.android.model.AppUiState
import chat.caper.android.model.ForwardDestination
import chat.caper.android.ui.ErrorText
import chat.caper.android.ui.SurfaceRaised
import chat.caper.android.ui.TextMuted
import kotlinx.coroutines.CancellationException

/**
 * "Share to Caper": what another app shared, and the conversations it can go to, most recent
 * first and searchable. Picking one opens it with the files staged and the text in the composer.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun SharePickerSheet(share: IncomingShare, state: AppUiState, viewModel: CaperViewModel) {
    var destinations by remember { mutableStateOf<List<ForwardDestination>?>(null) }
    var search by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    LaunchedEffect(attempt) {
        error = null
        try { destinations = viewModel.shareDestinations() }
        catch (reason: Throwable) { if (reason is CancellationException) throw reason; error = friendlyError(reason, "Conversations are unavailable.") }
    }
    val avatars = remember(state.directConversations) { state.directConversations.associate { it.id to it.peer } }
    ModalBottomSheet(onDismissRequest = { viewModel.cancelShare(share) }, containerColor = SurfaceRaised) {
        Column(Modifier.fillMaxWidth().heightIn(max = 660.dp).padding(horizontal = 18.dp).padding(bottom = 18.dp).imePadding().semantics { paneTitle = "Share to Caper" },
            verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Share to Caper", Modifier.semantics { heading() }, fontWeight = FontWeight.Bold, fontSize = 16.sp)
            Text(summary(share), color = TextMuted, fontSize = 12.sp, maxLines = 3, overflow = TextOverflow.Ellipsis)
            OutlinedTextField(search, { search = it }, label = { Text("Find a channel or DM") }, singleLine = true, modifier = Modifier.fillMaxWidth())
            val visible = destinations?.let { Sharing.matching(it, search) }
            if (destinations == null && error == null) Text("Loading conversations…", color = TextMuted)
            error?.let { Text(it, color = ErrorText); TextButton({ attempt++ }, shape = MaterialTheme.shapes.small) { Text("Retry loading") } }
            if (visible?.isEmpty() == true) Text(if (search.isBlank()) "No conversations yet. Join a channel or start a DM." else "No matching conversations.", color = TextMuted)
            if (!visible.isNullOrEmpty()) {
                if (search.isBlank()) Text("Recent", color = TextMuted, fontSize = 11.sp, fontWeight = FontWeight.Bold)
                LazyColumn(Modifier.weight(1f, fill = false)) {
                    items(visible, key = { it.id }) { destination ->
                        val target = ShareTarget.of(destination)
                        Row(Modifier.fillMaxWidth().clip(MaterialTheme.shapes.small)
                            .clickable(enabled = target != null, role = Role.Button) { target?.let { viewModel.placeShare(share, it) } }
                            .padding(horizontal = 4.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            val peer = avatars[destination.id].takeIf { destination.direct }
                            if (peer != null) Avatar(peer.displayName, 32.dp, avatarId = peer.avatarId, avatarAsset = peer.avatarAsset, avatarUrl = peer.avatarUrl)
                            else Box(Modifier.size(32.dp), contentAlignment = Alignment.Center) { Text(if (destination.direct) destination.name.take(1).uppercase() else "#", color = TextMuted, fontWeight = FontWeight.Bold) }
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                Text(destination.name, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(destination.spaceName, color = TextMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    }
                }
            }
        }
    }
}

private fun summary(share: IncomingShare): String {
    val files = share.uris.size
    val parts = buildList {
        if (files > 0) add("$files file${if (files == 1) "" else "s"}${if (files > AttachmentPolicy.MAX_ATTACHMENTS) " (the first ${AttachmentPolicy.MAX_ATTACHMENTS} are attached)" else ""}")
        if (share.text.isNotEmpty()) add("“${share.text.take(140)}”")
    }
    return "${parts.joinToString(" and ")}. Pick a conversation; you can edit before sending."
}
