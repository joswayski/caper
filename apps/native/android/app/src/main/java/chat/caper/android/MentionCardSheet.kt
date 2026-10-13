package chat.caper.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.ui.*
import chat.caper.android.model.AppUiState
import chat.caper.android.model.BlockedAccount
import kotlinx.coroutines.launch

/**
 * The card a person's @mention pill opens: on phones, a bottom sheet in the
 * who-reacted pattern. Compose here without leaving the conversation; only Send
 * resolves a DM and opens it after confirmation. For you it says "You" instead.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun MentionCardSheet(
    card: MentionCard,
    message: (done: () -> Unit, failed: (String) -> Unit) -> Unit,
    state: AppUiState? = null,
    viewModel: CaperViewModel? = null,
    onMessageSent: (() -> Unit)? = null,
    onDismiss: () -> Unit,
) {
    var pending by remember(card) { mutableStateOf<String?>(null) }
    var error by remember(card) { mutableStateOf<String?>(null) }
    var choosingSpace by remember(card.id) { mutableStateOf(false) }
    /** Space ID -> "Invited" or "Member", so a sent or redundant invitation can't be retried. */
    var invited by remember(card.id) { mutableStateOf(emptyMap<String, String>()) }
    var status by remember(card.id) { mutableStateOf<String?>(null) }
    var confirmBlock by remember(card.id) { mutableStateOf(false) }
    var profileMenu by remember(card.id) { mutableStateOf(false) }
    var picking by remember(card.id) { mutableStateOf(false) }
    val id = card.id
    val person = card.id ?: "@${card.username?.lowercase(java.util.Locale.ROOT)}"
    val profileDraft = viewModel?.profileDrafts?.state?.collectAsState()?.value?.get(person) ?: ProfileMessageDraft()
    val idle = pending == null && !profileDraft.sending
    val blocked = id != null && state?.blockedIds?.contains(id) == true
    val accountId = state?.account?.id
    // Don't offer the open space to someone who is already in it.
    val openSpace = state?.selectedSpace?.takeIf { detail -> detail.members.any { it.id == id } }?.space?.id
    val ownedSpaces = state?.spaces.orEmpty().filter { space ->
        accountId != null && space.ownerId == accountId && !space.demo && space.id != openSpace
    }
    val scope = rememberCoroutineScope()
    val direct = state?.directConversations?.firstOrNull {
        if (card.id != null) it.peer.id == card.id else it.peer.username.equals(card.username, ignoreCase = true)
    }
    val canMessage = !card.self && !blocked && card.username != null
    fun openConversation() {
        pending = "message"
        error = null
        message({ pending = null; onDismiss() }, { pending = null; error = it })
    }
    // Block closes the card first, as on web: only the confirmation stays.
    if (!confirmBlock) ProfileBottomSheet(card.title, profileDraft, picking || pending != null, onDismiss, header = {
        // Identity, then Open conversation (and the sheet's Close) at the end, as on web.
        Avatar(card.displayName ?: card.username ?: card.title, 48.dp, avatarId = card.avatarId)
        Column(Modifier.weight(1f)) {
            Text(
                card.title, Modifier.semantics { heading() }, color = Text, fontSize = 16.sp,
                fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            card.subtitle?.let { Text(it, color = TextMuted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
        if (!card.self && id != null && viewModel != null && state?.account != null) Box {
            ProfileIconButton(R.drawable.lucide_ellipsis, "More profile actions", idle) { profileMenu = true }
            DropdownMenu(profileMenu, { profileMenu = false }, containerColor = SurfaceRaised) {
                DropdownMenuItem(text = {
                    Text(if (pending == "unblock") "Unblocking…" else if (blocked) "Unblock" else "Block")
                }, enabled = idle, onClick = {
                    profileMenu = false
                    if (!blocked) confirmBlock = true
                    else {
                        pending = "unblock"; error = null
                        scope.launch {
                            viewModel.unblock(id, failed = { error = it }).join()
                            pending = null
                            if (error == null) status = "Unblocked."
                        }
                    }
                })
            }
        }
        if (canMessage) ProfileIconButton(R.drawable.lucide_message_circle, if (pending == "message") "Opening…" else "Open conversation", idle, ::openConversation)
    }) {
        if (card.self) Text("You", color = TextMuted, fontSize = 13.sp)
        else {
            val invite = !blocked && card.username != null && ownedSpaces.isNotEmpty() && viewModel != null
            if (invite) OutlinedButton({ choosingSpace = !choosingSpace }, Modifier.fillMaxWidth().heightIn(min = 48.dp),
                enabled = idle, shape = MaterialTheme.shapes.small) { Text("Invite to space", maxLines = 1, overflow = TextOverflow.Ellipsis) }
            if (invite && choosingSpace && card.username != null && viewModel != null) Column(
                Modifier.fillMaxWidth().heightIn(max = 240.dp).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                ownedSpaces.forEach { space ->
                    OutlinedButton({
                        pending = space.id; error = null; status = null
                        viewModel.inviteToSpace(space.id, card.username, {
                            pending = null; invited = invited + (space.id to "Invited")
                            status = "Invitation sent to ${space.name}."
                        }, { already -> invited = invited + (space.id to already) }, { pending = null; error = it })
                    }, Modifier.fillMaxWidth().heightIn(min = 48.dp), enabled = idle && space.id !in invited,
                        shape = MaterialTheme.shapes.small) { Text(if (pending == space.id) "Sending…" else space.name + (invited[space.id]?.let { " · $it" } ?: "")) }
                }
            }
            status?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 12.sp) }
            error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
            if (canMessage && viewModel != null) {
                HorizontalDivider(color = Border)
                val writable = direct?.blocked != true && direct?.incoming != true
                if (!writable) Text(
                    if (direct?.incoming == true) "Open the conversation and accept this message request before replying."
                    else "Unblock this person to send a message.", color = TextMuted, fontSize = 12.sp,
                )
                ProfileMessageComposer(
                    card.username!!, profileDraft, writable && pending == null, state?.uploadsEnabled == true,
                    text = { viewModel.profileDrafts.text(person, it) },
                    add = { viewModel.addProfileAttachments(person, it) },
                    paste = { viewModel.pasteProfileAttachments(person) },
                    remove = { viewModel.removeProfileAttachment(person, it) },
                    send = { viewModel.sendProfileMessage(person, card.id, card.username, onMessageSent ?: onDismiss) },
                    pickerInteraction = { picking = it },
                )
            }
        }
    }
    if (confirmBlock && id != null && viewModel != null) BlockConfirmDialog(
        BlockedAccount(id, card.username.orEmpty(), card.displayName ?: card.title, card.avatarId),
        viewModel, onDismiss,
    )
}

@Composable private fun ProfileIconButton(icon: Int, label: String, enabled: Boolean, onClick: () -> Unit) {
    IconButton(onClick, Modifier.size(48.dp).semantics { contentDescription = label }, enabled = enabled) {
        Icon(painterResource(icon), null, Modifier.size(20.dp), tint = if (enabled) TextMuted else TextMuted.copy(alpha = 0.4f))
    }
}

/** Drafts survive dismissal; only sends and child pickers keep the sheet open. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun ProfileBottomSheet(
    title: String, draft: ProfileMessageDraft, picking: Boolean, onDismiss: () -> Unit,
    header: @Composable RowScope.() -> Unit = { Spacer(Modifier.weight(1f)) },
    content: @Composable () -> Unit,
) {
    val keepOpen by rememberUpdatedState(draft.sending || picking)
    val sheet = rememberModalBottomSheetState(confirmValueChange = { it != SheetValue.Hidden || !keepOpen })
    val scope = rememberCoroutineScope()
    ModalBottomSheet(
        onDismissRequest = { if (!keepOpen) onDismiss() }, sheetState = sheet,
        // Back is decided below from the current send/picker state.
        properties = ModalBottomSheetProperties(shouldDismissOnBackPress = false),
        containerColor = SurfaceRaised, contentColor = Text,
    ) {
        BackHandler(enabled = !keepOpen) {
            scope.launch { sheet.hide() }.invokeOnCompletion { if (!sheet.isVisible) onDismiss() }
        }
        Column(
            Modifier.fillMaxWidth().swipeCancelsPresses().imePadding().verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp).semantics { paneTitle = title },
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                header()
                ProfileIconButton(R.drawable.lucide_x, "Close profile", !draft.sending, onDismiss)
            }
            content()
        }
    }
}

/**
 * The channel composer's field: Attach (with Paste from the clipboard) at the start, Emoji at the end and Send
 * beside it, with staged files and their thumbnails above. Enter adds a line; only Send sends.
 */
@Composable internal fun ProfileMessageComposer(
    username: String, draft: ProfileMessageDraft, writable: Boolean, uploadsEnabled: Boolean,
    text: (String) -> Unit, add: (List<android.net.Uri>) -> Unit, paste: () -> Unit,
    remove: (String) -> Unit, send: () -> Unit,
    pickerInteraction: (Boolean) -> Unit = {},
) {
    var field by remember { mutableStateOf(TextFieldValue(draft.text, TextRange(draft.text.length))) }
    var emojiPicker by remember { mutableStateOf(false) }
    var filePicker by remember { mutableStateOf(false) }
    LaunchedEffect(draft.text) {
        if (field.text != draft.text) field = TextFieldValue(draft.text, TextRange(draft.text.length))
    }
    val editable = writable && draft.editable
    DraftAttachmentStrip(draft.files, remove, enabled = editable, uploadOnSend = !draft.sending)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
        EmojiTextField(
            field, { value ->
                val limited = if (value.text.codePointCount(0, value.text.length) <= 4000) value else {
                    val limitedText = value.text.substring(0, value.text.offsetByCodePoints(0, 4000))
                    value.copy(text = limitedText, selection = TextRange(value.selection.start.coerceAtMost(limitedText.length), value.selection.end.coerceAtMost(limitedText.length)), composition = null)
                }
                field = limited; text(limited.text)
            }, Modifier.weight(1f).attachmentPaste(editable && uploadsEnabled, add), placeholder = { Text("Message @$username", maxLines = 1, overflow = TextOverflow.Ellipsis) }, maxLines = 6,
            visualTransformation = MentionVisualTransformation(MentionSource(listOf(MentionCandidate(username, username)), specials = false, feedback = true)),
            leadingIcon = {
                AttachButton(
                    when {
                        !uploadsEnabled -> "File uploads are unavailable"
                        draft.files.size >= chat.caper.android.data.AttachmentPolicy.MAX_ATTACHMENTS -> "You can attach up to ${chat.caper.android.data.AttachmentPolicy.MAX_ATTACHMENTS} files."
                        !editable -> "Sending…"
                        else -> null
                    }, add, interaction = { filePicker = it; pickerInteraction(it || emojiPicker) }, paste = paste,
                )
            },
            trailingIcon = {
                EmojiInsertButton(editable, interaction = { emojiPicker = it; pickerInteraction(it || filePicker) }) { emoji ->
                    if (editable) insertEmojiAtSelection(field, emoji)?.let { field = it; text(it.text) }
                }
            },
            enabled = editable, colors = composerFieldColors(),
        )
        FilledIconButton(
            send,
            Modifier.padding(bottom = 4.dp).size(48.dp).semantics { contentDescription = if (draft.uncertain) "Retry send" else "Send to @$username" },
            enabled = writable && draft.canSend, shape = MaterialTheme.shapes.small,
            colors = IconButtonDefaults.filledIconButtonColors(
                containerColor = Action, contentColor = OnAction,
                disabledContainerColor = SurfaceRaised, disabledContentColor = TextMuted,
            ),
        ) { Icon(painterResource(R.drawable.lucide_send), null, Modifier.size(20.dp)) }
    }
    if (draft.sending) Text("Sending…", Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 12.sp)
    draft.error?.let {
        Text(if (draft.uncertain) "Delivery isn’t confirmed. $it Send retries the same message." else it,
            Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp)
    }
}
