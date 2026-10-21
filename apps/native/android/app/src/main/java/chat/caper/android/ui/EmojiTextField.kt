package chat.caper.android.ui

import android.icu.text.BreakIterator
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.*
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.focus.focusProperties
import chat.caper.android.data.toggleFormat
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import chat.caper.android.entrance
import chat.caper.android.KeyboardAttachmentInput
import chat.caper.android.MOTION_FAST
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.util.Locale
import kotlin.math.roundToInt

/** Material's usual composer decoration, with non-consuming emoji inspection on its actual text. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun EmojiTextField(
    value: TextFieldValue, onValueChange: (TextFieldValue) -> Unit, modifier: Modifier = Modifier,
    enabled: Boolean = true, maxLines: Int = 6, readOnly: Boolean = false,
    placeholder: (@Composable () -> Unit)? = null,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    leadingIcon: (@Composable () -> Unit)? = null, trailingIcon: (@Composable () -> Unit)? = null,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default, keyboardActions: KeyboardActions = KeyboardActions.Default,
    colors: TextFieldColors = OutlinedTextFieldDefaults.colors(),
    keyboardAttachment: ((android.net.Uri) -> Job?)? = null,
) = KeyboardAttachmentInput(keyboardAttachment.takeIf { enabled }) {
    val context = LocalContext.current
    val density = LocalDensity.current
    val interaction = remember { MutableInteractionSource() }
    val focused by interaction.collectIsFocusedAsState()
    val style = LocalTextStyle.current.merge(androidx.compose.ui.text.TextStyle(
        color = if (!enabled) colors.disabledTextColor else if (focused) colors.focusedTextColor else colors.unfocusedTextColor,
    ))
    val textHeight = with(density) { style.lineHeight.toDp() * maxLines }
    val ranges = remember(value.text) {
        val iterator = BreakIterator.getCharacterInstance(Locale.ROOT).apply { setText(value.text) }
        buildList {
            var start = iterator.first()
            var end = iterator.next()
            while (end != BreakIterator.DONE) {
                EmojiArtwork.name(context, value.text.substring(start, end))?.let { add(Triple(start, end, it)) }
                start = end; end = iterator.next()
            }
        }
    }
    var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
    var name by remember(value.text) { mutableStateOf<String?>(null) }
    var anchor by remember { mutableStateOf(Offset.Zero) }
    var pending by remember { mutableStateOf<Job?>(null) }
    val scope = rememberCoroutineScope()
    val scroll = rememberScrollState()
    fun format(marker: String) {
        if (!enabled || readOnly || value.composition != null) return
        val result = toggleFormat(value.text, value.selection.min, value.selection.max, marker)
        // Reject expansion rather than letting a caller truncate the selected content.
        if (result.text.codePointCount(0, result.text.length) > 4000) return
        onValueChange(TextFieldValue(result.text, TextRange(result.start, result.end)))
    }
    LaunchedEffect(scroll.value, enabled) { pending?.cancel(); name = null }
    LaunchedEffect(focused) { if (!focused) { pending?.cancel(); name = null } }
    DisposableEffect(value.text) { onDispose { pending?.cancel() } }
    fun at(point: Offset): Triple<Int, Int, String>? {
        val result = layout ?: return null
        return ranges.firstOrNull { range ->
            (range.first until range.second).any { result.getBoundingBox(it).contains(point) }
        }
    }
    BasicTextField(
        value, onValueChange, modifier.onPreviewKeyEvent { event ->
            if (enabled && !readOnly && event.type == KeyEventType.KeyDown && (event.isCtrlPressed || event.isMetaPressed) && !event.isAltPressed) {
                when (event.key) {
                    Key.B -> { format("**"); true }
                    Key.I -> { format("_"); true }
                    else -> false
                }
            } else false
        }.defaultMinSize(OutlinedTextFieldDefaults.MinWidth, OutlinedTextFieldDefaults.MinHeight),
        enabled = enabled, readOnly = readOnly, textStyle = style, interactionSource = interaction,
        cursorBrush = SolidColor(colors.cursorColor), visualTransformation = visualTransformation,
        keyboardOptions = keyboardOptions, keyboardActions = keyboardActions,
        // Scroll the inner text, keeping both icon sockets stationary. Pointer coordinates
        // then follow the same scrolled text that supplied the glyph bounds.
        onTextLayout = { layout = it },
        decorationBox = { inner ->
            OutlinedTextFieldDefaults.DecorationBox(
                value = value.text, innerTextField = {
                    Box(Modifier.heightIn(max = textHeight).verticalScroll(scroll)) {
                        Box(Modifier.pointerInput(ranges, enabled) {
                            var target: Triple<Int, Int, String>? = null
                            var origin: Offset? = null
                            fun hide() { pending?.cancel(); name = null; target = null }
                            fun inspect(point: Offset, wait: Long) {
                                val next = if (enabled) at(point) else null
                                if (next == target) return
                                hide(); target = next
                                if (next != null) pending = scope.launch {
                                    delay(wait); anchor = point; name = next.third
                                }
                            }
                            try {
                                awaitPointerEventScope {
                                    while (true) {
                                        val event = awaitPointerEvent(PointerEventPass.Initial)
                                        val pointer = event.changes.firstOrNull() ?: continue
                                        when {
                                            event.type == PointerEventType.Exit -> hide()
                                            pointer.type == PointerType.Mouse && !pointer.pressed -> inspect(pointer.position, 400)
                                            event.type == PointerEventType.Press -> { hide(); origin = pointer.position; inspect(pointer.position, 500) }
                                            event.changes.size > 1 || origin?.let { (pointer.position - it).getDistance() > viewConfiguration.touchSlop } == true -> hide()
                                            event.type == PointerEventType.Release -> {
                                                pending?.cancel(); origin = null
                                                if (name != null) pending = scope.launch { delay(1500); hide() } else hide()
                                            }
                                        }
                                    }
                                }
                            } finally { hide() }
                        }) {
                            Column {
                                inner()
                                if (enabled && !readOnly && focused && value.composition == null && !value.selection.collapsed) {
                                    Surface(color = Surface, shape = MaterialTheme.shapes.small) {
                                        Row {
                                            TextButton({ format("**") }, modifier = Modifier.size(48.dp).focusProperties { canFocus = false }.semantics { contentDescription = "Bold selection" },
                                                shape = RectangleShape, contentPadding = PaddingValues(0.dp), colors = ButtonDefaults.textButtonColors(contentColor = Text)) {
                                                Text("B", fontWeight = androidx.compose.ui.text.font.FontWeight.Bold)
                                            }
                                            TextButton({ format("_") }, modifier = Modifier.size(48.dp).focusProperties { canFocus = false }.semantics { contentDescription = "Italic selection" },
                                                shape = RectangleShape, contentPadding = PaddingValues(0.dp), colors = ButtonDefaults.textButtonColors(contentColor = Text)) {
                                                Text("I", fontStyle = androidx.compose.ui.text.font.FontStyle.Italic)
                                            }
                                        }
                                    }
                                }
                            }
                            name?.let { label ->
                                Popup(alignment = Alignment.TopStart, offset = IntOffset(anchor.x.roundToInt(), anchor.y.roundToInt() - with(density) { 42.dp.roundToPx() })) {
                                    Surface(Modifier.entrance(MOTION_FAST, dy = 4.dp), color = Blackout, contentColor = Text, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                                        Text(":$label:", Modifier.padding(horizontal = 9.dp, vertical = 7.dp).widthIn(max = 260.dp), style = MaterialTheme.typography.labelMedium)
                                    }
                                }
                            }
                        }
                    }
                }, enabled = enabled, singleLine = false, visualTransformation = visualTransformation,
                interactionSource = interaction, placeholder = placeholder, leadingIcon = leadingIcon, trailingIcon = trailingIcon,
                contentPadding = OutlinedTextFieldDefaults.contentPadding(start = if (leadingIcon != null) 12.dp else 16.dp),
                colors = colors, container = {
                    OutlinedTextFieldDefaults.Container(enabled, false, interaction, colors = colors, shape = MaterialTheme.shapes.small)
                },
            )
        },
    )
}
