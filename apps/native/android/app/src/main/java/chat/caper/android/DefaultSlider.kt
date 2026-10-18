package chat.caper.android

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Slider
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import chat.caper.android.ui.TextMuted
import kotlin.math.abs

/** A native slider with an explicit home position, without quantizing keyboard or TalkBack input. */
@Composable
internal fun DefaultSlider(
    value: Float,
    onValueChange: (Float) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    valueRange: ClosedFloatingPointRange<Float>,
    defaultValue: Float,
) {
    var pointerDown by remember { mutableStateOf(false) }
    Slider(
        value = value,
        onValueChange = { next ->
            val radius = (valueRange.endInclusive - valueRange.start) * 0.02f
            onValueChange(if (pointerDown && abs(next - defaultValue) <= radius) defaultValue else next)
        },
        enabled = enabled,
        valueRange = valueRange,
        modifier = modifier.pointerInput(Unit) {
            try {
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent(PointerEventPass.Initial)
                        pointerDown = event.changes.any { it.pressed }
                    }
                }
            } finally {
                pointerDown = false
            }
        }.drawWithContent {
            drawContent()
            // A short neutral notch below the track stays visible even beneath the thumb.
            val fraction = (defaultValue - valueRange.start) / (valueRange.endInclusive - valueRange.start)
            val position = if (layoutDirection == LayoutDirection.Rtl) 1f - fraction else fraction
            // Material 3's 4dp thumb reserves half its width at each end.
            val inset = 2.dp.toPx()
            val x = inset + (size.width - 2 * inset) * position
            val y = size.height - 4.dp.toPx()
            drawLine(TextMuted.copy(alpha = if (enabled) 0.8f else 0.35f), Offset(x, y), Offset(x, y + 4.dp.toPx()), 2.dp.toPx())
        }.padding(bottom = 6.dp),
    )
}
