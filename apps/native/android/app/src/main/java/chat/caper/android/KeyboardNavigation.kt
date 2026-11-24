package chat.caper.android

import android.content.Context
import android.view.KeyEvent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.model.AppUiState
import chat.caper.android.model.Channel
import chat.caper.android.model.SessionScreen
import chat.caper.android.ui.TerracottaWash
import chat.caper.android.ui.TextMuted
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceState

/**
 * Keyboard navigation shared by the quick switcher (Ctrl+K) and stepping through the sidebar's
 * conversations (Alt+Up/Down, with Shift only the unread ones). Every client implements the same
 * matching and stepping, checked against `shared/shortcuts/switcher-cases.json`.
 */
internal object KeyboardNavigation {
    /** How well [query] matches [text]: higher is better, null for no match, 0 for an empty query. */
    fun score(query: String, text: String): Int? {
        var trimmed = query.trim()
        if (trimmed.startsWith("#") || trimmed.startsWith("@")) trimmed = trimmed.substring(1)
        val joined = trimmed.lowercase()
        val q = joined.codePoints().toArray()
        if (q.isEmpty()) return 0
        val lower = text.lowercase()
        if (lower == joined) return 1000
        if (lower.startsWith(joined)) return 900
        val t = lower.codePoints().toArray()
        fun wordStart(index: Int) = index == 0 || !Character.isLetterOrDigit(t[index - 1])
        for (index in 0..t.size - q.size) if (wordStart(index) && q.indices.all { t[index + it] == q[it] }) return 800
        if (lower.contains(joined)) return 700
        var score = 100
        var last = -2
        var from = 0
        for (point in q) {
            val index = (from until t.size).firstOrNull { t[it] == point } ?: return null
            if (index == last + 1) score += 10
            if (wordStart(index)) score += 5
            score -= index - from
            last = index
            from = index + 1
        }
        return score.coerceIn(1, 699)
    }

    /** The indexes of the items (their texts, unread) that match, best first; unread ones lead among equals. */
    fun rank(items: List<Pair<List<String>, Boolean>>, query: String): List<Int> =
        items.mapIndexedNotNull { index, (texts, unread) ->
            texts.mapNotNull { score(query, it) }.maxOrNull()?.let { (it + if (unread) 50 else 0) to index }
        }.sortedWith(compareByDescending<Pair<Int, Int>> { it.first }.thenBy { it.second }).map { it.second }

    /** The conversation Alt+Up/Down (or with [unreadOnly], Alt+Shift+Up/Down) moves to from [current], wrapping. */
    fun adjacent(unread: List<Boolean>, current: Int?, forward: Boolean, unreadOnly: Boolean): Int? {
        val count = unread.size
        for (step in 0 until count) {
            val index = if (current != null) Math.floorMod(current + if (forward) step + 1 else -step - 1, count)
                else if (forward) step else count - 1 - step
            if (index == current) return null
            if (!unreadOnly || unread[index]) return index
        }
        return null
    }

    /** The keyboard shortcuts sheet's sections. */
    val shortcuts: List<Pair<String, List<Pair<String, String>>>> = listOf(
        "Navigation" to listOf(
            "Quick switcher" to "Ctrl+K",
            "Previous or next conversation" to "Alt+↑ / Alt+↓",
            "Previous or next unread conversation" to "Alt+Shift+↑ / Alt+Shift+↓",
            "Keyboard shortcuts" to "Ctrl+/",
        ),
        "Conversation" to listOf(
            "Mark as read" to "Esc",
            "Edit your last message (in an empty message box)" to "↑",
            "Bold, italic, underline, code" to "Ctrl+B / Ctrl+I / Ctrl+U / Ctrl+E",
        ),
        "Voice" to listOf(
            "Mute or unmute (in a call)" to "Ctrl+Shift+M",
            "Deafen or undeafen (in a call)" to "Ctrl+Shift+D",
        ),
    )
}

/** A quick switcher destination. */
internal data class SwitcherItem(
    val id: String,
    val name: String,
    val alternates: List<String>,
    /** The second line: the space, a username, or "Space". */
    val detail: String,
    val unread: Boolean,
    val target: Target,
) {
    sealed interface Target {
        data class OpenChannel(val spaceId: String, val channel: Channel) : Target
        data class Direct(val id: String) : Target
        data class OpenSpace(val id: String) : Target
        /** Someone you share a space with but have no DM with: opening starts one. */
        data class Person(val username: String) : Target
    }
}

internal enum class KeyboardSheet { SWITCHER, SHORTCUTS }

/**
 * A hardware keyboard's navigation keys, taken ahead of the focused field only for combinations
 * no text field or input method uses. Escape is not here: it reaches [escapeMarksRead] only after
 * suggestions, menus and dialogs had their turn. True when the key was used.
 */
internal fun navigationKey(event: KeyEvent, viewModel: CaperViewModel, context: Context): Boolean {
    if (event.action != KeyEvent.ACTION_DOWN || event.repeatCount > 0) return false
    val state = viewModel.state.value
    if (state.screen != SessionScreen.Home && state.screen !is SessionScreen.Spaces || state.account == null) return false
    val primary = event.isCtrlPressed || event.isMetaPressed
    val inCall = VoiceCallService.state.value.phase == VoiceState.Phase.CONNECTED
    when {
        primary && !event.isAltPressed && !event.isShiftPressed && event.keyCode == KeyEvent.KEYCODE_K -> viewModel.toggleKeyboardSheet(KeyboardSheet.SWITCHER)
        primary && !event.isAltPressed && event.keyCode == KeyEvent.KEYCODE_SLASH -> viewModel.toggleKeyboardSheet(KeyboardSheet.SHORTCUTS)
        event.isAltPressed && !primary && (event.keyCode == KeyEvent.KEYCODE_DPAD_UP || event.keyCode == KeyEvent.KEYCODE_DPAD_DOWN) ->
            viewModel.stepConversation(forward = event.keyCode == KeyEvent.KEYCODE_DPAD_DOWN, unreadOnly = event.isShiftPressed)
        inCall && primary && event.isShiftPressed && !event.isAltPressed && event.keyCode == KeyEvent.KEYCODE_M -> VoiceCallService.toggleMute(context)
        inCall && primary && event.isShiftPressed && !event.isAltPressed && event.keyCode == KeyEvent.KEYCODE_D -> VoiceCallService.toggleDeafen(context)
        else -> return false
    }
    return true
}

/** Escape nothing else took marks the open conversation read, as on web. */
internal fun escapeMarksRead(event: KeyEvent, viewModel: CaperViewModel): Boolean =
    event.action == KeyEvent.ACTION_DOWN && event.keyCode == KeyEvent.KEYCODE_ESCAPE && !event.isCtrlPressed && !event.isAltPressed &&
        !event.isShiftPressed && !event.isMetaPressed && viewModel.markOpenConversationRead()

/** Ctrl+K: channels, DMs, spaces and people, unread first, best match first; Enter opens the highlighted one. */
@Composable internal fun QuickSwitcherDialog(state: AppUiState, viewModel: CaperViewModel, close: () -> Unit, opened: () -> Unit) {
    var query by remember { mutableStateOf("") }
    var selected by remember { mutableIntStateOf(0) }
    val others by viewModel.switcherSpaces.collectAsStateWithLifecycle()
    val all = remember(state.selectedSpace, state.directConversations, state.spaces, state.people, state.unreadSpaces, state.blocks, state.notificationSettings, others) {
        viewModel.switcherItems()
    }
    val results = remember(all, query) { viewModel.rankSwitcherItems(all, query) }
    val list = rememberLazyListState()
    LaunchedEffect(Unit) { viewModel.refreshPeople(); viewModel.loadSwitcherSpaces() }
    // Other spaces' channels arriving keep the highlighted destination.
    var highlighted by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(results) { highlighted?.let { id -> results.indexOfFirst { it.id == id }.takeIf { it >= 0 }?.let { selected = it } } }
    LaunchedEffect(selected, results) { highlighted = results.getOrNull(selected)?.id }
    LaunchedEffect(selected, results.size) { if (selected in results.indices) list.animateScrollToItem(selected) }
    fun open(item: SwitcherItem) { close(); viewModel.openSwitcherItem(item); opened() }
    CaperDialog("Jump to", close) {
        val field = remember { FocusRequester() }
        LaunchedEffect(Unit) { runCatching { field.requestFocus() } }
        OutlinedTextField(query, { query = it; selected = 0; highlighted = null }, placeholder = { Text("Channels, DMs, spaces and people") }, singleLine = true,
            modifier = Modifier.fillMaxWidth().focusRequester(field).onPreviewKeyEvent { event ->
                if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when {
                    event.key == Key.DirectionDown && results.isNotEmpty() -> { selected = (selected + 1) % results.size; true }
                    event.key == Key.DirectionUp && results.isNotEmpty() -> { selected = (selected - 1 + results.size) % results.size; true }
                    event.key == Key.Enter || event.key == Key.NumPadEnter -> { results.getOrNull(selected)?.let(::open); true }
                    event.key == Key.Escape -> { close(); true }
                    event.key == Key.K && (event.isCtrlPressed || event.isMetaPressed) -> { close(); true }
                    else -> false
                }
            })
        if (results.isEmpty()) Text("No matches", color = TextMuted, fontSize = 13.sp)
        LazyColumn(Modifier.fillMaxWidth().heightIn(max = 420.dp), state = list) {
            itemsIndexed(results, key = { _, item -> item.id }) { index, item ->
                val on = index == selected
                Column(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(MaterialTheme.shapes.small)
                    .background(if (on) TerracottaWash else Color.Transparent)
                    .clickable(role = Role.Button) { open(item) }
                    .semantics { if (on) this.selected = true }
                    .padding(horizontal = 10.dp, vertical = 6.dp), verticalArrangement = Arrangement.Center) {
                    Text(item.name, fontSize = 14.sp, fontWeight = if (item.unread) FontWeight.Bold else FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(if (item.unread) "${item.detail} · Unread" else item.detail, color = TextMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
}

/** Ctrl+/: every keyboard shortcut. */
@Composable internal fun ShortcutHelpDialog(close: () -> Unit) = CaperDialog("Keyboard shortcuts", close) {
    for ((title, rows) in KeyboardNavigation.shortcuts) {
        Text(title, Modifier.semantics { heading() }.padding(top = 6.dp), color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
        for ((action, keys) in rows) {
            Row(Modifier.fillMaxWidth().heightIn(min = 32.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(action, Modifier.weight(1f), fontSize = 13.sp)
                Spacer(Modifier.padding(start = 12.dp))
                Text(keys, fontSize = 13.sp, fontWeight = FontWeight.Bold)
            }
        }
    }
}
