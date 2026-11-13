package chat.caper.android

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.ParagraphStyle
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.ResolvedTextDirection
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextIndent
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.FormattedRun
import chat.caper.android.ui.Border
import chat.caper.android.ui.LinkBlue

/**
 * Visual-only padding preserves source text, wrapping, selection and Compose's link semantics.
 * Also draws [QuoteBlock] bars and [SpoilerText] backgrounds; while a spoiler is hidden, TalkBack hears
 * "Spoiler, reveal" in its place and gets a custom action that reveals it, beside [actions].
 */
@Composable internal fun MessageLinkText(
    text: AnnotatedString,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    fontSize: TextUnit = TextUnit.Unspecified,
    lineHeight: TextUnit = TextUnit.Unspecified,
    maxLines: Int = Int.MAX_VALUE,
    actions: List<CustomAccessibilityAction> = emptyList(),
) {
    var layout by remember(text) { mutableStateOf<TextLayoutResult?>(null) }
    val spoilers = remember(text) { text.getStringAnnotations(SpoilerText, 0, text.length) }
    val reveals = remember(text) {
        text.getLinkAnnotations(0, text.length).mapNotNull { link ->
            val item = link.item as? LinkAnnotation.Clickable ?: return@mapNotNull null
            if (item.tag != SpoilerReveal) return@mapNotNull null
            CustomAccessibilityAction(SPOILER_REVEAL_LABEL) { item.linkInteractionListener?.onClick(item); true }
        }
    }
    // Web fades a revealed spoiler's background from the hidden one, over the fast motion token.
    val still = reducedMotion()
    val shown = spoilers.filter { it.item == SPOILER_SHOWN }.map { it.start until it.end }.toSet()
    var previous by remember { mutableStateOf(shown) }
    val revealing = remember { Animatable(1f) }
    val arriving = remember(shown) { shown - previous }
    LaunchedEffect(shown) {
        previous = shown
        if (arriving.isNotEmpty() && !still) { revealing.snapTo(0f); revealing.animateTo(1f, tween(MOTION_FAST, easing = EaseOut)) }
    }
    val described = if (reveals.isEmpty() && actions.isEmpty()) Modifier else Modifier.semantics {
        customActions = actions + reveals
        if (reveals.isNotEmpty()) contentDescription = spokenText(text)
    }
    Text(
        text,
        modifier.then(described).drawBehind {
            val result = layout ?: return@drawBehind
            // Until the fade starts, a just-revealed spoiler keeps its hidden colour rather than flashing.
            drawFormatting(result) { range -> if (range !in arriving) null else if (previous != shown) Border else lerp(Border, SpoilerShown, revealing.value) }
            // Clickable mentions/history deliberately do not receive the URL treatment.
            drawLinkPills(result, text.getLinkAnnotations(0, text.length)
                .filter { it.item is LinkAnnotation.Url }.map { it.start until it.end })
        },
        color = color, fontSize = fontSize, lineHeight = lineHeight,
        overflow = if (maxLines == Int.MAX_VALUE) TextOverflow.Clip else TextOverflow.Ellipsis, maxLines = maxLines,
        inlineContent = messageInlineContent(), onTextLayout = { layout = it },
    )
}

/** String annotation over a quote block's text, drawn with a bar beside it. */
internal const val QuoteBlock = "quote"
/** String annotation over one spoiler: [SPOILER_HIDDEN], [SPOILER_SHOWN] or, in a draft, [SPOILER_DRAFT]. */
internal const val SpoilerText = "spoiler"
internal const val SPOILER_HIDDEN = "hidden"
internal const val SPOILER_SHOWN = "shown"
internal const val SPOILER_DRAFT = "draft"
/** Tag of the link that reveals a hidden spoiler. */
internal const val SpoilerReveal = "spoiler"
internal const val SPOILER_REVEAL_LABEL = "Spoiler, reveal"
private val SpoilerShown = Color.White.copy(alpha = 0.06f)
private val QuoteParagraph = ParagraphStyle(textIndent = TextIndent(15.sp, 15.sp))

/** A run's inline styles, or null without any; [drawFormatting] draws its spoiler and quote. */
internal fun FormattedRun.spanStyle(): SpanStyle? {
    if (!bold && !italic && !underline && !strikethrough) return null
    return SpanStyle(
        fontWeight = if (bold) FontWeight.Bold else null,
        fontStyle = if (italic) FontStyle.Italic else null,
        textDecoration = listOfNotNull(TextDecoration.Underline.takeIf { underline }, TextDecoration.LineThrough.takeIf { strikethrough })
            .takeIf { it.isNotEmpty() }?.let { TextDecoration.combine(it) },
    )
}

/** What TalkBack reads while spoilers are hidden: each in its place as "Spoiler, reveal", without icon placeholders. */
private fun spokenText(text: AnnotatedString): String {
    val hidden = text.getStringAnnotations(SpoilerText, 0, text.length).filter { it.item == SPOILER_HIDDEN }
    return buildString {
        var cursor = 0
        for (spoiler in hidden) {
            append(text.text, cursor, spoiler.start)
            append(SPOILER_REVEAL_LABEL)
            cursor = spoiler.end
        }
        append(text.text, cursor, text.length)
    }.replace("\uFFFD", "").replace(QUOTE_BREAK, '\n')
}

private const val QUOTE_BREAK = '\u200B'

/**
 * [text] (displayed text) for quote [blocks]: each block becomes its own paragraph, and Compose already
 * breaks lines between paragraphs, so the plain newline at a block's edge is drawn as a zero-width
 * space. Lengths, and so every offset, stay the same.
 */
internal fun quoteBreaks(text: String, blocks: List<IntRange>): String {
    if (blocks.isEmpty()) return text
    val chars = text.toCharArray()
    for (block in blocks) {
        if (block.first > 0 && chars[block.first - 1] == '\n') chars[block.first - 1] = QUOTE_BREAK
        if (block.last + 1 < chars.size && chars[block.last + 1] == '\n') chars[block.last + 1] = QUOTE_BREAK
    }
    return String(chars)
}

/**
 * Indents quote block [start]..[end] of [text] (built from [quoteBreaks]) and marks it for its bar. The break
 * after it joins its paragraph, so a caret before that break still sits at the quote's end.
 */
internal fun AnnotatedString.Builder.addQuoteBlock(text: CharSequence, start: Int, end: Int) {
    addStyle(QuoteParagraph, start, if (end < text.length && text[end] == QUOTE_BREAK) end + 1 else end)
    addStringAnnotation(QuoteBlock, "", start, end)
}

/**
 * A 3dp bar beside each quote block and rounded backgrounds behind spoilers, as web's
 * `.chat-quote` and `.chat-spoiler`. [spoilerColor] can override a spoiler's colour.
 */
internal fun DrawScope.drawFormatting(result: TextLayoutResult, spoilerColor: (IntRange) -> Color? = { null }) {
    val text = result.layoutInput.text
    text.getStringAnnotations(SpoilerText, 0, text.length).forEach { spoiler ->
        val range = spoiler.start until spoiler.end
        val color = spoilerColor(range) ?: if (spoiler.item == SPOILER_SHOWN) SpoilerShown else Border
        drawPills(result, listOf(range), color, 0f, 0f)
    }
    text.getStringAnnotations(QuoteBlock, 0, text.length).forEach { block ->
        if (block.start >= block.end) return@forEach
        val first = result.getLineForOffset(block.start)
        val last = result.getLineForOffset(block.end - 1)
        if (first >= result.lineCount) return@forEach
        val top = result.getLineTop(first)
        val bottom = result.getLineBottom(minOf(last, result.lineCount - 1))
        val width = 3.dp.toPx()
        val x = if (result.getParagraphDirection(block.start) == ResolvedTextDirection.Rtl) size.width - width else 0f
        drawRect(Border, Offset(x, top), Size(width, bottom - top))
    }
}

/** Attached to the inner editor, so decorations scroll with text, not the field chrome. */
@Composable internal fun Modifier.draftLinkPills(layout: () -> TextLayoutResult?): Modifier {
    val icon = painterResource(R.drawable.lucide_link)
    return drawBehind {
        val result = layout() ?: return@drawBehind
        val text = result.layoutInput.text
        drawFormatting(result)
        val links = text.getStringAnnotations(DraftLinkPill, 0, text.length)
        drawLinkPills(result, links.map { it.start until it.end })
        links.forEach { link ->
            val bounds = result.getBoundingBox(link.start)
            // Scale with the reserved em (and therefore with the user's text size).
            val side = minOf(bounds.width * 0.78f, bounds.height)
            translate(bounds.left, bounds.center.y - side / 2) {
                with(icon) { draw(Size(side, side), colorFilter = ColorFilter.tint(LinkBlue)) }
            }
        }
    }
}

/** One subtly rounded pill per wrapped line, shared by drafts and posted URLs. */
private fun DrawScope.drawLinkPills(result: TextLayoutResult, ranges: List<IntRange>) =
    drawPills(result, ranges, LinkBlue.copy(alpha = 0.10f), 2.dp.toPx(), 1.dp.toPx())

private fun DrawScope.drawPills(result: TextLayoutResult, ranges: List<IntRange>, color: Color, padX: Float, padY: Float) {
    ranges.forEach { range ->
        if (range.isEmpty()) return@forEach
        val first = result.getLineForOffset(range.first)
        val last = result.getLineForOffset(range.last)
        for (line in first..last) {
            val start = maxOf(range.first, result.getLineStart(line))
            val end = minOf(range.last + 1, result.getLineEnd(line, visibleEnd = true))
            if (start >= end) continue
            val bounds = result.getPathForRange(start, end).getBounds()
            if (bounds.isEmpty) continue
            drawRoundRect(
                color,
                Offset(bounds.left - padX, bounds.top - padY),
                Size(bounds.width + padX * 2, bounds.height + padY * 2),
                CornerRadius(4.dp.toPx()),
            )
        }
    }
}
