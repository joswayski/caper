package chat.caper.android

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyListLayoutInfo
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.dropShadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.shadow.Shadow
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.model.ChatMessage
import chat.caper.android.ui.BorderSubtle
import chat.caper.android.ui.SurfaceRaised
import chat.caper.android.ui.Terracotta
import chat.caper.android.ui.primaryButtonColors

/**
 * A message list's live Jump to latest toast: whether it shows, and how many of other people's messages arrived at the
 * live end while it did. [known] is the message IDs the list last showed.
 */
internal class JumpToLatest {
    var shown by mutableStateOf(false)
    var unseen by mutableIntStateOf(0)
    /** A tap's scroll is under way: the toast stays hidden until it ends. */
    var jumping = false
    var known: Set<String>? = null

    fun hide() { shown = false; unseen = 0 }
}

/**
 * Whether a live list's Jump to latest toast shows: from more than half a viewport away from the newest row; once shown
 * it stays until the reader is back within [threshold] of the bottom. A detached window ([live] false) shows its own,
 * uncounted toast instead (see [jumpToastCount]).
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

/** The toast's label: the count of other people's arrivals, or that the reader is looking at older messages. */
internal fun jumpToLatestLabel(unseen: Int): String = when {
    unseen <= 0 -> "You’re viewing older messages"
    unseen == 1 -> "1 new message"
    unseen > 99 -> "99+ new messages"
    else -> "$unseen new messages"
}

/** The button's accessible name, with the count when there is one. */
internal fun jumpToLatestName(unseen: Int): String = if (unseen > 0) "Jump to latest, ${jumpToLatestLabel(unseen)}" else "Jump to latest"

/**
 * The toast's count, or null while it is hidden: a detached history window (one that stops short of the live end)
 * always shows it, uncounted; a live list shows it only while [shown], with what arrived meanwhile.
 */
internal fun jumpToastCount(detached: Boolean, shown: Boolean, unseen: Int): Int? = when {
    detached -> 0
    shown -> unseen
    else -> null
}

/**
 * Its own scope, so showing it or a new count recomposes only the toast, not the timeline's rows. In a [detached]
 * window its button reloads the latest page ([onReturn]); otherwise it glides back to the newest row ([onJump]).
 */
@Composable internal fun JumpToLatestOverlay(jump: JumpToLatest, detached: Boolean, onJump: () -> Unit, onReturn: () -> Unit, modifier: Modifier) {
    val count = jumpToastCount(detached, jump.shown, jump.unseen) ?: return
    JumpToLatestToast(count, if (detached) onReturn else onJump, modifier)
}

/**
 * Discord's "You’re viewing older messages" toast (web's `.chat-jump-latest`): a raised card with a hairline border
 * (terracotta once something new arrived), the label, which wraps on a narrow list rather than truncating, and a
 * primary Jump to latest button.
 */
@Composable internal fun JumpToLatestToast(unseen: Int, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val shape = MaterialTheme.shapes.medium
    Row(
        modifier.entrance(MOTION_FAST, dy = 6.dp)
            .dropShadow(shape, Shadow(24.dp, color = Color.Black.copy(alpha = 0.35f), offset = DpOffset(0.dp, 8.dp)))
            .background(SurfaceRaised, shape)
            .border(1.dp, if (unseen > 0) Terracotta else BorderSubtle, shape)
            .padding(start = 14.dp, top = 6.dp, end = 6.dp, bottom = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(jumpToLatestLabel(unseen), Modifier.weight(1f, fill = false), color = chat.caper.android.ui.Text,
            fontSize = 13.sp, fontWeight = FontWeight.SemiBold, lineHeight = 16.sp)
        val name = jumpToLatestName(unseen)
        Button(
            onClick, Modifier.heightIn(min = 40.dp).clearAndSetSemantics {
                role = Role.Button
                contentDescription = name
                onClick { onClick(); true }
            },
            shape = RoundedCornerShape(10.dp), colors = primaryButtonColors(), contentPadding = PaddingValues(horizontal = 12.dp),
        ) { Text("Jump to latest", fontSize = 13.sp, fontWeight = FontWeight.Bold, maxLines = 1, softWrap = false) }
    }
}
