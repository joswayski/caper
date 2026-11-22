package chat.caper.android.ui

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
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

// BEGIN GENERATED THEME TOKENS
val Blackout = Color(0xFF0B0B0C)
val Surface = Color(0xFF121214)
val Border = Color(0xFF29292F)
val BorderSubtle = Color(0x17FFFFFF)
val SurfaceRim = Color(0x14FFFFFF)
val Text = Color(0xFFF4F3EF)
val Terracotta = Color(0xFF557A38)
/** Primary actions are warm chalk white with obsidian labels (shared/themes/obsidian.json). */
val Action = Color(0xFFF4F3EF)
val OnAction = Color(0xFF0B0B0C)
val CaperGreen = Color(0xFF6F9948)
// Caper green mixed into Border: replaces a panel divider while its resize handle is in use.
val ResizeHighlight = Color(0xFF485D38)
val VoiceSessionGreen = Color(0xFF4ADE80)
val SurfaceSidebar = Color(0xFF0F0F11)
/** Web's --surface-hover (6% white) over the sidebar, opaque: a private channel's own members in the member list. */
val SurfaceSidebarRaised = Color(0xFF1C1C20)
val SurfaceConversation = Color(0xFF131316)
val SurfaceComposer = Color(0xFF1D1D22)
val SurfaceRaised = Color(0xFF1B1B1F)
val TextMuted = Color(0xFF9A99A2)
val MessageText = Color(0xFFE6E5E0)
val LinkBlue = Color(0xFF7CC7FF)
val LinkBlueActive = Color(0xFFB2DFFF)
val TerracottaBright = Color(0xFF87BC5C)
val TerracottaDark = Color(0xFF1C2816)
val TerracottaBorder = Color(0xFF486832)
val TerracottaWash = Color(0x29557A38)
/** Text on an accent wash, such as your own reaction's count. */
val TerracottaLight = Color(0xFFC3E4A4)
val PinGold = Color(0xFFE4C76A)
val PinGoldWash = Color(0x0FE4C76A)
val Idle = Color(0xFFD6A44E)
val Danger = Color(0xFFC83C4E)
val ErrorText = Color(0xFFFF9B82)
// END GENERATED THEME TOKENS
private val ControlShape = RoundedCornerShape(10.dp)
private val Figtree = FontFamily(
    Font(R.font.figtree_regular, FontWeight.Normal),
    Font(R.font.figtree_medium, FontWeight.Medium),
    Font(R.font.figtree_bold, FontWeight.Bold),
    Font(R.font.figtree_black, FontWeight.Black),
)
private val Bricolage = FontFamily(
    Font(R.font.bricolage_bold, FontWeight.Bold),
    Font(R.font.bricolage_extrabold, FontWeight.ExtraBold),
    Font(R.font.bricolage_extrabold, FontWeight.Black),
)

@Composable fun CaperTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        shapes = Shapes(
            extraSmall = ControlShape,
            small = ControlShape,
            medium = RoundedCornerShape(13.dp),
            large = RoundedCornerShape(15.dp),
            extraLarge = RoundedCornerShape(16.dp),
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
                displayLarge = displayLarge.copy(fontFamily = Bricolage, letterSpacing = (-0.4).sp), displayMedium = displayMedium.copy(fontFamily = Bricolage, letterSpacing = (-0.3).sp), displaySmall = displaySmall.copy(fontFamily = Bricolage, letterSpacing = (-0.25).sp),
                headlineLarge = headlineLarge.copy(fontFamily = Bricolage, letterSpacing = (-0.3).sp), headlineMedium = headlineMedium.copy(fontFamily = Bricolage, letterSpacing = (-0.25).sp), headlineSmall = headlineSmall.copy(fontFamily = Bricolage, letterSpacing = (-0.2).sp),
                titleLarge = titleLarge.copy(fontFamily = Bricolage, letterSpacing = (-0.2).sp), titleMedium = titleMedium.copy(fontFamily = Figtree, letterSpacing = 0.sp), titleSmall = titleSmall.copy(fontFamily = Figtree, letterSpacing = 0.sp),
                bodyLarge = bodyLarge.copy(fontFamily = Figtree, letterSpacing = 0.sp), bodyMedium = bodyMedium.copy(fontFamily = Figtree, letterSpacing = 0.sp), bodySmall = bodySmall.copy(fontFamily = Figtree, letterSpacing = 0.sp),
                labelLarge = labelLarge.copy(fontFamily = Figtree, letterSpacing = 0.sp), labelMedium = labelMedium.copy(fontFamily = Figtree, letterSpacing = 0.sp), labelSmall = labelSmall.copy(fontFamily = Figtree, letterSpacing = 0.sp),
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

/**
 * Web's primary action (shared/design.css --action): an eggshell fill with a blackout label.
 * Disabled, it steps back to neutral chrome rather than a faded fill. Theme `primary` stays
 * terracotta for selection, focus and text buttons.
 */
@Composable fun primaryButtonColors(): ButtonColors = ButtonDefaults.buttonColors(
    containerColor = Action, contentColor = OnAction, disabledContainerColor = SurfaceRaised, disabledContentColor = TextMuted,
)

/**
 * Secondary and dismissive actions (Cancel, Close, Decline, Reset, Back): neutral labels, as on web.
 * Material's text and outlined buttons otherwise take theme `primary`, which stays terracotta.
 */
@Composable fun neutralTextButtonColors(contentColor: Color = Text): ButtonColors = ButtonDefaults.textButtonColors(contentColor = contentColor)

/** [neutralTextButtonColors] for bordered secondary actions. */
@Composable fun neutralOutlinedButtonColors(): ButtonColors = ButtonDefaults.outlinedButtonColors(contentColor = Text)
