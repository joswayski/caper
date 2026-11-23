package chat.caper.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.DELETE_ACCOUNT_URL
import chat.caper.android.data.blockedRunLabel
import chat.caper.android.data.directPrivacyOptions
import chat.caper.android.data.friendlyError
import chat.caper.android.model.*
import chat.caper.android.ui.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** The sidebar's "Message requests" row (with the request count) and, when open, the requests list. */
@Composable internal fun MessageRequestsSection(
    requests: List<DirectConversation>,
    selectedId: String?,
    open: Boolean,
    toggle: () -> Unit,
    select: (DirectConversation) -> Unit,
) {
    if (requests.isEmpty()) return
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp).heightIn(min = 44.dp).clip(MaterialTheme.shapes.small)
            .clickable(role = Role.Button, onClickLabel = if (open) "Hide message requests" else "Show message requests", onClick = toggle)
            .semantics(mergeDescendants = true) { contentDescription = "Message requests, ${requests.size}" }
            .padding(horizontal = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // In the avatars' column, turning like the Channels chevron (web).
        Box(Modifier.width(26.dp), contentAlignment = Alignment.Center) { DisclosureChevron(open, Modifier.size(16.dp)) }
        Spacer(Modifier.width(9.dp))
        Text("Message requests", Modifier.weight(1f), color = Text, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
        // A neutral count of conversations: requests never add to unread dots.
        Surface(color = SurfaceRaised, shape = CircleShape, border = BorderStroke(1.dp, Border)) {
            Text(requests.size.toString(), Modifier.padding(horizontal = 7.dp, vertical = 1.dp), color = TextMuted, fontSize = 11.sp, fontWeight = FontWeight.Bold, style = TabularNumbers)
        }
    }
    if (open) requests.forEach { request ->
        val selected = request.id == selectedId
        Row(
            Modifier.fillMaxWidth().padding(start = 28.dp, end = 16.dp).heightIn(min = 44.dp).clip(MaterialTheme.shapes.small)
                .background(if (selected) TerracottaWash else androidx.compose.ui.graphics.Color.Transparent)
                .clickable(role = Role.Button) { select(request) }.semantics { if (selected) this.selected = true }.padding(horizontal = 9.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Avatar(request.peer.displayName, 20.dp, modifier = Modifier.padding(horizontal = 3.dp), avatarId = request.peer.avatarId,
                avatarAsset = request.peer.avatarAsset, avatarUrl = request.peer.avatarUrl)
            Spacer(Modifier.width(9.dp))
            Column(Modifier.weight(1f)) {
                Text(request.peer.displayName, color = if (selected) Text else TextMuted, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text("@${request.peer.username}", color = TextMuted, fontSize = 10.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}

/** Replaces the composer in an incoming request: Accept, Decline or Block, with errors inline. */
@Composable internal fun RequestBar(request: DirectConversation, viewModel: CaperViewModel, left: () -> Unit) {
    var working by remember(request.id) { mutableStateOf(false) }
    var error by remember(request.id) { mutableStateOf<String?>(null) }
    var confirmBlock by remember(request.id) { mutableStateOf(false) }
    val failed: (String) -> Unit = { working = false; error = it }
    val peer = request.peer
    Column(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(buildAnnotatedString {
            withStyle(SpanStyle(fontWeight = FontWeight.Bold, color = Text)) { append(peer.displayName) }
            append(" (@${peer.username}) wants to message you. You don't share a space.")
        }, color = TextMuted, fontSize = 13.sp)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Button({ working = true; error = null; viewModel.acceptRequest(request) { failed(it) } }, Modifier.heightIn(min = 48.dp), enabled = !working, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { Text("Accept") }
            OutlinedButton({ working = true; error = null; viewModel.declineRequest(request, left) { failed(it) } }, Modifier.heightIn(min = 48.dp), enabled = !working,
                shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) { Text("Decline", color = Text) }
            if (peer.canBlock) TextButton({ confirmBlock = true }, Modifier.heightIn(min = 48.dp), enabled = !working, shape = MaterialTheme.shapes.small) { Text("Block", color = ErrorText) }
        }
        error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
    }
    if (confirmBlock) BlockConfirmDialog(BlockedAccount(peer.id, peer.username, peer.displayName, peer.avatarId, peer.avatarAsset, peer.avatarUrl, peer.admin, peer.team), viewModel, { confirmBlock = false }) { left() }
}

/** Replaces the composer in a DM whose peer you blocked. */
@Composable internal fun BlockedDirectNotice(conversation: DirectConversation, viewModel: CaperViewModel) {
    var working by remember(conversation.id) { mutableStateOf(false) }
    var error by remember(conversation.id) { mutableStateOf<String?>(null) }
    Column(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("You blocked @${conversation.peer.username}.", Modifier.weight(1f), color = TextMuted, fontSize = 13.sp)
            OutlinedButton({ working = true; error = null; viewModel.unblock(conversation.peer.id) { working = false; error = it } },
                Modifier.heightIn(min = 48.dp), enabled = !working, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) { Text("Unblock", color = Text) }
        }
        error?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
    }
}

/** The quiet notice above the composer while your request waits. */
@Composable internal fun OutgoingRequestNotice(conversation: DirectConversation) = Text(
    "Waiting for @${conversation.peer.username} to accept. They'll see your messages when they do.",
    Modifier.fillMaxWidth().padding(bottom = 8.dp), color = TextMuted, fontSize = 12.sp,
)

/** Confirms a block; Cancel or a failure leaves everything as it was. */
@Composable internal fun BlockConfirmDialog(account: BlockedAccount, viewModel: CaperViewModel, close: () -> Unit, blocked: () -> Unit = {}) {
    var working by remember(account.id) { mutableStateOf(false) }
    var error by remember(account.id) { mutableStateOf<String?>(null) }
    val name = account.displayName.ifBlank { "@${account.username}" }
    // As web and ConfirmDialog: Cancel takes the initial focus, so Enter never confirms.
    val cancelFocus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { cancelFocus.requestFocus() } }
    CaperDialog("Block $name?", { if (!working) close() }) {
        Text("You won't see their messages unless you choose to, and they can't send you DMs or requests.", color = TextMuted)
        error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            TextButton(close, Modifier.focusRequester(cancelFocus), enabled = !working, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Cancel") }
            Spacer(Modifier.width(8.dp))
            Button(
                { working = true; error = null; viewModel.block(account, { close(); blocked() }) { working = false; error = it } },
                enabled = !working, shape = MaterialTheme.shapes.small, colors = ButtonDefaults.buttonColors(containerColor = Danger),
            ) { PendingLabel("Block", "Blocking…", working) }
        }
    }
}

/** Web's Ban icon, "N blocked messages — Show", or Hide above a revealed run; the icon sits in the avatar column. */
@Composable internal fun BlockedRunRow(count: Int, revealed: Boolean, toggle: () -> Unit) = Row(
    Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(start = 27.dp, end = 18.dp),
    verticalAlignment = Alignment.CenterVertically,
) {
    Icon(painterResource(R.drawable.lucide_ban), null, Modifier.size(16.dp), tint = TextMuted)
    Spacer(Modifier.width(8.dp))
    Text(blockedRunLabel(count), Modifier.weight(1f, fill = false), color = TextMuted, fontSize = 12.sp)
    // Decorative, as on web (aria-hidden): TalkBack reads the count, then Show or Hide.
    Text(" — ", Modifier.clearAndSetSemantics {}, color = TextMuted, fontSize = 12.sp)
    TextButton(toggle, Modifier.heightIn(min = 48.dp).semantics { contentDescription = "${if (revealed) "Hide" else "Show"} ${blockedRunLabel(count)}" },
        shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors(TextMuted)) {
        Text(if (revealed) "Hide" else "Show", fontSize = 12.sp)
    }
}

/** User settings: appearance/themes, who can start a DM with you, the accounts you blocked, and how to delete your account. */
@Composable internal fun PrivacySettingsDialog(state: AppUiState, viewModel: CaperViewModel, close: () -> Unit) = CaperDialog("Settings & Privacy", close) {
    val scope = rememberCoroutineScope()
    val context = androidx.compose.ui.platform.LocalContext.current
    val activeTheme = CaperThemeStore.active

    SettingLabel("Appearance")
    Text(
        "Choose between Obsidian Dark and Paper Light.",
        color = activeTheme.textMuted,
        fontSize = 12.sp,
    )
    Row(Modifier.fillMaxWidth().selectableGroup(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        ThemePresetCard(
            preset = CaperThemeStore.obsidian,
            selected = activeTheme.id != "paper",
            activeTheme = activeTheme,
            modifier = Modifier.weight(1f),
            onClick = { CaperThemeStore.selectPreset(context, "obsidian") },
        )
        ThemePresetCard(
            preset = CaperThemeStore.paper,
            selected = activeTheme.id == "paper",
            activeTheme = activeTheme,
            modifier = Modifier.weight(1f),
            onClick = { CaperThemeStore.selectPreset(context, "paper") },
        )
    }
    HorizontalDivider(color = activeTheme.border)

    var setting by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    var settingError by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    LaunchedEffect(attempt) {
        settingError = null
        try { setting = viewModel.directPrivacy() }
        catch (error: CancellationException) { throw error }
        catch (error: Throwable) { settingError = friendlyError(error, "Couldn’t load this setting.") }
    }
    LaunchedEffect(Unit) { viewModel.refreshBlocks() }

    SettingLabel("Who can start a DM with you")
    Column(Modifier.selectableGroup()) {
        directPrivacyOptions.forEach { (value, label, detail) ->
            Row(
                // Only disabled while loading: disabling during a save would drop focus from the chosen option.
                Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(MaterialTheme.shapes.small).selectable(setting == value, enabled = setting != null, role = Role.RadioButton) {
                    val previous = setting
                    if (saving || previous == value) return@selectable
                    setting = value; saving = true; settingError = null
                    scope.launch {
                        try { setting = viewModel.setDirectPrivacy(value) }
                        catch (error: CancellationException) { throw error }
                        // Revert and explain when the save fails.
                        catch (error: Throwable) { setting = previous; settingError = friendlyError(error, "Couldn’t save this setting.") }
                        finally { saving = false }
                    }
                },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RadioButton(setting == value, null, enabled = setting != null)
                Spacer(Modifier.width(8.dp))
                Column(Modifier.weight(1f)) {
                    Text(label, fontSize = 13.sp)
                    detail?.let { Text(it, color = TextMuted, fontSize = 11.sp) }
                }
            }
        }
    }
    if (setting == null && settingError == null) Text("Loading…", color = TextMuted, fontSize = 12.sp)
    settingError?.let {
        Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp)
        if (setting == null) TextButton({ attempt++ }, shape = MaterialTheme.shapes.small) { Text("Retry") }
    }

    HorizontalDivider(color = Border)
    SettingLabel("Blocked accounts")
    var unblockError by remember { mutableStateOf<String?>(null) }
    // "None" only once the list has loaded, and never alongside a load error.
    if (state.blocks.isEmpty()) Text(when {
        state.blocksError != null -> "Couldn’t load blocked accounts."
        state.blocksLoaded -> "You haven't blocked anyone."
        else -> "Loading…"
    }, color = TextMuted, fontSize = 12.sp)
    state.blocks.forEach { account ->
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(account.displayName.ifBlank { account.username }, 28.dp, avatarId = account.avatarId, avatarAsset = account.avatarAsset, avatarUrl = account.avatarUrl)
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f)) {
                Text(account.displayName.ifBlank { "@${account.username}" }, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (account.username.isNotBlank()) Text("@${account.username}", color = TextMuted, fontSize = 11.sp, maxLines = 1)
            }
            OutlinedButton({ unblockError = null; viewModel.unblock(account.id) { unblockError = it } }, Modifier.heightIn(min = 48.dp),
                shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) { Text("Unblock", color = Text) }
        }
    }
    (unblockError ?: state.blocksError?.takeIf { state.blocks.isNotEmpty() })?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
    if (state.blocksError != null) TextButton(viewModel::refreshBlocks, shape = MaterialTheme.shapes.small) { Text("Retry") }

    HorizontalDivider(color = Border)
    val uriHandler = LocalUriHandler.current
    TextButton({ runCatching { uriHandler.openUri(DELETE_ACCOUNT_URL) } }, Modifier.heightIn(min = 48.dp),
        shape = MaterialTheme.shapes.small, colors = ButtonDefaults.textButtonColors(contentColor = ErrorText)) {
        Text("Delete account", fontSize = 13.sp)
        Spacer(Modifier.width(4.dp))
        Icon(painterResource(R.drawable.lucide_external_link), null, Modifier.size(12.dp))
    }
}

@Composable private fun ThemePresetCard(
    preset: CaperThemePalette,
    selected: Boolean,
    activeTheme: CaperThemePalette,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    val cardShape = RoundedCornerShape(13.dp)
    val swatchShape = RoundedCornerShape(9.dp)
    val convoColor = if (preset.isLight) Color.White else preset.surface
    val sidebarColor = if (preset.isLight) preset.blackout else Color(0xFF0F0F11)
    val borderColor = if (selected) activeTheme.accentBright else activeTheme.border
    val borderWidth = if (selected) 1.5.dp else 1.dp

    Column(
        modifier
            .clip(cardShape)
            .background(activeTheme.surfaceRaised, cardShape)
            .border(borderWidth, borderColor, cardShape)
            .selectable(selected = selected, role = Role.RadioButton, onClick = onClick)
            .semantics(mergeDescendants = true) {
                contentDescription = "${preset.name} (${if (preset.isLight) "Light" else "Dark"})"
                this.selected = selected
            }
            .padding(11.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .height(54.dp)
                .clip(swatchShape)
                .background(preset.blackout, swatchShape)
                .border(1.dp, Color(0x3D808080), swatchShape)
                .padding(6.dp),
            horizontalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Column(
                Modifier
                    .weight(0.28f)
                    .fillMaxHeight()
                    .clip(RoundedCornerShape(5.dp))
                    .background(sidebarColor)
                    .padding(5.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Box(Modifier.fillMaxWidth(0.78f).height(4.5.dp).clip(CircleShape).background(preset.caperGreen.copy(alpha = 0.9f)))
                Box(Modifier.fillMaxWidth(0.56f).height(4.5.dp).clip(CircleShape).background(preset.textMuted.copy(alpha = 0.72f)))
                Box(Modifier.fillMaxWidth(0.66f).height(4.5.dp).clip(CircleShape).background(preset.textMuted.copy(alpha = 0.72f)))
            }
            Column(
                Modifier
                    .weight(0.72f)
                    .fillMaxHeight()
                    .clip(RoundedCornerShape(6.dp))
                    .background(convoColor)
                    .padding(6.dp),
                verticalArrangement = Arrangement.SpaceBetween,
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Box(Modifier.fillMaxWidth(0.68f).height(4.5.dp).clip(CircleShape).background(preset.text.copy(alpha = 0.78f)))
                    Box(Modifier.fillMaxWidth(0.48f).height(4.5.dp).clip(CircleShape).background(preset.textMuted.copy(alpha = 0.68f)))
                }
                Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
                    Box(Modifier.width(26.dp).height(8.5.dp).clip(CircleShape).background(preset.action))
                }
            }
        }
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(preset.name, color = activeTheme.text, fontSize = 13.sp, fontWeight = FontWeight.Bold)
            Text(if (preset.isLight) "Light" else "Dark", color = activeTheme.textMuted, fontSize = 11.5.sp)
        }
    }
}

