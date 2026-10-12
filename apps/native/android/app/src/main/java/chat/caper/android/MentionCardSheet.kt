package chat.caper.android

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.ui.*
import chat.caper.android.model.AppUiState
import chat.caper.android.model.BlockedAccount
import kotlinx.coroutines.launch

/**
 * The card a person's @mention pill opens: on phones, a bottom sheet in the
 * who-reacted pattern. Message opens (or creates) your DM with them; for you it
 * says "You" instead.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun MentionCardSheet(
    card: MentionCard,
    message: (done: () -> Unit, failed: (String) -> Unit) -> Unit,
    state: AppUiState? = null,
    viewModel: CaperViewModel? = null,
    onDismiss: () -> Unit,
) {
    var pending by remember(card) { mutableStateOf<String?>(null) }
    var error by remember(card) { mutableStateOf<String?>(null) }
    var choosingSpace by remember(card.id) { mutableStateOf(false) }
    /** Space ID -> "Invited" or "Member", so a sent or redundant invitation can't be retried. */
    var invited by remember(card.id) { mutableStateOf(emptyMap<String, String>()) }
    var status by remember(card.id) { mutableStateOf<String?>(null) }
    var confirmBlock by remember(card.id) { mutableStateOf(false) }
    val id = card.id
    val blocked = id != null && state?.blockedIds?.contains(id) == true
    val accountId = state?.account?.id
    // Don't offer the open space to someone who is already in it.
    val openSpace = state?.selectedSpace?.takeIf { detail -> detail.members.any { it.id == id } }?.space?.id
    val ownedSpaces = state?.spaces.orEmpty().filter { space ->
        accountId != null && space.ownerId == accountId && !space.demo && space.id != openSpace
    }
    val scope = rememberCoroutineScope()
    // Block closes the card first, as on web: only the confirmation stays.
    if (!confirmBlock) ModalBottomSheet(onDismissRequest = onDismiss, containerColor = SurfaceRaised, contentColor = Text) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 24.dp).semantics { paneTitle = card.title },
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Avatar(card.displayName ?: card.username ?: card.title, 48.dp, avatarId = card.avatarId)
            Column {
                Text(
                    card.title, Modifier.semantics { heading() }, color = Text, fontSize = 16.sp,
                    fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
                card.subtitle?.let { Text(it, color = TextMuted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
            if (card.self) Text("You", color = TextMuted, fontSize = 13.sp)
            else {
                if (!blocked && card.username != null) Button(
                    {
                        pending = "message"
                        error = null
                        message({ pending = null; onDismiss() }, { pending = null; error = it })
                    },
                    Modifier.fillMaxWidth().heightIn(min = 48.dp), enabled = pending == null, shape = MaterialTheme.shapes.small, colors = primaryButtonColors(),
                ) { Text(if (pending == "message") "Opening…" else "Message") }
                if (!blocked && card.username != null && ownedSpaces.isNotEmpty() && viewModel != null) {
                    OutlinedButton({ choosingSpace = !choosingSpace }, Modifier.fillMaxWidth().heightIn(min = 48.dp),
                        enabled = pending == null, shape = MaterialTheme.shapes.small) { Text("Invite to space") }
                    if (choosingSpace) Column(Modifier.fillMaxWidth().heightIn(max = 240.dp).verticalScroll(rememberScrollState())) {
                        ownedSpaces.forEach { space ->
                            OutlinedButton({
                                pending = space.id; error = null; status = null
                                viewModel.inviteToSpace(space.id, card.username, {
                                    pending = null; invited = invited + (space.id to "Invited")
                                    status = "Invitation sent to ${space.name}."
                                }, { already -> invited = invited + (space.id to already) }, { pending = null; error = it })
                            }, Modifier.fillMaxWidth().heightIn(min = 48.dp), enabled = pending == null && space.id !in invited,
                                shape = MaterialTheme.shapes.small) { Text(if (pending == space.id) "Sending…" else space.name + (invited[space.id]?.let { " · $it" } ?: "")) }
                        }
                    }
                }
                if (id != null && viewModel != null && state?.account != null) OutlinedButton({
                    if (!blocked) confirmBlock = true
                    else {
                        pending = "unblock"; error = null
                        scope.launch {
                            viewModel.unblock(id, failed = { error = it }).join()
                            pending = null
                            // Unblock keeps the card open and says so, as on web.
                            if (error == null) status = "Unblocked."
                        }
                    }
                }, Modifier.fillMaxWidth().heightIn(min = 48.dp), enabled = pending == null, shape = MaterialTheme.shapes.small) {
                    Text(if (pending == "unblock") "Unblocking…" else if (blocked) "Unblock" else "Block", color = ErrorText)
                }
                status?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 12.sp) }
                error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
            }
        }
    }
    if (confirmBlock && id != null && viewModel != null) BlockConfirmDialog(
        BlockedAccount(id, card.username.orEmpty(), card.displayName ?: card.title, card.avatarId),
        viewModel, onDismiss,
    )
}
