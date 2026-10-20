package chat.caper.android

import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
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
            text.getLinkAnnotations(0, text.length).filter { it.item is LinkAnnotation.Url }.forEach { link ->
                if (link.start >= link.end) return@forEach
                val first = result.getLineForOffset(link.start)
                val last = result.getLineForOffset(link.end - 1)
                for (line in first..last) {
                    val start = maxOf(link.start, result.getLineStart(line))
                    val end = minOf(link.end, result.getLineEnd(line))
                    if (start >= end) continue
                    // The annotation includes the inline external-link placeholder, even if it wraps.
                    val bounds = result.getPathForRange(start, end).getBounds()
                    if (bounds.isEmpty) continue
                    val left = bounds.left - 2.dp.toPx()
                    val top = bounds.top - 1.dp.toPx()
                    val width = bounds.width + 4.dp.toPx()
                    val height = bounds.height + 2.dp.toPx()
                    // Low-opacity concentric halos give a faint glow without a bitmap or blur layer.
                    for (spread in 3 downTo 1) {
                        val halo = spread.dp.toPx()
                        drawRoundRect(
                            LinkBlue.copy(alpha = 0.012f),
                            Offset(left - halo, top - halo), Size(width + halo * 2, height + halo * 2),
                            CornerRadius(4.dp.toPx() + halo),
                        )
                    }
                    drawRoundRect(
                        LinkBlue.copy(alpha = 0.10f), Offset(left, top), Size(width, height),
                        CornerRadius(4.dp.toPx()),
                    )
                }
            }
        },
        color = color, fontSize = fontSize, lineHeight = lineHeight,
        overflow = if (maxLines == Int.MAX_VALUE) TextOverflow.Clip else TextOverflow.Ellipsis, maxLines = maxLines,
        inlineContent = messageInlineContent(), onTextLayout = { layout = it },
    )
}
