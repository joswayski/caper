package chat.caper.android

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.*
import chat.caper.android.model.AppUiState
import chat.caper.android.model.NotificationOverride
import chat.caper.android.model.NotificationSettings
import chat.caper.android.push.PushRegistration
import chat.caper.android.ui.*
import java.time.Duration
import java.time.Instant
import kotlinx.coroutines.delay

/** Which part of an options menu shows: its own items, the notification levels, or the mute choices. */
internal enum class NotificationMenuPage { Main, Level, Mute }

/** The current time, advanced when the next mute ends so muted rows brighten on time. */
@Composable internal fun rememberMuteClock(settings: NotificationSettings?): Instant {
    var now by remember { mutableStateOf(Instant.now()) }
    LaunchedEffect(settings, now) {
        val end = settings?.nextMuteEnd(now) ?: return@LaunchedEffect
        delay((Duration.between(Instant.now(), end).toMillis() + 500).coerceAtLeast(0))
        now = Instant.now()
    }
    return now
}

/** The bell-slash beside a muted space, channel or DM; the row itself says "Muted" to accessibility. */
@Composable internal fun MutedBell(modifier: Modifier = Modifier) =
    Icon(painterResource(R.drawable.lucide_bell_off), null, modifier.size(14.dp), tint = TextMuted)

/** A failed notification save, beside the control that made it. */
@Composable internal fun NotificationSaveError(error: String?, modifier: Modifier = Modifier, dismiss: () -> Unit) {
    if (error == null) return
    Row(modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite }, verticalAlignment = Alignment.CenterVertically) {
        Text(error, Modifier.weight(1f), color = ErrorText, fontSize = 11.sp)
        IconButton(dismiss, Modifier.size(36.dp)) { Icon(painterResource(R.drawable.lucide_x), "Dismiss", Modifier.size(14.dp), tint = TextMuted) }
    }
}

@Composable private fun MenuLabel(label: String, detail: String?, fontSize: TextUnit) = Column {
    Text(label, fontSize = fontSize)
    detail?.let { Text(it, color = TextMuted, fontSize = 11.sp) }
}

@Composable private fun MenuIcon(icon: Int, modifier: Modifier = Modifier) = Icon(painterResource(icon), null, modifier.size(16.dp))

@Composable private fun MenuBack(title: String, fontSize: TextUnit, back: () -> Unit) = DropdownMenuItem(
    text = { Text(title, fontSize = fontSize, fontWeight = FontWeight.Bold) }, onClick = back,
    leadingIcon = { Icon(painterResource(R.drawable.lucide_arrow_right), "Back", Modifier.size(16.dp).graphicsLayer { rotationZ = 180f }) },
)

@Composable private fun MutePage(title: String, fontSize: TextUnit, choose: (MutePreset) -> Unit, back: () -> Unit) {
    MenuBack(title, fontSize, back)
    MutePreset.entries.forEach { preset -> DropdownMenuItem(text = { Text(preset.label, fontSize = fontSize) }, onClick = { choose(preset) }) }
}

/**
 * A space or channel menu: [before] and [after] (its own items) around Notifications and
 * Mute, or the level or mute choices while [page] shows them. Choosing one closes the menu.
 * [inherited] is what Default means, and null while settings load. A channel in a muted
 * space ([spaceMuted]) says so and keeps its own mute choices.
 */
@Composable internal fun ScopeMenuContent(
    page: NotificationMenuPage,
    showPage: (NotificationMenuPage) -> Unit,
    noun: String,
    override: NotificationOverride?,
    inherited: String?,
    change: (OverrideChange) -> Unit,
    close: () -> Unit,
    spaceMuted: Boolean = false,
    fontSize: TextUnit = TextUnit.Unspecified,
    before: @Composable () -> Unit = {},
    after: @Composable () -> Unit = {},
) {
    val choose = { next: OverrideChange -> close(); change(next) }
    when (page) {
        NotificationMenuPage.Main -> {
            before()
            val enabled = inherited != null
            DropdownMenuItem(
                text = { MenuLabel("Notifications", inherited?.let { override?.level?.let(::overrideLevelLabel) ?: defaultLevelLabel(it) }, fontSize) },
                onClick = { showPage(NotificationMenuPage.Level) }, enabled = enabled,
                leadingIcon = { MenuIcon(R.drawable.lucide_bell) }, trailingIcon = { MenuIcon(R.drawable.lucide_chevron_right) },
            )
            val muted = muteLabel(override?.mutedUntil, Instant.now())
            if (muted != null) DropdownMenuItem(
                text = { MenuLabel("Unmute $noun", muted, fontSize) }, onClick = { choose(OverrideChange.Mute(null)) }, enabled = enabled,
                leadingIcon = { MenuIcon(R.drawable.lucide_bell) },
            ) else DropdownMenuItem(
                text = { Text("Mute $noun", fontSize = fontSize) }, onClick = { showPage(NotificationMenuPage.Mute) }, enabled = enabled,
                leadingIcon = { MenuIcon(R.drawable.lucide_bell_off) }, trailingIcon = { MenuIcon(R.drawable.lucide_chevron_right) },
            )
            if (spaceMuted) DropdownMenuItem(text = { Text("Muted with the space", fontSize = fontSize) }, onClick = {}, enabled = false,
                leadingIcon = { MenuIcon(R.drawable.lucide_bell_off) })
            after()
        }
        NotificationMenuPage.Level -> {
            MenuBack("Notifications", fontSize) { showPage(NotificationMenuPage.Main) }
            val choices = listOf<Pair<String?, String>>(null to defaultLevelLabel(inherited ?: LEVEL_ALL)) + overrideLevelOptions
            choices.forEach { (value, label) ->
                val current = override?.level == value
                DropdownMenuItem(
                    text = { Text(label, fontSize = fontSize) }, onClick = { choose(OverrideChange.Level(value)) },
                    modifier = Modifier.semantics { selected = current }, leadingIcon = { RadioButton(current, null) },
                )
            }
        }
        NotificationMenuPage.Mute -> MutePage("Mute $noun", fontSize, { choose(OverrideChange.Mute(it.mutedUntil(Instant.now()))) }) {
            showPage(NotificationMenuPage.Main)
        }
    }
}

/** A DM row's options: notifications on or off, and mute. [override] is null when none is set; [loaded] is false while settings load. */
@Composable internal fun DirectOptionsMenu(
    expanded: Boolean,
    dismiss: () -> Unit,
    override: NotificationOverride?,
    loaded: Boolean,
    change: (OverrideChange) -> Unit,
) {
    var page by remember { mutableStateOf(NotificationMenuPage.Main) }
    val close = { page = NotificationMenuPage.Main; dismiss() }
    val choose = { next: OverrideChange -> close(); change(next) }
    DropdownMenu(expanded, close, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
        if (page == NotificationMenuPage.Mute) MutePage("Mute conversation", 13.sp, { choose(OverrideChange.Mute(it.mutedUntil(Instant.now()))) }) {
            page = NotificationMenuPage.Main
        } else {
            val off = override?.level == LEVEL_NOTHING
            DropdownMenuItem(
                text = { Text(if (off) "Turn on notifications" else "Turn off notifications", fontSize = 13.sp) },
                onClick = { choose(OverrideChange.Level(if (off) null else LEVEL_NOTHING)) }, enabled = loaded,
                leadingIcon = { MenuIcon(if (off) R.drawable.lucide_bell else R.drawable.lucide_bell_off) },
            )
            val muted = muteLabel(override?.mutedUntil, Instant.now())
            if (muted != null) DropdownMenuItem(
                text = { MenuLabel("Unmute conversation", muted, 13.sp) }, onClick = { choose(OverrideChange.Mute(null)) }, enabled = loaded,
                leadingIcon = { MenuIcon(R.drawable.lucide_bell) },
            ) else DropdownMenuItem(
                text = { Text("Mute conversation", fontSize = 13.sp) }, onClick = { page = NotificationMenuPage.Mute }, enabled = loaded,
                leadingIcon = { MenuIcon(R.drawable.lucide_bell_off) }, trailingIcon = { MenuIcon(R.drawable.lucide_chevron_right) },
            )
        }
    }
}

@Composable private fun SettingChoices(title: String, options: List<Pair<String, String>>, selected: String?, enabled: Boolean, choose: (String) -> Unit) {
    Text(title, fontWeight = FontWeight.Bold, fontSize = 13.sp)
    Column(Modifier.selectableGroup()) {
        options.forEach { (value, label) ->
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).selectable(selected == value, enabled = enabled, role = Role.RadioButton) {
                    if (selected != value) choose(value)
                },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RadioButton(selected == value, null, enabled = enabled)
                Spacer(Modifier.width(8.dp))
                Text(label, fontSize = 13.sp)
            }
        }
    }
}

/**
 * User settings → Notifications: the account level and, when this build has Firebase and
 * the server offers FCM, "Send to this phone" and this phone's on/off switch. The Android 13+
 * permission prompt only follows that switch.
 */
@Composable internal fun NotificationSettingsSection(state: AppUiState, viewModel: CaperViewModel) {
    val context = LocalContext.current
    val account = state.account?.id
    var pushAvailable by remember(account) { mutableStateOf(false) }
    var pushEnabled by remember(account) { mutableStateOf(PushRegistration.enabled(context)) }
    var pushWorking by remember(account) { mutableStateOf(false) }
    var pushError by remember(account) { mutableStateOf<String?>(null) }
    var pushRequestEpoch by remember { mutableLongStateOf(-1L) }
    val enablePush = {
        pushWorking = true
        viewModel.enablePush { error ->
            pushWorking = false
            pushEnabled = PushRegistration.enabled(context)
            pushError = error
        }
    }
    val pushPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (pushRequestEpoch == viewModel.accountEpoch) {
            if (granted) enablePush() else pushError = "Notification permission was denied. You can allow it in Android settings."
        }
    }
    LaunchedEffect(account) {
        viewModel.refreshNotificationSettings()
        if (BuildConfig.FIREBASE_ENABLED && account != null) pushAvailable = viewModel.canEnablePush()
    }
    val settings = state.notificationSettings
    HorizontalDivider(color = Border)
    Text("Notifications", Modifier.semantics { heading() }, fontWeight = FontWeight.Bold, fontSize = 14.sp)
    SettingChoices("Notify me about", accountLevelOptions, settings?.level, settings != null, viewModel::setNotificationLevel)
    NotificationSaveError(state.notificationErrors[ACCOUNT_LEVEL_KEY]) { viewModel.dismissNotificationError(ACCOUNT_LEVEL_KEY) }
    if (settings == null) {
        val error = state.notificationSettingsError
        Text(error ?: "Loading…", color = if (error != null) ErrorText else TextMuted, fontSize = 12.sp)
        if (error != null) TextButton(viewModel::refreshNotificationSettings) { Text("Retry") }
    }
    if (!pushAvailable) return
    SettingChoices("Send to this phone", mobileOptions, settings?.mobile, settings != null, viewModel::setMobileNotifications)
    NotificationSaveError(state.notificationErrors[MOBILE_KEY]) { viewModel.dismissNotificationError(MOBILE_KEY) }
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).toggleable(pushEnabled, enabled = !pushWorking, role = Role.Switch) {
            pushError = null
            if (pushEnabled) { pushWorking = true; viewModel.disablePush { pushWorking = false; pushEnabled = false } }
            else if (Build.VERSION.SDK_INT >= 33) {
                pushRequestEpoch = viewModel.accountEpoch
                pushPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
            } else enablePush()
        },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("Notifications on this phone", Modifier.weight(1f), fontSize = 13.sp)
        Switch(pushEnabled, null, enabled = !pushWorking)
    }
    pushError?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
}
