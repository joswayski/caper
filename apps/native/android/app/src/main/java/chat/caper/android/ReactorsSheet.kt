package chat.caper.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.fallbackReactionSummary
import chat.caper.android.data.reactionEmojiLabel
import chat.caper.android.data.reactionSummary
import chat.caper.android.data.reactorName
import chat.caper.android.model.*
import chat.caper.android.ui.*
import kotlinx.coroutines.CancellationException

internal const val ShowReactorsLabel = "Show who reacted"

/**
 * A reaction chip: a tap toggles your reaction, press and hold shows who reacted.
 * Read-only previews can still see who reacted but have nothing to toggle.
 */
@Composable internal fun ReactionChip(
    reaction: MessageReaction,
    selected: Boolean,
    canReact: Boolean,
    toggle: () -> Unit,
    showReactors: () -> Unit,
) {
    val shape = MaterialTheme.shapes.small
    val haptics = LocalHapticFeedback.current
    val show by rememberUpdatedState(showReactors)
    val gestures = if (canReact) Modifier.combinedClickable(
        onClick = toggle, onLongClick = showReactors, onLongClickLabel = ShowReactorsLabel, role = Role.Button,
    ) else Modifier.pointerInput(Unit) {
        detectTapGestures(onLongPress = { haptics.performHapticFeedback(HapticFeedbackType.LongPress); show() })
    }
    Row(
        Modifier
            .defaultMinSize(minWidth = 58.dp, minHeight = 48.dp)
            .clip(shape)
            .border(1.dp, if (selected) Terracotta else Border, shape)
            .background(if (selected) Terracotta.copy(alpha = .18f) else Color.Transparent)
            .then(gestures)
            .semantics(mergeDescendants = true) {
                this.selected = selected
                contentDescription = "${reaction.emoji} reaction, ${reaction.authorIds.size}"
                customActions = listOf(CustomAccessibilityAction(ShowReactorsLabel) { show(); true })
            }
            .padding(horizontal = 10.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        EmojiImage(reaction.emoji, null, Modifier.size(19.dp))
        Spacer(Modifier.width(5.dp))
        Text(
            reaction.authorIds.size.toString(),
            color = if (canReact) Terracotta else Text.copy(alpha = .38f),
            style = MaterialTheme.typography.labelLarge,
        )
    }
}

/**
 * Who reacted, opened by pressing and holding a chip: one tab per emoji in chip
 * order, then the people who reacted with the selected one in reaction order.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun ReactorsSheet(
    message: ChatMessage,
    initialEmoji: String,
    selfId: String?,
    load: suspend (ChatMessage) -> ReactorList,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    var selected by remember(message.id) { mutableStateOf(initialEmoji) }
    // A removed emoji falls back to the first remaining one; no reactions closes the sheet.
    val current = message.reactions.firstOrNull { it.emoji == selected } ?: message.reactions.firstOrNull()
    if (current == null) {
        LaunchedEffect(Unit) { onDismiss() }
        return
    }
    LaunchedEffect(current.emoji) { selected = current.emoji }

    var list by remember(message.id) { mutableStateOf<ReactorList?>(null) }
    var loading by remember(message.id) { mutableStateOf(true) }
    var failed by remember(message.id) { mutableStateOf(false) }
    var attempt by remember(message.id) { mutableIntStateOf(0) }
    // The loader reuses its cache until the snapshot's reaction revision changes.
    LaunchedEffect(message.id, message.reactionSeq, attempt) {
        list = null
        loading = true
        failed = false
        try {
            list = load(message)
            loading = false
        } catch (error: CancellationException) {
            throw error
        } catch (error: Throwable) {
            failed = true
            loading = false
        }
    }

    val authors = list?.reactions?.firstOrNull { it.emoji == current.emoji }?.authors
    val name = remember(current.emoji) { EmojiArtwork.name(context, current.emoji) }
    val summary = authors?.let { reactionSummary(it, selfId, name, current.emoji) }
        ?: fallbackReactionSummary(current.authorIds, selfId, name, current.emoji)

    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = SurfaceRaised, contentColor = Text) {
        Column(Modifier.fillMaxWidth().padding(bottom = 24.dp).semantics { paneTitle = "Reactions" }) {
            Text(
                "Reactions", Modifier.padding(horizontal = 18.dp).semantics { heading() },
                fontWeight = FontWeight.Bold, fontSize = 16.sp,
            )
            Spacer(Modifier.height(6.dp))
            ReactionTabs(message.reactions, current.emoji) { selected = it }
            HorizontalDivider(color = Border)
            Text(
                reactionEmojiLabel(name, current.emoji),
                Modifier.padding(start = 18.dp, end = 18.dp, top = 12.dp, bottom = 4.dp).semantics { contentDescription = summary },
                color = TextMuted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            when {
                failed -> Column(
                    Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text("Couldn’t load reactions", Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 13.sp)
                    OutlinedButton(
                        { attempt++ }, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border),
                        colors = ButtonDefaults.outlinedButtonColors(containerColor = SurfaceRaised, contentColor = Text),
                    ) { Text("Retry", fontSize = 12.sp) }
                }
                authors != null -> LazyColumn(Modifier.fillMaxWidth()) {
                    items(authors, key = { it.id }) { ReactorRow(it) }
                }
                loading -> Text("Loading…", Modifier.padding(horizontal = 18.dp, vertical = 8.dp), color = TextMuted, fontSize = 13.sp)
                // Loaded, but this emoji is still only a local, unsaved reaction.
                else -> Text(summary, Modifier.padding(horizontal = 18.dp, vertical = 8.dp), color = TextMuted, fontSize = 13.sp)
            }
        }
    }
}

@Composable private fun ReactionTabs(reactions: List<MessageReaction>, selected: String, select: (String) -> Unit) {
    val index = reactions.indexOfFirst { it.emoji == selected }.coerceAtLeast(0)
    val tabs = rememberLazyListState(initialFirstVisibleItemIndex = index)
    LaunchedEffect(index) {
        if (tabs.layoutInfo.visibleItemsInfo.none { it.index == index }) tabs.animateScrollToItem(index)
    }
    LazyRow(
        Modifier.fillMaxWidth().selectableGroup(), state = tabs,
        contentPadding = PaddingValues(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        items(reactions, key = { it.emoji }) { reaction ->
            val active = reaction.emoji == selected
            Box(
                Modifier
                    .clip(MaterialTheme.shapes.small)
                    .selectable(active, onClick = { select(reaction.emoji) }, role = Role.Tab)
                    .semantics { contentDescription = "${reaction.emoji} ${reaction.authorIds.size}" }
                    .height(48.dp)
                    .padding(horizontal = 12.dp)
                    // Terracotta marks the selected tab, as it marks selection elsewhere.
                    .drawBehind {
                        if (active) drawRect(
                            Terracotta, topLeft = Offset(0f, size.height - 2.dp.toPx()), size = Size(size.width, 2.dp.toPx()),
                        )
                    },
                contentAlignment = Alignment.Center,
            ) {
                Row(Modifier.clearAndSetSemantics {}, verticalAlignment = Alignment.CenterVertically) {
                    EmojiImage(reaction.emoji, null, Modifier.size(20.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(
                        reaction.authorIds.size.toString(), color = if (active) Text else TextMuted,
                        fontSize = 13.sp, fontWeight = FontWeight.Bold,
                    )
                }
            }
        }
    }
}

@Composable private fun ReactorRow(reactor: Reactor) {
    val name = reactorName(reactor)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(horizontal = 18.dp, vertical = 6.dp).semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Avatar(name, 34.dp, avatarId = reactor.avatarId)
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(name, fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            reactor.username?.takeIf { it.isNotBlank() }?.let {
                Text("@$it", color = TextMuted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}
