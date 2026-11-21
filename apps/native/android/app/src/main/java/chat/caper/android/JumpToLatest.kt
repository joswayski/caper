package chat.caper.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListLayoutInfo
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.model.ChatMessage
import chat.caper.android.ui.Border
import chat.caper.android.ui.Terracotta

/**
 * A message list's Jump to latest button: whether it shows, and how many of other people's messages arrived at the
 * live end while it did. [known] is the message IDs the list last showed.
 */
internal class JumpToLatest {
    var shown by mutableStateOf(false)
    var unseen by mutableIntStateOf(0)
    /** A tap's scroll is under way: the button stays hidden until it ends. */
    var jumping = false
    var known: Set<String>? = null

    fun hide() { shown = false; unseen = 0 }
}

/**
 * Whether Jump to latest shows: only while the loaded history reaches the live end, from more than half a viewport
 * away from the newest row; once shown it stays until the reader is back within [threshold] of the bottom.
 */
internal fun jumpToLatestShown(shown: Boolean, live: Boolean, distance: Int, viewport: Int, threshold: Int): Boolean =
    live && if (shown) distance > threshold else distance > viewport / 2

/**
 * Of [now], the messages from other people (not you or anyone blocked) after the newest message [known] already had:
 * what arrived at the live end. An older page or an edit adds none.
 */
internal fun liveArrivals(known: Set<String>, now: List<ChatMessage>, selfId: String?, blocked: Set<String>): Int {
    val newest = now.indexOfLast { it.id in known }
    if (newest < 0) return 0
    return (newest + 1 until now.size).count { index ->
        now[index].author.id != selfId && !chat.caper.android.data.collapsesFor(now[index], blocked, selfId)
    }
}

/** How far the list's content end lies below the viewport, estimating rows not laid out from the visible ones. */
internal fun LazyListLayoutInfo.distanceFromEnd(): Int {
    val visible = visibleItemsInfo
    val last = visible.lastOrNull() ?: return 0
    val beyond = (last.offset + last.size - (viewportEndOffset - afterContentPadding)).coerceAtLeast(0)
    val hidden = totalItemsCount - 1 - last.index
    return beyond + if (hidden > 0) hidden * (visible.sumOf { it.size } / visible.size) else 0
}

internal fun jumpToLatestLabel(unseen: Int): String = when {
    unseen <= 0 -> ""
    unseen == 1 -> "1 new message"
    unseen > 99 -> "99+ new messages"
    else -> "$unseen new messages"
}

/** Its own scope, so showing it or a new count recomposes only the button, not the timeline's rows. */
@Composable internal fun JumpToLatestOverlay(jump: JumpToLatest, onClick: () -> Unit, modifier: Modifier) {
    if (jump.shown) JumpToLatestButton(jump.unseen, onClick, modifier)
}

/** Surface fill, border (terracotta once something new arrived), a down arrow and the count beside it. */
@Composable internal fun JumpToLatestButton(unseen: Int, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val label = jumpToLatestLabel(unseen)
    Surface(
        onClick, modifier.entrance(MOTION_BASE, dy = 6.dp).heightIn(min = 44.dp).clearAndSetSemantics {
            role = Role.Button
            contentDescription = if (label.isEmpty()) "Jump to latest" else "Jump to latest, $label"
            onClick { onClick(); true }
        },
        shape = RoundedCornerShape(8.dp), color = chat.caper.android.ui.Surface, contentColor = chat.caper.android.ui.Text,
        border = BorderStroke(1.dp, if (unseen > 0) Terracotta else Border),
    ) {
        Row(Modifier.padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally)) {
            Icon(painterResource(R.drawable.lucide_arrow_down), null, Modifier.size(18.dp))
            if (label.isNotEmpty()) Text(label, fontSize = 12.sp, maxLines = 1, softWrap = false)
        }
    }
}
