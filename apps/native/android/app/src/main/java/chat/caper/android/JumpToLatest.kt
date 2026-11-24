package chat.caper.android

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Icon
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextAlign
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

/** How many rows below the viewport show a live list's toast: reading the last few is still reading the newest. */
internal const val JUMP_AFTER_ROWS = 10

/**
 * Whether a live list's Jump to latest toast shows: once [below] (rows wholly below the viewport) reaches
 * [JUMP_AFTER_ROWS]; once shown it stays until the reader is back within [threshold] of the bottom. A detached window
 * ([live] false) shows its own, uncounted toast instead (see [jumpToastCount]).
 */
internal fun jumpToLatestShown(shown: Boolean, live: Boolean, below: Int, distance: Int, threshold: Int): Boolean =
    live && if (shown) distance > threshold else below >= JUMP_AFTER_ROWS

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

/** The rows that start below the viewport: laid out ones past its end, and every row not yet laid out after them. */
internal fun LazyListLayoutInfo.rowsBelow(): Int {
    val visible = visibleItemsInfo
    val last = visible.lastOrNull() ?: return 0
    val end = viewportEndOffset - afterContentPadding
    return visible.count { it.offset >= end } + (totalItemsCount - 1 - last.index).coerceAtLeast(0)
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
@Composable internal fun BoxScope.JumpToLatestOverlay(jump: JumpToLatest, detached: Boolean, onJump: () -> Unit, onReturn: () -> Unit) {
    val count = jumpToastCount(detached, jump.shown, jump.unseen) ?: return
    // Phones get Discord's arrow at the right edge: the full toast covers too much of a short conversation.
    if (LocalConfiguration.current.screenWidthDp < 600) {
        JumpToLatestArrow(count, if (detached) onReturn else onJump, Modifier.align(Alignment.BottomEnd).padding(end = 12.dp, bottom = 12.dp))
    } else {
        JumpToLatestToast(count, if (detached) onReturn else onJump,
            Modifier.align(Alignment.BottomCenter).padding(start = 12.dp, end = 12.dp, bottom = 12.dp))
    }
}

/** The phone form: a 44dp raised arrow, its border terracotta and a count badge on its corner once others write. */
@Composable internal fun JumpToLatestArrow(unseen: Int, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val shape = RoundedCornerShape(10.dp)
    val name = jumpToLatestName(unseen)
    Box(modifier.entrance(MOTION_FAST, dy = 6.dp)) {
        Box(
            Modifier.size(44.dp)
                .dropShadow(shape, Shadow(24.dp, color = Color.Black.copy(alpha = 0.35f), offset = DpOffset(0.dp, 8.dp)))
                .clip(shape).background(SurfaceRaised)
                .border(1.dp, if (unseen > 0) Terracotta else BorderSubtle, shape)
                .clickable(onClick = onClick)
                .clearAndSetSemantics {
                    role = Role.Button
                    contentDescription = name
                    onClick { onClick(); true }
                },
            contentAlignment = Alignment.Center,
        ) { Icon(painterResource(R.drawable.lucide_arrow_down), null, Modifier.size(20.dp), tint = chat.caper.android.ui.Text) }
        if (unseen > 0) Text(
            if (unseen > 99) "99+" else unseen.toString(),
            Modifier.align(Alignment.TopEnd).offset(x = 5.dp, y = (-7).dp).heightIn(min = 18.dp).widthIn(min = 18.dp)
                .background(Terracotta, RoundedCornerShape(50)).padding(horizontal = 5.dp).clearAndSetSemantics { },
            color = chat.caper.android.ui.Text, fontSize = 10.sp, fontWeight = FontWeight.Bold, lineHeight = 18.sp,
            textAlign = TextAlign.Center,
        )
    }
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
