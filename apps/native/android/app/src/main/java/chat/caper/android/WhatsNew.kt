package chat.caper.android

import android.content.Context
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withLink
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
) = CaperDialog("What’s new", close, description = "Updates to Caper on Android.") {
    val releases = displayed?.releases.orEmpty().filter { it.highlights.isNotEmpty() }.asReversed()
    if (isReleaseRevision(BuildConfig.SOURCE_REVISION)) {
        ReleaseCommitLink("Installed: ${BuildConfig.VERSION_NAME} · ${BuildConfig.SOURCE_REVISION.take(7)}", BuildConfig.SOURCE_REVISION)
    } else {
        Text("Installed: ${BuildConfig.VERSION_NAME}", fontSize = 12.sp, color = TextMuted)
    }
    if (displayed == null && state.loading) {
        CircularProgressIndicator(color = Terracotta, modifier = Modifier.size(24.dp))
        Text("Loading release notes…", color = TextMuted)
    } else if (displayed == null && state.error != null) {
        Text(state.error, color = ErrorText)
        TextButton(retry, enabled = !state.loading) { Text("Retry") }
    } else if (releases.isEmpty()) {
        Text("No release highlights for this version yet.", color = TextMuted)
    } else releases.groupBy { it.publishedAt.take(10) }.forEach { (date, dayReleases) ->
        Text(date, fontSize = 12.sp, color = TextMuted)
        dayReleases.forEach { release ->
            ReleaseCommitLink(release.version?.let { "Version $it" } ?: "Commit ${release.revision.take(7)}", release.revision)
            release.highlights.forEach { highlight ->
                Text(highlight.title, fontSize = 16.sp, fontWeight = FontWeight.Bold)
                Text(highlight.body, fontSize = 14.sp, color = TextMuted)
            }
        }
        HorizontalDivider(color = Border)
    }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
        Button(close) { Text(if (automatic) "Got it" else "Done") }
    }
}

@Composable private fun ReleaseCommitLink(label: String, revision: String) {
    val uriHandler = LocalUriHandler.current
    val url = "https://github.com/joswayski/caper/commit/$revision"
    Text(buildAnnotatedString {
        withLink(LinkAnnotation.Url(url, MessageLink) { openExternalLink(uriHandler, url) }) { append(label) }
    }, fontSize = 12.sp, color = TextMuted)
}
