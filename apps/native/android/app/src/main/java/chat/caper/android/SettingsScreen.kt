package chat.caper.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.accountLevelOptions
import chat.caper.android.data.isReleaseRevision
import chat.caper.android.model.AppUiState
import chat.caper.android.ui.*
import chat.caper.android.voice.VoiceState

/** The Settings screen's pages: Settings itself, then one per section. */
internal enum class SettingsPage(val title: String) {
    Root("Settings"),
    Appearance("Appearance"),
    Notifications("Notifications"),
    Audio("Voice & audio"),
    Privacy("Privacy & blocked accounts"),
    About("About"),
}

/**
 * The one Settings screen, opened from the account bar's gear: a full screen over the app with
 * Profile, the Appearance, Notifications, Voice & audio, Privacy & blocked accounts and About
 * pages, and Log out. Back returns from a page to Settings, and from Settings to the app. Edit
 * profile, What’s new and the audio panels open as dialogs above it, so closing one comes back here.
 */
@Composable internal fun SettingsScreen(
    state: AppUiState,
    voice: VoiceState,
    viewModel: CaperViewModel,
    page: SettingsPage,
    showPage: (SettingsPage) -> Unit,
    close: () -> Unit,
    editProfile: () -> Unit,
    openWhatsNew: () -> Unit,
    openAudioPanel: (AudioPanel) -> Unit,
) {
    val back = { if (page == SettingsPage.Root) close() else showPage(SettingsPage.Root) }
    BackHandler(onBack = back)
    val signedIn = state.account != null
    // Signed-in pages have nothing to show to a guest.
    if (!signedIn && (page == SettingsPage.Notifications || page == SettingsPage.Privacy)) LaunchedEffect(Unit) { showPage(SettingsPage.Root) }
    Surface(Modifier.fillMaxSize().semantics { paneTitle = page.title }, color = Surface) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing)) {
            Row(Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(back, Modifier.size(48.dp)) {
                    Icon(painterResource(R.drawable.lucide_chevron_left), if (page == SettingsPage.Root) "Close Settings" else "Back to Settings", Modifier.size(20.dp), tint = Text)
                }
                Text(page.title, Modifier.weight(1f).semantics { heading() }, fontSize = 17.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            HorizontalDivider(color = Border)
            // Each page arrives with a short slide; going back is instant.
            key(page) {
                Box(Modifier.weight(1f).fillMaxWidth().entrance(MOTION_BASE, dx = if (page == SettingsPage.Root) 0.dp else 16.dp), contentAlignment = Alignment.TopCenter) {
                    Column(
                        Modifier.widthIn(max = 640.dp).fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 18.dp, vertical = 16.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        when (page) {
                            SettingsPage.Root -> SettingsRoot(state, viewModel, showPage, close, editProfile)
                            SettingsPage.Appearance -> AppearanceSettings()
                            SettingsPage.Notifications -> if (signedIn) NotificationSettingsSection(state, viewModel)
                            SettingsPage.Audio -> VoiceAudioSettings(state, voice, openAudioPanel)
                            SettingsPage.Privacy -> if (signedIn) PrivacySettingsSection(state, viewModel)
                            SettingsPage.About -> AboutSettings(openWhatsNew)
                        }
                    }
                }
            }
        }
    }
}

@Composable private fun SettingsRoot(
    state: AppUiState,
    viewModel: CaperViewModel,
    showPage: (SettingsPage) -> Unit,
    close: () -> Unit,
    editProfile: () -> Unit,
) {
    val account = state.account
    if (account != null) {
        SettingLabel("Profile")
        val name = account.displayName?.takeIf { it.isNotBlank() } ?: account.username.orEmpty()
        Row(verticalAlignment = Alignment.CenterVertically) {
            Avatar(name, 56.dp, avatarId = account.avatarId, avatarAsset = account.avatarAsset, avatarUrl = account.avatarUrl)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(name, fontSize = 15.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                account.username?.takeIf { it.isNotBlank() }?.let { Text("@$it", color = TextMuted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
        }
        OutlinedButton(editProfile, Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
            Text("Edit profile", color = Text)
        }
        HorizontalDivider(color = Border)
    }
    SettingsRow("Appearance", CaperThemeStore.active.name) { showPage(SettingsPage.Appearance) }
    if (account != null) {
        val settings = state.notificationSettings
        val dnd = rememberDndLabel(settings?.pausedUntil)
        val level = settings?.let { loaded -> accountLevelOptions.firstOrNull { it.first == loaded.level }?.second }
        SettingsRow("Notifications", dnd?.let { "Do Not Disturb · $it" } ?: level) { showPage(SettingsPage.Notifications) }
    }
    SettingsRow("Voice & audio", null) { showPage(SettingsPage.Audio) }
    if (account != null) SettingsRow("Privacy & blocked accounts", null) { showPage(SettingsPage.Privacy) }
    SettingsRow("About", if (isReleaseRevision(BuildConfig.SOURCE_REVISION)) "Version ${BuildConfig.VERSION_NAME}" else "Development build") {
        showPage(SettingsPage.About)
    }
    HorizontalDivider(color = Border)
    if (account != null) OutlinedButton({ if (!state.loggingOut) { close(); viewModel.logout() } }, Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
        Text(if (state.loggingOut) "Logging out…" else "Log out", color = Text)
    } else OutlinedButton({ close(); viewModel.showLogin() }, Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
        Text("Sign in", color = Text)
    }
}

@Composable private fun SettingsRow(title: String, detail: String?, open: () -> Unit) = Row(
    Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(MaterialTheme.shapes.small).clickable(role = Role.Button, onClick = open)
        .padding(horizontal = 10.dp, vertical = 8.dp),
    verticalAlignment = Alignment.CenterVertically,
) {
    Column(Modifier.weight(1f)) {
        Text(title, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
        detail?.let { Text(it, color = TextMuted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
    }
    Icon(painterResource(R.drawable.lucide_chevron_right), null, Modifier.size(16.dp), tint = TextMuted)
}
