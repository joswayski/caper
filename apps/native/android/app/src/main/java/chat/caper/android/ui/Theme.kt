package chat.caper.android.ui

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

val Blackout = Color(0xFF0C0D0F)
val Surface = Color(0xFF151719)
val Border = Color(0xFF34383B)
val Text = Color(0xFFF3F4F5)
val Terracotta = Color(0xFFB64D32)
val CaperGreen = Color(0xFF637A43)
private val ControlShape = RoundedCornerShape(8.dp)

@Composable fun CaperTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        shapes = Shapes(
            extraSmall = ControlShape,
            small = ControlShape,
            medium = ControlShape,
            large = ControlShape,
            extraLarge = ControlShape,
        ),
        colorScheme = darkColorScheme(
            primary = Terracotta,
            secondary = CaperGreen,
            background = Blackout,
            surface = Surface,
            outline = Border,
            onPrimary = Color.White,
            onBackground = Text,
            onSurface = Text,
        ),
        content = content,
    )
}
