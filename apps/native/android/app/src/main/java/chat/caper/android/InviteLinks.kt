package chat.caper.android

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.DEFAULT_INVITE_MAX_AGE
import chat.caper.android.data.INVALID_INVITE_LINK
import chat.caper.android.data.INVITE_MAX_AGES
import chat.caper.android.data.INVITE_MAX_USES
import chat.caper.android.data.inviteCode
import chat.caper.android.data.inviteLinkMeta
import chat.caper.android.data.inviteMemberCount
import chat.caper.android.model.AppUiState
import chat.caper.android.model.InvitePreview
import chat.caper.android.model.SpaceDetail
import chat.caper.android.ui.*
import java.time.Instant
import kotlinx.coroutines.delay

/**
 * Manage space's "Invite link" section (web's InviteLinkManager): expiry, use limit and landing
 * channel, Create link (copied at once), then the live links with Copy and Revoke.
 */
@Composable internal fun InviteLinkSection(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel) {
    val context = LocalContext.current
    val clipboard = remember(context) { context.getSystemService(ClipboardManager::class.java) }
    // Links can only open public channels.
    val publicChannels = detail.channels.filter { !it.private && !it.direct }
    var maxAge by remember(detail.space.id) { mutableStateOf<Int?>(DEFAULT_INVITE_MAX_AGE) }
    var maxUses by remember(detail.space.id) { mutableStateOf<Int?>(null) }
    var channelId by remember(detail.space.id) {
        mutableStateOf(state.selectedChannel?.takeIf { it.spaceId == detail.space.id && !it.private && !it.direct }?.id)
    }
    var error by remember(detail.space.id) { mutableStateOf<String?>(null) }
    var loaded by remember(detail.space.id) { mutableStateOf(false) }
    var copied by remember(detail.space.id) { mutableStateOf<String?>(null) }
    // Which request is running, so only that button says so.
    var creating by remember { mutableStateOf(false) }
    var revoking by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(detail.space.id) { viewModel.loadInviteLinks({ loaded = true }) { error = it; loaded = true } }
    LaunchedEffect(state.busy) { if (!state.busy) { creating = false; revoking = null } }
    LaunchedEffect(copied) { if (copied != null) { delay(2_000); copied = null } }
    // A channel deleted or made private meanwhile falls back to the default.
    val landing = publicChannels.firstOrNull { it.id == channelId }
    fun copy(code: String) {
        clipboard?.setPrimaryClip(ClipData.newPlainText("Invite link", viewModel.inviteUrl(code)))
        copied = code
    }

    Text("Invite link", fontSize = 13.sp, fontWeight = FontWeight.Bold)
    Text("Anyone with the link can join this space.", color = TextMuted, fontSize = 11.sp)
    InviteChoice("Expire after", INVITE_MAX_AGES, maxAge, !state.busy) { maxAge = it; error = null }
    InviteChoice("Max uses", INVITE_MAX_USES, maxUses, !state.busy) { maxUses = it; error = null }
    InviteChoice(
        "Opens in", listOf<Pair<String?, String>>(null to "Default channel") + publicChannels.map { it.id to "#${it.name}" },
        landing?.id, !state.busy,
    ) { channelId = it; error = null }
    Button({
        error = null; creating = true
        viewModel.createInviteLink(landing?.id, maxAge, maxUses, { copy(it.code) }) { error = it }
    }, enabled = !state.busy, shape = MaterialTheme.shapes.small) { PendingLabel("Create link", "Creating…", state.busy && creating) }
    error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
    val now = Instant.now()
    if (loaded && state.inviteLinks.isEmpty()) Text("No active invite links.", color = TextMuted, fontSize = 11.sp)
    state.inviteLinks.forEach { link ->
        key(link.code) {
            Row(Modifier.fillMaxWidth().heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(viewModel.inviteUrl(link.code), fontFamily = FontFamily.Monospace, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(inviteLinkMeta(link, now), color = TextMuted, fontSize = 10.sp)
                }
                TextButton({ copy(link.code) }, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, shape = MaterialTheme.shapes.small) {
                    PendingLabel("Copy", "Copied", copied == link.code)
                }
                TextButton({ error = null; revoking = link.code; viewModel.revokeInviteLink(link) { error = it } }, enabled = !state.busy,
                    shape = MaterialTheme.shapes.small, colors = ButtonDefaults.textButtonColors(contentColor = ErrorText)) {
                    PendingLabel("Revoke", "Revoking…", state.busy && revoking == link.code)
                }
            }
        }
    }
}

/** A labelled picker: the current choice on a quiet button, the options in a menu. */
@Composable private fun <T> InviteChoice(label: String, options: List<Pair<T, String>>, current: T, enabled: Boolean, choose: (T) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val shown = options.firstOrNull { it.first == current }?.second ?: options.first().second
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f), color = TextMuted, fontSize = 12.sp)
        Box {
            OutlinedButton({ open = true }, Modifier.widthIn(max = 240.dp).semantics { contentDescription = "$label, $shown" }, enabled = enabled,
                shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border), colors = ButtonDefaults.outlinedButtonColors(contentColor = Text)) {
                Text(shown, Modifier.weight(1f, fill = false), fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.width(6.dp))
                Icon(painterResource(R.drawable.lucide_chevron_down), null, Modifier.size(14.dp), tint = TextMuted)
            }
            DropdownMenu(open, { open = false }, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                options.forEach { (option, text) ->
                    DropdownMenuItem(
                        text = { Text(text, fontSize = 13.sp) }, onClick = { open = false; choose(option) },
                        modifier = Modifier.semantics { selected = option == current },
                        leadingIcon = { RadioButton(option == current, null) },
                    )
                }
            }
        }
    }
}

/**
 * "Join with invite link": a pasted link (any host), `caper.chat/invite/<code>` or a bare code,
 * previewed before joining. Joining opens the space at the link's channel.
 */
@Composable internal fun JoinInviteDialog(busy: Boolean, viewModel: CaperViewModel, close: () -> Unit) {
    var input by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var checking by remember { mutableStateOf(false) }
    var code by remember { mutableStateOf<String?>(null) }
    var preview by remember { mutableStateOf<InvitePreview?>(null) }
    CaperDialog("Join with invite link", close, busy = busy || checking) {
        val shown = preview
        val shownCode = code
        if (shown == null || shownCode == null) {
            val field = remember { FocusRequester() }
            LaunchedEffect(Unit) { runCatching { field.requestFocus() } }
            fun submit() {
                val parsed = inviteCode(input)
                if (parsed == null) { error = INVALID_INVITE_LINK; return }
                error = null; checking = true
                viewModel.previewInvite(parsed, { code = parsed; preview = it; checking = false }) { error = it; checking = false }
            }
            OutlinedTextField(input, { input = it; error = null }, label = { Text("Invite link") }, placeholder = { Text("https://caper.chat/invite/…") },
                modifier = Modifier.fillMaxWidth().focusRequester(field), singleLine = true, readOnly = checking,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                keyboardActions = KeyboardActions(onGo = { if (!checking && input.isNotBlank()) submit() }))
            error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(close, enabled = !checking, shape = MaterialTheme.shapes.small) { Text("Cancel") }; Spacer(Modifier.width(8.dp))
                Button({ submit() }, enabled = !checking && input.isNotBlank(), shape = MaterialTheme.shapes.small) { PendingLabel("Continue", "Checking…", checking) }
            }
        } else {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Surface(Modifier.size(48.dp), color = SurfaceRaised, shape = MaterialTheme.shapes.medium, border = BorderStroke(1.dp, Border)) {
                    Box(contentAlignment = Alignment.Center) { Text(initialOf(shown.space.name), fontWeight = FontWeight.Black, color = TextMuted) }
                }
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(shown.space.name, fontSize = 18.sp, fontWeight = FontWeight.Bold)
                    Text("Invited by ${shown.inviter.displayName}", color = TextMuted, fontSize = 12.sp)
                    Text(listOfNotNull(inviteMemberCount(shown.memberCount), shown.channel?.let { "Opens in #${it.name}" }).joinToString(" · "),
                        color = TextMuted, fontSize = 12.sp)
                }
            }
            error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(close, enabled = !busy, shape = MaterialTheme.shapes.small) { Text("Cancel") }; Spacer(Modifier.width(8.dp))
                Button({ error = null; viewModel.joinInvite(shownCode, close) { error = it } }, enabled = !busy, shape = MaterialTheme.shapes.small) {
                    PendingLabel("Join space", "Joining…", busy)
                }
            }
        }
    }
}
