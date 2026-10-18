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
import chat.caper.android.data.DEFAULT_INVITE_EXPIRY_NUMBER
import chat.caper.android.data.DEFAULT_INVITE_EXPIRY_UNIT
import chat.caper.android.data.DEFAULT_INVITE_MAX_USES
import chat.caper.android.data.INVALID_INVITE_LINK
import chat.caper.android.data.InviteExpiryUnit
import chat.caper.android.data.InviteOption
import chat.caper.android.data.inviteCode
import chat.caper.android.data.inviteExpirySeconds
import chat.caper.android.data.inviteLinkMeta
import chat.caper.android.data.inviteMaxUses
import chat.caper.android.data.inviteMemberCount
import chat.caper.android.model.AppUiState
import chat.caper.android.model.InvitePreview
import chat.caper.android.model.SpaceDetail
import chat.caper.android.ui.*
import java.time.Instant
import kotlinx.coroutines.delay

/** Opening prepares a link once; revocation never silently replaces it in this dialog. */
@Composable internal fun DefaultInviteLinkSection(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel, initialChannelId: String?) {
    val context = LocalContext.current
    var code by remember(detail.space.id) { mutableStateOf<String?>(null) }
    var preparing by remember(detail.space.id) { mutableStateOf(true) }
    var error by remember(detail.space.id) { mutableStateOf<String?>(null) }
    var copied by remember(detail.space.id) { mutableStateOf(false) }
    var settings by remember(detail.space.id) { mutableStateOf(false) }
    var now by remember { mutableStateOf(Instant.now()) }
    LaunchedEffect(Unit) { while (true) { delay(30_000); now = Instant.now() } }
    fun prepare() {
        preparing = true; error = null; copied = false
        viewModel.ensureDefaultInviteLink(initialChannelId, { code = it.code; preparing = false }) { error = it; preparing = false }
    }
    var started by remember(detail.space.id) { mutableStateOf(false) }
    LaunchedEffect(detail.space.id, state.busy) {
        if (!started && !state.busy) { started = true; prepare() }
    }
    LaunchedEffect(copied) { if (copied) { delay(2_000); copied = false } }
    val link = state.inviteLinks.firstOrNull { it.code == code }
    Text("Invite link", fontSize = 13.sp, fontWeight = FontWeight.Bold)
    Text("Anyone with this link can join this space.", color = TextMuted, fontSize = 11.sp)
    Text("New links allow 100 joins for 30 days.", color = TextMuted, fontSize = 11.sp)
    if (preparing) Text("Preparing link…", color = TextMuted, fontSize = 12.sp)
    if (link != null) {
        Text(viewModel.inviteUrl(link.code), fontFamily = FontFamily.Monospace, fontSize = 12.sp)
        Text(inviteLinkMeta(link, now), color = TextMuted, fontSize = 11.sp)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button({
                context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("Invite link", viewModel.inviteUrl(link.code)))
                copied = true
            }, enabled = !state.busy && !preparing, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { Text(if (copied) "Copied" else "Copy") }
            InviteLinkActions(!state.busy && !preparing, {
                error = null
                viewModel.regenerateInviteLink(link, { code = it.code; copied = false }) { error = it }
            }, { settings = !settings }, { error = null; viewModel.revokeInviteLink(link) { error = it } })
        }
    } else if (!preparing) {
        Text("No active invite link.", color = TextMuted, fontSize = 12.sp)
        Button({ prepare() }, enabled = !state.busy, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { Text("Create another link") }
        TextButton({ settings = !settings }, enabled = !state.busy, colors = neutralTextButtonColors()) { Text(if (settings) "Hide custom links" else "Custom links") }
    }
    error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
    if (settings) InviteLinkSection(if (preparing) state.copy(busy = true) else state, detail, viewModel)
}

/** The link's actions, immediately beside Copy; no space administration actions. */
@Composable internal fun InviteLinkActions(enabled: Boolean, regenerate: () -> Unit, settings: () -> Unit, deactivate: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    var confirmation by remember { mutableStateOf<String?>(null) }
    confirmation?.let { action ->
        ConfirmDialog("$action?", "The previous URL stops admitting new members." +
            if (action == "Regenerate link") " The new link resets to 30 days and 100 joins." else "",
            action, false, { confirmation = null }, focusCancel = true) {
            confirmation = null
            if (enabled) { if (action == "Regenerate link") regenerate() else deactivate() }
        }
    }
    Box {
        OutlinedButton({ open = true }, Modifier.semantics { contentDescription = "More link options" }, enabled = enabled,
            shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border), colors = ButtonDefaults.outlinedButtonColors(contentColor = Text)) {
            Text("…", fontSize = 18.sp)
        }
        DropdownMenu(open && enabled, { open = false }, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
            DropdownMenuItem(text = { Column {
                Text("Regenerate link", fontSize = 13.sp)
                Text("Previous link stops working. Resets to 30 days / 100 joins.", color = TextMuted, fontSize = 11.sp)
            } }, onClick = { open = false; confirmation = "Regenerate link" })
            DropdownMenuItem(text = { Text("Custom links", fontSize = 13.sp) }, onClick = { open = false; settings() })
            DropdownMenuItem(text = { Text("Deactivate link", color = ErrorText, fontSize = 13.sp) }, onClick = { open = false; confirmation = "Deactivate link" })
        }
    }
}

@Composable internal fun InviteLinkSettingsDisclosure(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel, enabled: Boolean = !state.busy) {
    var settings by remember(detail.space.id) { mutableStateOf(false) }
    TextButton({ settings = !settings }, enabled = enabled, colors = neutralTextButtonColors()) { Text(if (settings) "Hide custom links" else "Custom links") }
    if (settings) InviteLinkSection(state, detail, viewModel)
}

/**
 * Manage space's "Invite link" section (web's InviteLinkManager): expiry, use limit and landing
 * channel, Create link, then the live links with Copy and Revoke.
 */
@Composable internal fun InviteLinkSection(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel) {
    val context = LocalContext.current
    val clipboard = remember(context) { context.getSystemService(ClipboardManager::class.java) }
    // Links can only open public channels.
    val publicChannels = detail.channels.filter { !it.private && !it.direct }
    // Typed text; parsed and checked on Create link.
    var expiryNumber by remember(detail.space.id) { mutableStateOf(DEFAULT_INVITE_EXPIRY_NUMBER) }
    var expiryUnit by remember(detail.space.id) { mutableStateOf(DEFAULT_INVITE_EXPIRY_UNIT) }
    var maxUsesText by remember(detail.space.id) { mutableStateOf(DEFAULT_INVITE_MAX_USES) }
    var expiryError by remember(detail.space.id) { mutableStateOf<String?>(null) }
    var maxUsesError by remember(detail.space.id) { mutableStateOf<String?>(null) }
    var channelId by remember(detail.space.id) {
        mutableStateOf(publicChannels.firstOrNull()?.id)
    }
    var error by remember(detail.space.id) { mutableStateOf<String?>(null) }
    var loaded by remember(detail.space.id) { mutableStateOf(false) }
    var copied by remember(detail.space.id) { mutableStateOf<String?>(null) }
    // Which request is running, so only that button says so.
    var creating by remember { mutableStateOf(false) }
    var revoking by remember { mutableStateOf<String?>(null) }
    var revokeTarget by remember(detail.space.id) { mutableStateOf<chat.caper.android.model.InviteLink?>(null) }
    revokeTarget?.let { link ->
        ConfirmDialog("Revoke link?", "The previous URL stops admitting new members.",
            "Revoke", false, { revokeTarget = null }, focusCancel = true) {
            revokeTarget = null
            if (!state.busy) { error = null; revoking = link.code; viewModel.revokeInviteLink(link) { error = it } }
        }
    }
    LaunchedEffect(detail.space.id) { viewModel.loadInviteLinks({ loaded = true }) { error = it; loaded = true } }
    LaunchedEffect(state.busy) { if (!state.busy) { creating = false; revoking = null } }
    LaunchedEffect(copied) { if (copied != null) { delay(2_000); copied = null } }
    // A channel deleted or made private meanwhile falls back to the oldest public channel.
    val landing = publicChannels.firstOrNull { it.id == channelId } ?: publicChannels.firstOrNull()
    fun copy(code: String) {
        clipboard?.setPrimaryClip(ClipData.newPlainText("Invite link", viewModel.inviteUrl(code)))
        copied = code
    }
    // Out-of-range or malformed fields show their own error and send nothing.
    fun create() {
        val expiry = inviteExpirySeconds(expiryNumber, expiryUnit)
        val uses = inviteMaxUses(maxUsesText)
        expiryError = (expiry as? InviteOption.Invalid)?.error
        maxUsesError = (uses as? InviteOption.Invalid)?.error
        if (expiry !is InviteOption.Valid || uses !is InviteOption.Valid) return
        error = null; creating = true
        viewModel.createInviteLink(landing?.id, expiry.value, uses.value, {}) { error = it }
    }

    Text("Create a custom link", fontSize = 13.sp, fontWeight = FontWeight.Bold)
    Text("Anyone with the link can join this space.", color = TextMuted, fontSize = 11.sp)
    Text("Create a separate link. Existing links keep their settings.", color = TextMuted, fontSize = 11.sp)
    InviteRow("Expire after") {
        // "Never" has no number; the typed one comes back with another unit.
        if (expiryUnit != InviteExpiryUnit.Never) {
            InviteNumberField("Expire after", expiryNumber, null, expiryError != null, !state.busy, { create() }) {
                expiryNumber = it; expiryError = null; error = null
            }
            Spacer(Modifier.width(8.dp))
        }
        InviteMenu("Expire after unit", InviteExpiryUnit.entries.map { it to it.label }, expiryUnit, !state.busy) {
            expiryUnit = it; expiryError = null; error = null
        }
    }
    InviteFieldError(expiryError)
    InviteRow("Max uses") {
        InviteNumberField("Max uses", maxUsesText, "No limit", maxUsesError != null, !state.busy, { create() }) {
            maxUsesText = it; maxUsesError = null; error = null
        }
    }
    InviteFieldError(maxUsesError)
    InviteChoice(
        "Opens in", publicChannels.map { (it.id as String?) to "#${it.name}" }.ifEmpty { listOf(null to "No public channels") },
        landing?.id, !state.busy && publicChannels.isNotEmpty(),
    ) { channelId = it; error = null }
    Button({ create() }, enabled = !state.busy, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { PendingLabel("Create link", "Creating…", state.busy && creating) }
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
                TextButton({ copy(link.code) }, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, enabled = !state.busy, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) {
                    PendingLabel("Copy", "Copied", copied == link.code)
                }
                TextButton({ revokeTarget = link }, enabled = !state.busy,
                    shape = MaterialTheme.shapes.small, colors = ButtonDefaults.textButtonColors(contentColor = ErrorText)) {
                    PendingLabel("Revoke", "Revoking…", state.busy && revoking == link.code)
                }
            }
        }
    }
}

/** A labelled picker: the current choice on a quiet button, the options in a menu. */
@Composable private fun <T> InviteChoice(label: String, options: List<Pair<T, String>>, current: T, enabled: Boolean, choose: (T) -> Unit) {
    InviteRow(label) { InviteMenu(label, options, current, enabled, choose) }
}

/** An option's muted label on the left, its controls on the right. */
@Composable private fun InviteRow(label: String, controls: @Composable RowScope.() -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f), color = TextMuted, fontSize = 12.sp)
        controls()
    }
}

/** A short whole-number field on the number keyboard; Done creates the link. */
@Composable private fun InviteNumberField(
    label: String, value: String, placeholder: String?, invalid: Boolean, enabled: Boolean, done: () -> Unit, change: (String) -> Unit,
) {
    OutlinedTextField(
        value, change, Modifier.width(112.dp).semantics { contentDescription = label }, enabled = enabled, singleLine = true,
        textStyle = LocalTextStyle.current.copy(fontSize = 13.sp), isError = invalid,
        placeholder = if (placeholder != null) { { Text(placeholder, fontSize = 13.sp, maxLines = 1) } } else null,
        keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Number, imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { if (enabled) done() }),
    )
}

/** A field's inline error, under its row. */
@Composable private fun InviteFieldError(error: String?) {
    error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
}

/** The current choice on a quiet button, the options in a menu. */
@Composable private fun <T> InviteMenu(label: String, options: List<Pair<T, String>>, current: T, enabled: Boolean, choose: (T) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val shown = options.firstOrNull { it.first == current }?.second ?: options.first().second
    Box {
        OutlinedButton({ open = true }, Modifier.widthIn(max = 240.dp).semantics { contentDescription = "$label, $shown" }, enabled = enabled,
            shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border), colors = ButtonDefaults.outlinedButtonColors(contentColor = Text)) {
            Text(shown, Modifier.weight(1f, fill = false), fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Spacer(Modifier.width(6.dp))
            Icon(painterResource(R.drawable.lucide_chevron_down), null, Modifier.size(14.dp), tint = TextMuted)
        }
        DropdownMenu(open && enabled, { open = false }, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
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

/**
 * "Join with invite link": a pasted link (any host), `caper.chat/invite/<code>` or a bare code,
 * previewed before joining. Joining opens the space at the link's channel. A link [opened] from
 * another app starts filled in and loads its preview at once, as if Continue was pressed.
 */
@Composable internal fun JoinInviteDialog(busy: Boolean, viewModel: CaperViewModel, opened: InviteLinkRequest? = null, close: () -> Unit) {
    var input by remember { mutableStateOf(opened?.url.orEmpty()) }
    var error by remember { mutableStateOf<String?>(null) }
    var checking by remember { mutableStateOf(false) }
    var code by remember { mutableStateOf<String?>(null) }
    var preview by remember { mutableStateOf<InvitePreview?>(null) }
    fun submit() {
        val parsed = inviteCode(input)
        if (parsed == null) { error = INVALID_INVITE_LINK; return }
        error = null; checking = true
        viewModel.previewInvite(parsed, { code = parsed; preview = it; checking = false }) { error = it; checking = false }
    }
    LaunchedEffect(Unit) { if (opened != null) submit() }
    CaperDialog(if (preview?.alreadyMember == true) "Open space" else "Join with invite link", close, busy = busy || checking) {
        val shown = preview
        val shownCode = code
        if (shown == null || shownCode == null) {
            val field = remember { FocusRequester() }
            // An opened link is already loading; the keyboard would only cover its preview.
            LaunchedEffect(Unit) { if (opened == null) runCatching { field.requestFocus() } }
            Text("Paste it below to join!", color = TextMuted, fontSize = 12.sp)
            OutlinedTextField(input, { input = it; error = null }, placeholder = {
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("it looks like", color = TextMuted.copy(alpha = 0.65f), fontSize = 12.sp)
                    Icon(painterResource(R.drawable.lucide_arrow_right), null, Modifier.size(12.dp), tint = TextMuted.copy(alpha = 0.65f))
                    Text("caper.chat/invite/AbCdE12345", color = TextMuted.copy(alpha = 0.65f), fontSize = 12.sp, maxLines = 1)
                }
            },
                modifier = Modifier.fillMaxWidth().focusRequester(field).semantics { contentDescription = "Invite link" }, singleLine = true, readOnly = checking,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                keyboardActions = KeyboardActions(onGo = { if (!checking && input.isNotBlank()) submit() }))
            error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(close, enabled = !checking, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Cancel") }; Spacer(Modifier.width(8.dp))
                Button({ submit() }, enabled = !checking && input.isNotBlank(), shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { PendingLabel("Continue", "Checking…", checking) }
            }
        } else {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Surface(Modifier.size(48.dp), color = SurfaceRaised, shape = MaterialTheme.shapes.medium, border = BorderStroke(1.dp, Border)) {
                    SpaceTileFace(shown.space.name, shown.space.iconAsset, shown.space.iconUrl, 48.dp, TextMuted)
                }
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(shown.space.name, fontSize = 18.sp, fontWeight = FontWeight.Bold)
                    if (shown.alreadyMember) Text("You’re already a member", color = TextMuted, fontSize = 12.sp)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Avatar(shown.inviter.displayName, 16.dp, avatarId = shown.inviter.avatarId,
                            avatarAsset = shown.inviter.avatarAsset, avatarUrl = shown.inviter.avatarUrl)
                        Spacer(Modifier.width(6.dp))
                        Text("Invited by ${shown.inviter.displayName}", modifier = Modifier.weight(1f, fill = false),
                            color = TextMuted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (shown.inviter.admin) {
                            Spacer(Modifier.width(6.dp))
                            AdminBadge()
                        }
                    }
                    Text(listOfNotNull(inviteMemberCount(shown.memberCount), shown.channel?.let { "Opens in #${it.name}" }).joinToString(" · "),
                        color = TextMuted, fontSize = 12.sp)
                }
            }
            error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(close, enabled = !busy, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Cancel") }; Spacer(Modifier.width(8.dp))
                Button({ error = null; viewModel.joinInvite(shownCode, close) { error = it } }, enabled = !busy, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) {
                    PendingLabel(if (shown.alreadyMember) "Open space" else "Join space", if (shown.alreadyMember) "Opening…" else "Joining…", busy)
                }
            }
        }
    }
}
