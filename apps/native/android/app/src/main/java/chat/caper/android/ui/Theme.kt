package chat.caper.android.ui

import android.content.Context
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.SwitchColors
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.material3.Typography
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.R
import org.json.JSONObject

// BEGIN GENERATED THEME TOKENS
val Blackout = Color(0xFF090A0C)
val Surface = Color(0xFF111216)
val Border = Color(0xFF24262F)
val BorderSubtle = Color(0x17FFFFFF)
val SurfaceRim = Color(0x14FFFFFF)
val Text = Color(0xFFF4F4F6)
val Terracotta = Color(0xFF4E7835)
/** Primary actions are warm chalk white with obsidian labels (shared/themes/obsidian.json). */
val Action = Color(0xFFF4F4F6)
val OnAction = Color(0xFF090A0C)
val CaperGreen = Color(0xFF638E46)
// Caper green mixed into Border: replaces a panel divider while its resize handle is in use.
val ResizeHighlight = Color(0xFF3D5431)
val VoiceSessionGreen = Color(0xFF4CC970)
val SurfaceSidebar = Color(0xFF0D0E11)
/** Web's --surface-hover (6% white) over the sidebar, opaque: a private channel's own members in the member list. */
val SurfaceSidebarRaised = Color(0xFF17191F)
val SurfaceConversation = Color(0xFF121317)
val SurfaceComposer = Color(0xFF1B1D24)
val SurfaceRaised = Color(0xFF191B22)
val TextMuted = Color(0xFF8F919C)
val MessageText = Color(0xFFE4E5EA)
val LinkBlue = Color(0xFF6EB8FF)
val LinkBlueActive = Color(0xFFA3D3FF)
val TerracottaBright = Color(0xFF77A659)
val TerracottaDark = Color(0xFF172314)
val TerracottaBorder = Color(0xFF3F612C)
val TerracottaWash = Color(0x294E7835)
/** Text on an accent wash, such as your own reaction's count. */
val TerracottaLight = Color(0xFFB9D9A3)
val PinGold = Color(0xFFE2C36B)
val PinGoldWash = Color(0x0FE2C36B)
val Idle = Color(0xFFD8A64F)
val Danger = Color(0xFFCF4254)
val ErrorText = Color(0xFFFF9685)
// END GENERATED THEME TOKENS
/** A muted microphone or deafened output (shared themes' status.voiceOff), as web and Discord show it. */
val VoiceOff = Color(0xFFFF6E7F)
private val ControlShape = RoundedCornerShape(10.dp)
val PillShape = RoundedCornerShape(999.dp)
private val Figtree = FontFamily(
    Font(R.font.figtree_regular, FontWeight.Normal),
    Font(R.font.figtree_medium, FontWeight.Medium),
    Font(R.font.figtree_bold, FontWeight.Bold),
    Font(R.font.figtree_black, FontWeight.Black),
)
private val Nunito = FontFamily(
    Font(R.font.nunito_bold, FontWeight.Bold),
    Font(R.font.nunito_extrabold, FontWeight.ExtraBold),
    Font(R.font.nunito_extrabold, FontWeight.Black),
)

data class CaperThemePalette(
    val id: String,
    val name: String,
    val isLight: Boolean,
    val blackout: Color,
    val surface: Color,
    val surfaceRaised: Color,
    val surfaceComposer: Color,
    val border: Color,
    val text: Color,
    val textMuted: Color,
    val action: Color,
    val onAction: Color,
    val accent: Color,
    val accentBright: Color,
    val accentSurface: Color,
    val caperGreen: Color,
    val errorText: Color,
    val rawJson: String?,
)

object CaperThemeStore {
    private const val PREFS_NAME = "caper_theme_prefs"
    private const val PREF_KEY = "caper.theme.v1"

    val obsidian = CaperThemePalette(
        id = "obsidian",
        name = "Obsidian",
        isLight = false,
        blackout = Blackout,
        surface = Surface,
        surfaceRaised = SurfaceRaised,
        surfaceComposer = SurfaceComposer,
        border = Border,
        text = Text,
        textMuted = TextMuted,
        action = Action,
        onAction = OnAction,
        accent = Terracotta,
        accentBright = TerracottaBright,
        accentSurface = TerracottaDark,
        caperGreen = CaperGreen,
        errorText = ErrorText,
        rawJson = null,
    )

    val paper = CaperThemePalette(
        id = "paper",
        name = "Paper",
        isLight = true,
        blackout = Color(0xFFECEEF2),
        surface = Color(0xFFFFFFFF),
        surfaceRaised = Color(0xFFF4F5F8),
        surfaceComposer = Color(0xFFE9EBF0),
        border = Color(0xFFDCDFE6),
        text = Color(0xFF111217),
        textMuted = Color(0xFF656876),
        action = Color(0xFF111217),
        onAction = Color(0xFFFFFFFF),
        accent = Color(0xFF456C2D),
        accentBright = Color(0xFF3A5D24),
        accentSurface = Color(0xFFEAF1E5),
        caperGreen = Color(0xFF567D3B),
        errorText = Color(0xFFC23244),
        rawJson = null,
    )

    var active by mutableStateOf(obsidian)
        private set
    private var initialized = false

    fun init(context: Context) {
        if (initialized) return
        initialized = true
        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val saved = prefs.getString(PREF_KEY, null)
        if (!saved.isNullOrBlank()) {
            parsePreference(saved).onSuccess { parsed ->
                val normalized = if (parsed.id == "paper" || parsed.isLight) paper else obsidian
                active = normalized
                if (saved != normalized.id) {
                    prefs.edit().putString(PREF_KEY, normalized.id).apply()
                }
            }
        }
    }

    fun selectPreset(context: Context, id: String) {
        val next = if (id == "paper") paper else obsidian
        active = next
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit().putString(PREF_KEY, next.id).apply()
    }

    fun importThemeJson(context: Context, raw: String): Result<CaperThemePalette> {
        return parsePreference(raw).onSuccess { palette ->
            active = palette
            val stored = palette.rawJson ?: palette.id
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit().putString(PREF_KEY, stored).apply()
        }
    }

    fun exportCurrentJson(): String {
        active.rawJson?.let { return it }
        val p = active
        val hex: (Color) -> String = { c ->
            val r = (c.red * 255f + 0.5f).toInt().coerceIn(0, 255)
            val g = (c.green * 255f + 0.5f).toInt().coerceIn(0, 255)
            val b = (c.blue * 255f + 0.5f).toInt().coerceIn(0, 255)
            String.format("#%02x%02x%02x", r, g, b)
        }
        val doc = JSONObject()
        doc.put("schema", "caper.theme.v1")
        doc.put("id", p.id)
        doc.put("name", p.name)
        doc.put("appearance", if (p.isLight) "light" else "dark")
        doc.put("surfaces", JSONObject().apply {
            put("frame", hex(p.blackout))
            put("base", hex(p.surface))
            put("sidebar", hex(p.blackout))
            put("sidebarRaised", hex(p.surfaceRaised))
            put("conversation", hex(p.surface))
            put("raised", hex(p.surfaceRaised))
            put("composer", hex(p.surfaceComposer))
            put("replyDraft", hex(p.surfaceRaised))
            put("hover", if (p.isLight) "rgb(11 11 12 / 5%)" else "rgb(255 255 255 / 6%)")
            put("hoverAlpha", if (p.isLight) 0.05 else 0.06)
        })
        doc.put("borders", JSONObject().apply {
            put("default", hex(p.border))
            put("translucent", if (p.isLight) "rgb(11 11 12 / 9%)" else "rgb(255 255 255 / 9%)")
            put("subtle", if (p.isLight) "rgb(11 11 12 / 6%)" else "rgb(255 255 255 / 6%)")
            put("rim", if (p.isLight) "rgb(255 255 255 / 85%)" else "rgb(255 255 255 / 8%)")
        })
        doc.put("text", JSONObject().apply {
            put("primary", hex(p.text))
            put("message", hex(p.text))
            put("muted", hex(p.textMuted))
            put("link", if (p.isLight) "#1d6fe2" else "#7cc7ff")
            put("linkHover", if (p.isLight) "#1456b8" else "#b2dfff")
        })
        doc.put("action", JSONObject().apply {
            put("primary", hex(p.action))
            put("primaryHover", hex(p.action))
            put("primaryPress", hex(p.action))
            put("onPrimary", hex(p.onAction))
        })
        doc.put("accent", JSONObject().apply {
            put("primary", hex(p.accent))
            put("bright", hex(p.accentBright))
            put("softText", hex(p.accentBright))
            put("surface", hex(p.accentSurface))
            put("border", hex(p.accent))
        })
        doc.put("brand", JSONObject().apply {
            put("caper", hex(p.caperGreen))
            put("caperBright", hex(p.accentBright))
            put("resizeHighlight", hex(p.accent))
        })
        doc.put("status", JSONObject().apply {
            put("voice", if (p.isLight) "#15803d" else "#4ade80")
            put("pin", if (p.isLight) "#b58918" else "#e4c76a")
            put("pinWash", if (p.isLight) "rgb(181 137 24 / 10%)" else "rgb(228 199 106 / 6%)")
            put("idle", if (p.isLight) "#b47819" else "#d6a44e")
            put("errorText", hex(p.errorText))
            put("danger", "#c83c4e")
            put("dangerHover", if (p.isLight) "#ad2b3d" else "#e04c5f")
            put("dangerWash", if (p.isLight) "rgb(200 60 78 / 12%)" else "rgb(200 60 78 / 16%)")
            put("onDanger", "#ffffff")
            put("voiceOff", if (p.isLight) "#d63b4e" else "#ff7182")
            put("voiceOffIcon", if (p.isLight) "#d63b4e" else "#ff7182")
            put("voiceOffWash", if (p.isLight) "rgb(214 59 78 / 12%)" else "rgb(224 76 95 / 18%)")
        })
        doc.put("radii", JSONObject().apply {
            put("canvas", 16); put("card", 13); put("composer", 15); put("control", 10); put("pill", 999); put("avatar", "30%")
        })
        return doc.toString(2) + "\n"
    }

    private fun parseHexColor(value: String): Color? {
        if (!value.startsWith("#") || value.length != 7) return null
        val rgb = value.substring(1).toLongOrNull(16) ?: return null
        return Color(0xFF000000L or rgb)
    }

    fun parsePreference(raw: String): Result<CaperThemePalette> = runCatching {
        val trimmed = raw.trim()
        if (trimmed.equals("obsidian", ignoreCase = true)) return@runCatching obsidian
        if (trimmed.equals("paper", ignoreCase = true)) return@runCatching paper
        val root = JSONObject(trimmed)
        require(root.optString("schema") == "caper.theme.v1") { "Theme schema must be \"caper.theme.v1\"." }
        val appearance = root.optString("appearance")
        require(appearance == "dark" || appearance == "light") { "Theme appearance must be \"dark\" or \"light\"." }
        val id = root.optString("id").ifBlank { "custom" }
        val name = root.optString("name").ifBlank { "Custom" }
        val surfaces = root.getJSONObject("surfaces")
        val borders = root.getJSONObject("borders")
        val text = root.getJSONObject("text")
        val action = root.getJSONObject("action")
        val accent = root.getJSONObject("accent")
        val brand = root.getJSONObject("brand")
        val status = root.getJSONObject("status")
        fun req(obj: JSONObject, key: String): Color =
            parseHexColor(obj.optString(key)) ?: throw IllegalArgumentException("Invalid hex color for $key.")
        CaperThemePalette(
            id = id,
            name = name,
            isLight = appearance == "light",
            blackout = req(surfaces, "frame"),
            surface = req(surfaces, "base"),
            surfaceRaised = req(surfaces, "raised"),
            surfaceComposer = req(surfaces, "composer"),
            border = req(borders, "default"),
            text = req(text, "primary"),
            textMuted = req(text, "muted"),
            action = req(action, "primary"),
            onAction = req(action, "onPrimary"),
            accent = req(accent, "primary"),
            accentBright = req(accent, "bright"),
            accentSurface = req(accent, "surface"),
            caperGreen = req(brand, "caper"),
            errorText = req(status, "errorText"),
            rawJson = root.toString(2) + "\n",
        )
    }
}

@Composable fun CaperTheme(content: @Composable () -> Unit) {
    val context = LocalContext.current
    LaunchedEffect(context) { CaperThemeStore.init(context) }
    val p = CaperThemeStore.active
    val scheme = if (p.isLight) {
        lightColorScheme(
            primary = p.accent,
            onPrimary = Color.White,
            primaryContainer = p.accentSurface,
            onPrimaryContainer = p.text,
            inversePrimary = p.accentBright,
            secondary = p.caperGreen,
            onSecondary = Color.White,
            secondaryContainer = p.border,
            onSecondaryContainer = p.text,
            background = p.blackout,
            onBackground = p.text,
            surface = p.surface,
            onSurface = p.text,
            surfaceVariant = p.surfaceRaised,
            onSurfaceVariant = p.textMuted,
            inverseSurface = p.text,
            inverseOnSurface = p.blackout,
            error = p.errorText,
            onError = Color.White,
            outline = p.border,
            outlineVariant = p.border,
            surfaceDim = p.blackout,
            surfaceBright = p.surfaceComposer,
            surfaceContainerLowest = p.blackout,
            surfaceContainerLow = p.surface,
            surfaceContainer = p.surfaceRaised,
            surfaceContainerHigh = p.surfaceRaised,
            surfaceContainerHighest = p.surfaceComposer,
        )
    } else {
        darkColorScheme(
            primary = p.accent,
            onPrimary = Color.White,
            primaryContainer = p.accentSurface,
            onPrimaryContainer = p.text,
            inversePrimary = p.accentBright,
            secondary = p.caperGreen,
            onSecondary = p.text,
            secondaryContainer = p.border,
            onSecondaryContainer = p.text,
            background = p.blackout,
            onBackground = p.text,
            surface = p.surface,
            onSurface = p.text,
            surfaceVariant = p.surfaceRaised,
            onSurfaceVariant = p.textMuted,
            inverseSurface = p.text,
            inverseOnSurface = p.blackout,
            error = p.errorText,
            onError = p.blackout,
            outline = p.border,
            outlineVariant = p.border,
            surfaceDim = p.blackout,
            surfaceBright = p.surfaceComposer,
            surfaceContainerLowest = p.blackout,
            surfaceContainerLow = p.surface,
            surfaceContainer = p.surfaceRaised,
            surfaceContainerHigh = p.surfaceRaised,
            surfaceContainerHighest = p.surfaceComposer,
        )
    }
    MaterialTheme(
        shapes = Shapes(
            extraSmall = ControlShape,
            small = ControlShape,
            medium = RoundedCornerShape(13.dp),
            large = RoundedCornerShape(15.dp),
            extraLarge = RoundedCornerShape(16.dp),
        ),
        // Every role Material components read comes from the active theme palette; unset roles fall back to
        // Material's purple baseline (slider tracks, field labels, dividers, menus, sheets).
        colorScheme = scheme,
        typography = Typography().run {
            copy(
                displayLarge = displayLarge.copy(fontFamily = Nunito, letterSpacing = (-0.25).sp), displayMedium = displayMedium.copy(fontFamily = Nunito, letterSpacing = (-0.2).sp), displaySmall = displaySmall.copy(fontFamily = Nunito, letterSpacing = (-0.15).sp),
                headlineLarge = headlineLarge.copy(fontFamily = Nunito, letterSpacing = (-0.2).sp), headlineMedium = headlineMedium.copy(fontFamily = Nunito, letterSpacing = (-0.15).sp), headlineSmall = headlineSmall.copy(fontFamily = Nunito, letterSpacing = (-0.1).sp),
                titleLarge = titleLarge.copy(fontFamily = Nunito, letterSpacing = (-0.1).sp), titleMedium = titleMedium.copy(fontFamily = Figtree, letterSpacing = 0.sp), titleSmall = titleSmall.copy(fontFamily = Figtree, letterSpacing = 0.sp),
                bodyLarge = bodyLarge.copy(fontFamily = Figtree, letterSpacing = 0.sp), bodyMedium = bodyMedium.copy(fontFamily = Figtree, letterSpacing = 0.sp), bodySmall = bodySmall.copy(fontFamily = Figtree, letterSpacing = 0.sp),
                labelLarge = labelLarge.copy(fontFamily = Figtree, letterSpacing = 0.sp), labelMedium = labelMedium.copy(fontFamily = Figtree, letterSpacing = 0.sp), labelSmall = labelSmall.copy(fontFamily = Figtree, letterSpacing = 0.sp),
            )
        },
        content = content,
    )
}

/**
 * Web's toggle: a light thumb on a border-colored track, botanical accent when on.
 */
@Composable fun caperSwitchColors(): SwitchColors {
    val p = CaperThemeStore.active
    return SwitchDefaults.colors(
        checkedThumbColor = Color.White, checkedTrackColor = p.accent, checkedBorderColor = p.accent,
        uncheckedThumbColor = p.text, uncheckedTrackColor = p.border, uncheckedBorderColor = p.border,
    )
}

/** Tabular figures (web's `tabular-nums`), so timers and counts keep their width as digits change. */
val TabularNumbers: TextStyle
    @Composable @ReadOnlyComposable get() = LocalTextStyle.current.merge(TextStyle(fontFeatureSettings = "tnum"))

/**
 * Web's primary action (shared/design.css --action): high-contrast fill with crisp inverted label.
 * Disabled, it steps back to neutral chrome rather than a faded fill.
 */
@Composable fun primaryButtonColors(): ButtonColors {
    val p = CaperThemeStore.active
    return ButtonDefaults.buttonColors(
        containerColor = p.action, contentColor = p.onAction, disabledContainerColor = p.surfaceRaised, disabledContentColor = p.textMuted,
    )
}

/**
 * Secondary and dismissive actions (Cancel, Close, Decline, Reset, Back): neutral labels, as on web.
 */
@Composable fun neutralTextButtonColors(contentColor: Color = CaperThemeStore.active.text): ButtonColors =
    ButtonDefaults.textButtonColors(contentColor = contentColor)

/** [neutralTextButtonColors] for bordered secondary actions. */
@Composable fun neutralOutlinedButtonColors(): ButtonColors =
    ButtonDefaults.outlinedButtonColors(contentColor = CaperThemeStore.active.text)
