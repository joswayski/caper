package chat.caper.android

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
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import chat.caper.android.ui.LinkBlue

/** Visual-only padding preserves source text, wrapping, selection and Compose's link semantics. */
@Composable internal fun MessageLinkText(
    text: AnnotatedString,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    fontSize: TextUnit = TextUnit.Unspecified,
    lineHeight: TextUnit = TextUnit.Unspecified,
    maxLines: Int = Int.MAX_VALUE,
) {
    var layout by remember(text) { mutableStateOf<TextLayoutResult?>(null) }
    Text(
        text,
        modifier.drawBehind {
            val result = layout ?: return@drawBehind
            // Clickable mentions/history deliberately do not receive the URL treatment.
            drawLinkPills(result, text.getLinkAnnotations(0, text.length)
                .filter { it.item is LinkAnnotation.Url }.map { it.start until it.end })
        },
        color = color, fontSize = fontSize, lineHeight = lineHeight,
        overflow = if (maxLines == Int.MAX_VALUE) TextOverflow.Clip else TextOverflow.Ellipsis, maxLines = maxLines,
        inlineContent = messageInlineContent(), onTextLayout = { layout = it },
    )
}

/** Attached to the inner editor, so decorations scroll with text, not the field chrome. */
@Composable internal fun Modifier.draftLinkPills(layout: () -> TextLayoutResult?): Modifier {
    val icon = painterResource(R.drawable.lucide_link)
    return drawBehind {
        val result = layout() ?: return@drawBehind
        val text = result.layoutInput.text
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
private fun DrawScope.drawLinkPills(result: TextLayoutResult, ranges: List<IntRange>) {
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
                LinkBlue.copy(alpha = 0.10f),
                Offset(bounds.left - 2.dp.toPx(), bounds.top - 1.dp.toPx()),
                Size(bounds.width + 4.dp.toPx(), bounds.height + 2.dp.toPx()),
                CornerRadius(4.dp.toPx()),
            )
        }
    }
}
