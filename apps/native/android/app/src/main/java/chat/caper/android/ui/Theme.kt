package chat.caper.android.ui

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.SwitchColors
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.material3.Typography
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.R

val Blackout = Color(0xFF0C0D0F)
val Surface = Color(0xFF151719)
val Border = Color(0xFF34383B)
val Text = Color(0xFFF3F4F5)
val Terracotta = Color(0xFFB64D32)
val CaperGreen = Color(0xFF637A43)
val VoiceSessionGreen = Color(0xFF4AA86B)
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
/** Text on a terracotta wash, such as your own reaction's count (web's #F2AE9C). */
val TerracottaLight = Color(0xFFF2AE9C)
val PinGold = Color(0xFFE4C76A)
val PinGoldWash = Color(0x0FE4C76A)
val Idle = Color(0xFFC58B3C)
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
        // Every role Material components read comes from the tokens; unset roles fall back to
        // Material's purple baseline (slider tracks, field labels, dividers, menus, sheets).
        colorScheme = darkColorScheme(
            primary = Terracotta,
            onPrimary = Color.White,
            primaryContainer = TerracottaDark,
            onPrimaryContainer = Text,
            inversePrimary = TerracottaBright,
            secondary = CaperGreen,
            onSecondary = Text,
            secondaryContainer = Border,
            onSecondaryContainer = Text,
            background = Blackout,
            onBackground = Text,
            surface = Surface,
            onSurface = Text,
            surfaceVariant = SurfaceRaised,
            onSurfaceVariant = TextMuted,
            inverseSurface = Text,
            inverseOnSurface = Blackout,
            error = ErrorText,
            onError = Blackout,
            outline = Border,
            outlineVariant = Border,
            surfaceDim = Blackout,
            surfaceBright = SurfaceComposer,
            surfaceContainerLowest = Blackout,
            surfaceContainerLow = Surface,
            surfaceContainer = SurfaceRaised,
            surfaceContainerHigh = SurfaceRaised,
            surfaceContainerHighest = SurfaceComposer,
        ),
        typography = Typography().run {
            copy(
                displayLarge = displayLarge.copy(fontFamily = Satoshi, letterSpacing = 0.sp), displayMedium = displayMedium.copy(fontFamily = Satoshi, letterSpacing = 0.sp), displaySmall = displaySmall.copy(fontFamily = Satoshi, letterSpacing = 0.sp),
                headlineLarge = headlineLarge.copy(fontFamily = Satoshi, letterSpacing = 0.sp), headlineMedium = headlineMedium.copy(fontFamily = Satoshi, letterSpacing = 0.sp), headlineSmall = headlineSmall.copy(fontFamily = Satoshi, letterSpacing = 0.sp),
                titleLarge = titleLarge.copy(fontFamily = Satoshi, letterSpacing = 0.sp), titleMedium = titleMedium.copy(fontFamily = Satoshi, letterSpacing = 0.sp), titleSmall = titleSmall.copy(fontFamily = Satoshi, letterSpacing = 0.sp),
                bodyLarge = bodyLarge.copy(fontFamily = Satoshi, letterSpacing = 0.sp), bodyMedium = bodyMedium.copy(fontFamily = Satoshi, letterSpacing = 0.sp), bodySmall = bodySmall.copy(fontFamily = Satoshi, letterSpacing = 0.sp),
                labelLarge = labelLarge.copy(fontFamily = Satoshi, letterSpacing = 0.sp), labelMedium = labelMedium.copy(fontFamily = Satoshi, letterSpacing = 0.sp), labelSmall = labelSmall.copy(fontFamily = Satoshi, letterSpacing = 0.sp),
            )
        },
        content = content,
    )
}

/**
 * Web's toggle: a light thumb on a border-colored track, terracotta when on. Material's
 * unchecked thumb is the outline color, which nearly vanishes on its dark track.
 */
@Composable fun caperSwitchColors(): SwitchColors = SwitchDefaults.colors(
    checkedThumbColor = Text, checkedTrackColor = Terracotta, checkedBorderColor = Terracotta,
    uncheckedThumbColor = Text, uncheckedTrackColor = Border, uncheckedBorderColor = Border,
)

/** Tabular figures (web's `tabular-nums`), so timers and counts keep their width as digits change. */
val TabularNumbers: TextStyle
    @Composable @ReadOnlyComposable get() = LocalTextStyle.current.merge(TextStyle(fontFeatureSettings = "tnum"))
