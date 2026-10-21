package chat.caper.android

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.input.pointer.pointerHoverIcon
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.*
import chat.caper.android.ui.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

internal data class WhatsNewUi(
    val loading: Boolean = false, val error: String? = null, val snapshot: WhatsNewSnapshot? = null,
    val unread: List<PlatformRelease> = emptyList(),
)

internal class WhatsNewClient(context: Context, private val api: CaperApi) {
    private val preferences = context.getSharedPreferences("whats_new", Context.MODE_PRIVATE)
    private val key = "android:${api.baseUrl}"
    private val history = WhatsNewHistory(
        read = { if (preferences.contains(key)) preferences.getLong(key, 0L) else null },
        write = { preferences.edit().putLong(key, it).apply() },
    )
    private val mutable = MutableStateFlow(WhatsNewUi())
    val state = mutable.asStateFlow()

    suspend fun refresh() {
        if (mutable.value.loading) return
        if (!isReleaseRevision(BuildConfig.SOURCE_REVISION)) {
            mutable.value = WhatsNewUi(error = "Release notes are unavailable in development builds.")
            return
        }
        mutable.value = mutable.value.copy(loading = true, error = null)
        try {
            history.receive(api.whatsNew(BuildConfig.SOURCE_REVISION), "android", BuildConfig.SOURCE_REVISION)
            mutable.value = WhatsNewUi(snapshot = history.snapshot, unread = history.unread)
        } catch (error: CancellationException) {
            mutable.value = mutable.value.copy(loading = false)
            throw error
        } catch (_: Exception) {
            mutable.value = mutable.value.copy(loading = false, error = "Could not load release notes. Please try again.")
        }
    }

    fun dismiss(sequence: Long?) {
        sequence?.let(history::acknowledge)
        mutable.value = mutable.value.copy(unread = history.unread)
    }
}

@Composable internal fun WhatsNewDialog(
    state: WhatsNewUi, displayed: WhatsNewSnapshot?, automatic: Boolean, retry: () -> Unit, close: () -> Unit,
) = CaperDialog(
    "What’s new", close, heroIcon = R.drawable.caper_face,
    // Like the update review on desktop: the installed version right under the title.
    subtitle = {
        val version = "Installed version ${BuildConfig.VERSION_NAME}"
        if (isReleaseRevision(BuildConfig.SOURCE_REVISION)) ReleaseCommitLink(version, BuildConfig.SOURCE_REVISION, 13)
        else Text(version, fontSize = 13.sp, color = TextMuted)
    },
) {
    val releases = displayed?.releases.orEmpty().filter { it.highlights.isNotEmpty() }.asReversed()
    if (displayed == null && state.loading) {
        CircularProgressIndicator(color = Terracotta, modifier = Modifier.size(24.dp))
        Text("Loading release highlights…", color = TextMuted)
    } else if (displayed == null && state.error != null) {
        Text(state.error, color = ErrorText)
        TextButton(retry, enabled = !state.loading) { Text("Retry") }
    } else if (releases.isEmpty()) {
        Text("No release highlights for this version yet.", color = TextMuted)
    } else releases.groupBy { it.publishedAt.take(10) }.forEach { (date, dayReleases) ->
        Text(releaseDayLabel(date), fontSize = 12.sp, fontWeight = FontWeight.Bold, color = TextMuted)
        dayReleases.forEach { release ->
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                ReleaseCommitLink(release.version?.let { "Version $it" } ?: release.revision.take(7), release.revision)
                release.highlights.forEach { highlight ->
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(highlight.title, fontSize = 16.sp, fontWeight = FontWeight.Bold)
                        Text(highlight.body, fontSize = 14.sp, color = TextMuted)
                    }
                }
            }
            HorizontalDivider(color = Border)
        }
    }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
        Button(close) { Text(if (automatic) "Got it" else "Done") }
    }
}

/** The icon identifies an external link; underline and brighten only during interaction. */
@Composable private fun ReleaseCommitLink(label: String, revision: String, size: Int = 12) {
    val uriHandler = LocalUriHandler.current
    val url = "https://github.com/joswayski/caper/commit/$revision"
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    val focused by interaction.collectIsFocusedAsState()
    val pressed by interaction.collectIsPressedAsState()
    val active = hovered || focused || pressed
    val color = if (active) Text else TextMuted.copy(alpha = 0.72f)
    Row(
        Modifier.pointerHoverIcon(PointerIcon.Hand)
            .clickable(interaction, indication = null, role = Role.Button,
                onClickLabel = "View this version on GitHub") { openExternalLink(uriHandler, url) },
        horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, Modifier.weight(1f, fill = false), fontSize = size.sp, color = color,
            textDecoration = if (active) TextDecoration.Underline else TextDecoration.None)
        Icon(painterResource(R.drawable.lucide_external_link), null, Modifier.size(12.dp), tint = color)
    }
}

/** "October 18, 2026" for a release's UTC publication day ("2026-10-18"). */
internal fun releaseDayLabel(day: String): String = runCatching {
    java.time.LocalDate.parse(day).format(java.time.format.DateTimeFormatter.ofPattern("MMMM d, yyyy", java.util.Locale.US))
}.getOrDefault(day)
