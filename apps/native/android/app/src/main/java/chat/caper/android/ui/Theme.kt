package chat.caper.android.ui

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.material3.Typography
import androidx.compose.ui.unit.dp
import chat.caper.android.R

val Blackout = Color(0xFF0C0D0F)
val Surface = Color(0xFF151719)
val Border = Color(0xFF34383B)
val Text = Color(0xFFF3F4F5)
val Terracotta = Color(0xFFB64D32)
val CaperGreen = Color(0xFF637A43)
val SurfaceSidebar = Color(0xFF151C1E)
val SurfaceConversation = Color(0xFF192123)
val SurfaceComposer = Color(0xFF283133)
val SurfaceRaised = Color(0xFF1C1F21)
val TextMuted = Color(0xFFB9BCBE)
val MessageText = Color(0xFFDEDFE0)
val TerracottaBright = Color(0xFFDB6849)
val TerracottaDark = Color(0xFF39231E)
val TerracottaBorder = Color(0xFF805143)
val TerracottaWash = Color(0x29B64D32)
val Idle = Color(0xFFB79A58)
val Danger = Color(0xFFB93646)
val ErrorText = Color(0xFFFF9B82)
private val ControlShape = RoundedCornerShape(8.dp)
private val Satoshi = FontFamily(
    Font(R.font.satoshi_regular, FontWeight.Normal),
    Font(R.font.satoshi_medium, FontWeight.Medium),
    Font(R.font.satoshi_bold, FontWeight.Bold),
    Font(R.font.satoshi_black, FontWeight.Black),
)

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
        typography = Typography().run {
            copy(
                displayLarge = displayLarge.copy(fontFamily = Satoshi), displayMedium = displayMedium.copy(fontFamily = Satoshi), displaySmall = displaySmall.copy(fontFamily = Satoshi),
                headlineLarge = headlineLarge.copy(fontFamily = Satoshi), headlineMedium = headlineMedium.copy(fontFamily = Satoshi), headlineSmall = headlineSmall.copy(fontFamily = Satoshi),
                titleLarge = titleLarge.copy(fontFamily = Satoshi), titleMedium = titleMedium.copy(fontFamily = Satoshi), titleSmall = titleSmall.copy(fontFamily = Satoshi),
                bodyLarge = bodyLarge.copy(fontFamily = Satoshi), bodyMedium = bodyMedium.copy(fontFamily = Satoshi), bodySmall = bodySmall.copy(fontFamily = Satoshi),
                labelLarge = labelLarge.copy(fontFamily = Satoshi), labelMedium = labelMedium.copy(fontFamily = Satoshi), labelSmall = labelSmall.copy(fontFamily = Satoshi),
            )
        },
        content = content,
    )
}
