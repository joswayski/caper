package chat.caper.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import chat.caper.android.data.*
import chat.caper.android.model.*
import chat.caper.android.ui.*
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable internal fun MessageEditorDialog(message: ChatMessage, viewModel: CaperViewModel, close: () -> Unit) {
    var baseline by remember(message.id) { mutableStateOf(message) }
    var draft by remember(message.id) { mutableStateOf(message.content.text) }
    var saving by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    fun save() {
        if (saving || !validEditText(draft)) return
        saving = true; error = null
        scope.launch {
            try { viewModel.editMessage(baseline, draft); close() }
            catch (failure: CancellationException) { throw failure }
            catch (failure: Exception) { error = failure.message ?: "Edit could not be saved. Your draft is kept." }
            finally { saving = false }
        }
    }
    Dialog(onDismissRequest = { if (!saving) close() }) {
        Surface(color = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
            Column(Modifier.padding(20.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Edit message", style = MaterialTheme.typography.titleLarge)
                Text("Previous versions remain visible to people who can read this message.", color = TextMuted, fontSize = 12.sp)
                OutlinedTextField(draft, { draft = it }, Modifier.fillMaxWidth().heightIn(min = 140.dp, max = 300.dp), label = { Text("Message") }, enabled = !saving)
                Text("${draft.codePointCount(0, draft.length)} / 4,000", color = TextMuted, fontSize = 12.sp)
                error?.let { Text(it, color = Terracotta); TextButton(enabled = !saving, onClick = {
                    saving = true; error = null
                    scope.launch {
                        try { baseline = viewModel.reloadMessage(baseline); draft = baseline.content.text }
                        catch (failure: CancellationException) { throw failure }
                        catch (failure: Exception) { error = failure.message ?: "Couldn’t load the latest version." }
                        finally { saving = false }
                    }
                }) { Text("Discard draft and load latest") } }
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                    TextButton(close, enabled = !saving) { Text("Cancel") }
                    Button(::save, enabled = !saving && validEditText(draft)) { Text(if (saving) "Saving…" else "Save changes") }
                }
            }
        }
    }
}

@Composable internal fun MessageHistoryDialog(message: ChatMessage, viewModel: CaperViewModel, close: () -> Unit) {
    var versions by remember(message.id) { mutableStateOf(emptyList<MessageVersion>()) }
    var more by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var selected by remember { mutableStateOf<Int?>(null) }
    var menu by remember { mutableStateOf(false) }
    var request by remember { mutableStateOf(0) }
    val scope = rememberCoroutineScope()
    suspend fun load(older: Boolean) {
        val currentRequest = ++request
        loading = true; error = null
        try {
            val page = viewModel.messageVersions(message, if (older) versions.lastOrNull()?.revision else null)
            if (currentRequest != request) return
            versions = if (older) (versions + page.versions).distinctBy { it.revision } else page.versions
            more = page.hasMore
            if (!older) selected = null
        } catch (failure: CancellationException) { throw failure }
        catch (failure: Exception) { if (currentRequest == request) error = failure.message ?: "Message history could not be loaded." }
        finally { if (currentRequest == request) loading = false }
    }
    LaunchedEffect(message.id, message.revision) { load(false) }
    Dialog(onDismissRequest = close, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxWidth(.95f).widthIn(max = 880.dp).fillMaxHeight(.9f), color = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
            Column(Modifier.padding(18.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    Text("Message history", style = MaterialTheme.typography.titleLarge)
                    TextButton(close) { Text("Close") }
                }
                Text("${message.author.name} · Previous versions are retained.", color = TextMuted, fontSize = 12.sp)
                versions.firstOrNull()?.let { current ->
                    if (versions.size > 1) VersionComparison(versions[1], current, true)
                    else { Text("Original version"); Text(current.content.text) }
                }
                if (versions.size > 1) {
                    HorizontalDivider(color = Border)
                    Text("View previous versions")
                    Box {
                        OutlinedButton({ menu = true }) { Text(selected?.let { "Version $it" } ?: "Choose an earlier version…") }
                        DropdownMenu(menu, { menu = false }) {
                            DropdownMenuItem(text = { Text("Choose an earlier version…") }, onClick = { selected = null; menu = false })
                            versions.drop(1).forEach { item -> DropdownMenuItem(text = { Text(versionLabel(item)) }, onClick = { selected = item.revision; menu = false }) }
                        }
                    }
                    versions.find { it.revision == selected }?.let { version ->
                        val previous = versions.find { it.revision == version.revision - 1 }
                        if (previous != null) VersionComparison(previous, version)
                        else {
                            Text(if (version.revision == 1) "Original version" else "Version ${version.revision}")
                            Text(version.content.text, fontFamily = FontFamily.Monospace)
                            if (version.revision > 1 && more) Text("Load older versions to compare this change.", color = TextMuted)
                        }
                    }
                }
                if (loading) Text("Loading versions…", color = TextMuted)
                error?.let { Text(it, color = Terracotta); TextButton(enabled = !loading, onClick = { scope.launch { load(versions.isNotEmpty()) } }) { Text("Retry") } }
                if (more && error == null) TextButton(enabled = !loading, onClick = { scope.launch { load(true) } }) { Text("Load older versions") }
            }
        }
    }
}

private fun versionLabel(version: MessageVersion): String {
    val date = runCatching { DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT).withZone(ZoneId.systemDefault()).format(Instant.parse(version.createdAt)) }.getOrDefault(version.createdAt)
    return "${if (version.revision == 1) "Original version" else "Version ${version.revision}"} · $date"
}

@Composable private fun VersionComparison(before: MessageVersion, after: MessageVersion, current: Boolean = false) {
    val diff = remember(before, after) { messageDiff(before.content.text, after.content.text) }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        listOf(Triple(before, diff.first, if (current) "Previous version" else "Before"), Triple(after, diff.second, if (current) "Current version" else "After")).forEachIndexed { index, (version, tokens, label) ->
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(label, style = MaterialTheme.typography.titleSmall)
                Text(versionLabel(version), color = TextMuted, fontSize = 10.sp)
                Text(buildAnnotatedString {
                    tokens.forEach { token ->
                        withStyle(SpanStyle(background = if (token.changed) (if (index == 0) Terracotta else Color(0xFF637A43)).copy(alpha = .3f) else Color.Transparent)) { append(token.text) }
                    }
                }, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
            }
        }
    }
}
