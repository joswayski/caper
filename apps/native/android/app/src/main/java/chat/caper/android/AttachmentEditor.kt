package chat.caper.android

import android.graphics.Bitmap
import android.net.Uri
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.drag
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.pointerHoverIcon
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import chat.caper.android.data.AttachmentEdit
import chat.caper.android.data.AttachmentEdits
import chat.caper.android.data.AttachmentPreparer
import chat.caper.android.data.CropHandle
import chat.caper.android.data.EditHistory
import chat.caper.android.data.EditMark
import chat.caper.android.data.EditPainter
import chat.caper.android.data.EditRect
import chat.caper.android.ui.*
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal enum class EditTool(val label: String, val icon: Int) {
    CROP("Crop", R.drawable.lucide_crop), DRAW("Draw", R.drawable.lucide_brush), TEXT("Text", R.drawable.lucide_type),
}

/** The text field open over the picture: a new mark at [x], [y], or mark [index] being changed. [points] is its size on screen. */
private data class OpenText(val index: Int?, val x: Double, val y: Double, val value: TextFieldValue, val color: String, val points: Float)

/** A text mark being dragged to [x], [y]. */
private data class MovedText(val index: Int, val x: Double, val y: Double)

/**
 * Where the picture sits on the stage: [region] (fractions of the turned picture) is shown
 * [width] × [height] px at [left], [top]; [edge] is the whole turned picture's longer edge on
 * screen, which stroke widths and text sizes are fractions of (what you see is what is sent).
 */
private data class StageLayout(
    val region: EditRect, val turnedWidth: Int, val turnedHeight: Int,
    val left: Float, val top: Float, val width: Float, val height: Float, val edge: Float,
) {
    /** A stage position as fractions of the turned, uncropped picture. */
    fun point(position: Offset): Pair<Double, Double> = Pair(
        region.x + (position.x - left) / width * region.width,
        region.y + (position.y - top) / height * region.height,
    )

    /** Whether a stage position is on the picture. */
    fun contains(position: Offset): Boolean = position.x in left..(left + width) && position.y in top..(top + height)

    /** A picture position (fractions) on the stage. */
    fun offset(x: Double, y: Double): Offset = Offset(
        left + ((x - region.x) / region.width * width).toFloat(),
        top + ((y - region.y) / region.height * height).toFloat(),
    )
}

private val EditFont = FontFamily(Font(R.font.satoshi_bold, FontWeight.Bold))
/** The thumbnail and pending message's render of an edit, until the prepared file has its own. */
private const val PREVIEW_EDGE = 1280

/**
 * The Draw tool's pointer ring, in pixels: exactly as wide as a stroke drawn now looks on screen
 * ([strokePoints] dp, which strokes store as a fraction of the picture's on-screen edge), at least 6dp.
 */
internal fun brushRingDiameter(strokePoints: Float, density: Float): Float = max(6f, strokePoints) * density

/**
 * Full-screen editor for a file waiting to be sent: crop and turn it, draw on it and add text. It
 * edits a description of the changes, never the file; the composer prepares the original again
 * with them (docs/media.md "Editing attachments before sending"). [save] is called only for a
 * change, with null when it returns to the original, and the editor's render of the result. It
 * opens on [initialTool]: Draw from the pencil, or the tool chosen in the draft preview.
 */
@Composable internal fun AttachmentEditor(
    source: String, kind: String, name: String, initial: AttachmentEdit?,
    cancel: () -> Unit, save: (AttachmentEdit?, Bitmap?) -> Unit,
    initialTool: EditTool = EditTool.DRAW,
    pictureMode: Boolean = false,
) {
    val context = LocalContext.current
    val density = LocalDensity.current
    val scope = rememberCoroutineScope()
    val preparer = remember { AttachmentPreparer(context.applicationContext) }
    val typeface = remember { EditPainter.typeface(context) }
    var picture by remember { mutableStateOf<Bitmap?>(null) }
    var failed by remember { mutableStateOf(false) }
    LaunchedEffect(source, kind) {
        val loaded = if (pictureMode) chat.caper.android.data.PicturePreparer.editorPicture(java.io.File(requireNotNull(Uri.parse(source).path)))
            else preparer.editorPicture(Uri.parse(source), kind)
        if (loaded == null) failed = true else picture = loaded
    }
    if (pictureMode) DisposableEffect(picture) {
        val owned = picture
        onDispose { owned?.recycle() }
    }
    var history by remember { mutableStateOf(EditHistory(initial ?: AttachmentEdit.NONE)) }
    var tool by remember { mutableStateOf(initialTool) }
    var color by remember { mutableStateOf(AttachmentEdits.COLORS.first()) }
    var strokePoints by remember { mutableFloatStateOf(AttachmentEdits.STROKE_INITIAL) }
    var textPoints by remember { mutableFloatStateOf(AttachmentEdits.TEXT_INITIAL) }
    var liveStroke by remember { mutableStateOf<EditMark.Stroke?>(null) }
    // Draw's eraser: presses remove the lines they touch instead of drawing; [erased] are the
    // committed marks the press under way has touched.
    var erasing by remember { mutableStateOf(false) }
    var erased by remember { mutableStateOf(emptySet<Int>()) }
    var liveCrop by remember { mutableStateOf<EditRect?>(null) }
    var liveText by remember { mutableStateOf<MovedText?>(null) }
    var field by remember { mutableStateOf<OpenText?>(null) }
    var picking by remember { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    var stage by remember { mutableStateOf<StageLayout?>(null) }
    var keyboardFocus by remember { mutableStateOf(false) }
    // Where a mouse or hovering stylus is on the stage, for the Draw tool's brush ring; null for touch or once it leaves.
    var hover by remember { mutableStateOf<Offset?>(null) }
    val hiddenPointer = remember { PointerIcon(android.view.PointerIcon.getSystemIcon(context, android.view.PointerIcon.TYPE_NULL)) }
    val current = history.current
    val ready = picture != null

    fun square(edit: AttachmentEdit): AttachmentEdit {
        val bitmap = picture ?: return edit
        val (w, h) = AttachmentEdits.rotatedSize(bitmap.width, bitmap.height, edit.rotation)
        return edit.copy(crop = AttachmentEdits.pictureCrop(edit.crop, w, h))
    }
    LaunchedEffect(picture) {
        if (pictureMode && picture != null) history = EditHistory(square(initial ?: AttachmentEdit.NONE))
    }
    fun commit(next: AttachmentEdit) { history = history.commit(if (pictureMode) square(next) else next) }
    fun withField(edit: AttachmentEdit, open: OpenText?): AttachmentEdit {
        val layout = stage ?: return edit
        open ?: return edit
        return AttachmentEdits.withText(edit, open.index, open.x, open.y, open.value.text, open.color, open.points * density.density / layout.edge.toDouble())
    }
    // A press on the picture and the field losing focus can both commit it; the first one wins.
    fun commitField() {
        val open = field ?: return
        field = null
        commit(withField(history.current, open))
    }
    fun undo() { field = null; history = history.undo() }
    fun redo() { field = null; history = history.redo() }
    fun chooseTool(next: EditTool) { commitField(); tool = next; erasing = false }
    fun chooseColor(next: String) {
        color = next
        erasing = false
        field = field?.copy(color = next)
    }
    fun done() {
        if (saving) return
        val final = withField(history.current, field)
        if (!pictureMode && AttachmentEdits.same(final, initial)) return cancel()
        val result = final.takeIf { AttachmentEdits.isEdited(it) }
        saving = true
        val shown = picture
        scope.launch {
            val preview = if (result == null || shown == null) null else withContext(Dispatchers.Default) {
                runCatching { EditPainter.preview(shown, result, PREVIEW_EDGE, typeface) }.getOrNull()
            }
            save(result, preview)
        }
    }

    Dialog(
        onDismissRequest = { if (field != null) field = null else cancel() },
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
    ) {
        val focus = remember { FocusRequester() }
        LaunchedEffect(field == null) { if (field == null) runCatching { focus.requestFocus() } }
        Column(
            // Filled before the insets, so the bar and keyboard areas aren't left showing the dimmed scrim.
            Modifier.fillMaxSize().swipeCancelsPresses().entrance(MOTION_BASE, dy = 12.dp).background(Blackout).systemBarsPadding().imePadding()
                .semantics { paneTitle = if (kind == "video") "Edit video" else "Edit image" }
                // Hardware keyboards: Ctrl+Z undoes, Ctrl+Shift+Z or Ctrl+Y redoes, Escape closes the field or the
                // editor, and while cropping (with nothing else focused) arrows move the frame, Shift+arrows resize it.
                .onPreviewKeyEvent { event ->
                    if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                    val command = event.isCtrlPressed || event.isMetaPressed
                    val step = if (event.isAltPressed) 0.002 else 0.01
                    val nudge = when (event.key) {
                        Key.DirectionLeft -> -step to 0.0
                        Key.DirectionRight -> step to 0.0
                        Key.DirectionUp -> 0.0 to -step
                        Key.DirectionDown -> 0.0 to step
                        else -> null
                    }
                    when {
                        event.key == Key.Escape -> { if (field != null) field = null else cancel(); true }
                        command && field == null && event.key == Key.Z -> { if (event.isShiftPressed) redo() else undo(); true }
                        command && field == null && event.key == Key.Y -> { redo(); true }
                        nudge != null && keyboardFocus && ready && tool == EditTool.CROP -> {
                            val edit = history.current
                            val handle = if (event.isShiftPressed) CropHandle.SE else CropHandle.MOVE
                            commit(edit.copy(crop = AttachmentEdits.dragCrop(edit.crop, handle, nudge.first, nudge.second, AttachmentEdits.MIN_CROP, AttachmentEdits.MIN_CROP)))
                            true
                        }
                        else -> false
                    }
                }
                .focusRequester(focus).onFocusChanged { keyboardFocus = it.isFocused }.focusable(),
        ) {
            Row(Modifier.fillMaxWidth().padding(start = 4.dp, end = 12.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(cancel, Modifier.semantics { contentDescription = "Cancel editing" }) {
                    Icon(painterResource(R.drawable.lucide_x), null, tint = Text)
                }
                Column(Modifier.weight(1f).padding(horizontal = 4.dp)) {
                    Text(if (kind == "video") "Edit video" else "Edit image", Modifier.semantics { heading() }, color = Text, fontSize = 14.sp,
                        fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(name, color = TextMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                BarIcon(R.drawable.lucide_undo_2, "Undo", ready && history.canUndo, ::undo)
                BarIcon(R.drawable.lucide_redo_2, "Redo", ready && history.canRedo, ::redo)
                TextButton({ field = null; commit(AttachmentEdit.NONE) }, enabled = ready && AttachmentEdits.isEdited(current),
                    shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors(), contentPadding = PaddingValues(horizontal = 8.dp)) { Text("Reset") }
                Button(::done, enabled = ready && !saving, shape = MaterialTheme.shapes.small, colors = primaryButtonColors(),
                    contentPadding = PaddingValues(horizontal = 14.dp)) { Text("Done", fontWeight = FontWeight.Bold) }
            }
            if (pictureMode) Text("The square is what will be saved. Drag inside to reposition; drag its corners to resize.",
                Modifier.padding(horizontal = 16.dp, vertical = 4.dp), color = TextMuted, fontSize = 12.sp)
            BoxWithConstraints(Modifier.weight(1f).fillMaxWidth().clipToBounds(), contentAlignment = Alignment.Center) {
                val bitmap = picture
                when {
                    failed -> Text("This file can’t be edited here.", Modifier.padding(24.dp).semantics { liveRegion = LiveRegionMode.Polite },
                        color = TextMuted, fontSize = 14.sp, textAlign = TextAlign.Center)
                    bitmap == null -> Spinner(null, Modifier.size(28.dp))
                    else -> {
                        val (turnedWidth, turnedHeight) = AttachmentEdits.rotatedSize(bitmap.width, bitmap.height, current.rotation)
                        val region = if (tool == EditTool.CROP) EditRect.FULL else current.crop
                        val padding = with(density) { (if (tool == EditTool.CROP) 28.dp else 16.dp).toPx() }
                        val boxWidth = constraints.maxWidth.toFloat()
                        val boxHeight = constraints.maxHeight.toFloat()
                        val regionWidth = (region.width * turnedWidth).toFloat()
                        val regionHeight = (region.height * turnedHeight).toFloat()
                        val scale = max(0.0001f, min((boxWidth - padding * 2) / regionWidth, (boxHeight - padding * 2) / regionHeight))
                        val layout = StageLayout(
                            region, turnedWidth, turnedHeight, (boxWidth - regionWidth * scale) / 2f, (boxHeight - regionHeight * scale) / 2f,
                            regionWidth * scale, regionHeight * scale, scale * max(turnedWidth, turnedHeight),
                        )
                        SideEffect { stage = layout }
                        val latest by rememberUpdatedState(layout)
                        // Read while drawing, so the ring follows the pointer without recomposing the editor.
                        val ringShown by remember(layout) { derivedStateOf { tool == EditTool.DRAW && hover?.let(layout::contains) == true } }
                        // What is on screen: the committed edit plus any drag in progress.
                        val moved = liveText
                        val open = field
                        val shown = current.copy(
                            crop = liveCrop ?: current.crop,
                            marks = current.marks.mapIndexedNotNull { index, mark ->
                                when {
                                    index in erased -> null
                                    open?.index == index -> null
                                    moved?.index == index && mark is EditMark.Text -> mark.copy(x = moved.x, y = moved.y)
                                    else -> mark
                                }
                            } + listOfNotNull(liveStroke),
                        )
                        Canvas(
                            Modifier.fillMaxSize()
                                .semantics { contentDescription = if (kind == "video") "Video frame being edited" else "Image being edited" }
                                // The ring replaces the system pointer over the picture.
                                .pointerHoverIcon(if (ringShown) hiddenPointer else PointerIcon.Default)
                                .pointerInput(Unit) {
                                    // Watches without consuming, so it sees hovering and drawing alike.
                                    awaitPointerEventScope {
                                        while (true) {
                                            val event = awaitPointerEvent(PointerEventPass.Initial)
                                            val pointer = event.changes.firstOrNull() ?: continue
                                            hover = pointer.position.takeIf {
                                                event.type != PointerEventType.Exit && pointer.type != PointerType.Touch && pointer.type != PointerType.Unknown
                                            }
                                        }
                                    }
                                }
                                .pointerInput(tool) {
                                    val sample = 1.5.dp.toPx()
                                    val slop = 4.dp.toPx()
                                    val margin = 8.dp.toPx()
                                    awaitEachGesture {
                                        val down = awaitFirstDown()
                                        val start = latest
                                        val inside = start.contains(down.position)
                                        when (tool) {
                                            EditTool.CROP -> {
                                                val crop = history.current.crop
                                                val handle = cropHandle(down.position, crop, start, 24.dp.toPx()) ?: return@awaitEachGesture
                                                down.consume()
                                                val minWidth = min(1.0, max(AttachmentEdits.MIN_CROP, 32.dp.toPx() / start.width.toDouble()))
                                                val minHeight = min(1.0, max(AttachmentEdits.MIN_CROP, 32.dp.toPx() / start.height.toDouble()))
                                                val resize = if (pictureMode) AttachmentEdits::dragSquareCrop else AttachmentEdits::dragCrop
                                                drag(down.id) { change ->
                                                    liveCrop = resize(crop, handle, ((change.position.x - down.position.x) / start.width).toDouble(),
                                                        ((change.position.y - down.position.y) / start.height).toDouble(), minWidth, minHeight)
                                                    change.consume()
                                                }
                                                liveCrop?.let { commit(history.current.copy(crop = AttachmentEdits.clampCrop(it))) }
                                                liveCrop = null
                                            }
                                            EditTool.DRAW -> {
                                                if (!inside) return@awaitEachGesture
                                                down.consume()
                                                // A press outside the open field commits it, then draws.
                                                commitField()
                                                if (erasing) {
                                                    val edit = history.current
                                                    val radius = brushRingDiameter(strokePoints, density.density) / 2.0
                                                    val fullWidth = start.width / start.region.width
                                                    val fullHeight = start.height / start.region.height
                                                    // Sampled closely enough along the way that a fast swipe skips no line.
                                                    fun eraseAlong(from: Offset, to: Offset) {
                                                        val steps = max(1, ceil((to - from).getDistance() / max(1.0, radius / 2)).toInt())
                                                        var touched = erased
                                                        for (step in 1..steps) {
                                                            val (px, py) = start.point(from + (to - from) * (step.toFloat() / steps))
                                                            touched = touched + AttachmentEdits.strokesTouched(edit.marks, px, py, radius, fullWidth, fullHeight)
                                                        }
                                                        if (touched != erased) erased = touched
                                                    }
                                                    var last = down.position
                                                    eraseAlong(last, last)
                                                    drag(down.id) { change ->
                                                        eraseAlong(last, change.position)
                                                        last = change.position
                                                        change.consume()
                                                    }
                                                    val gone = erased
                                                    if (gone.isNotEmpty()) commit(edit.copy(marks = edit.marks.filterIndexed { index, _ -> index !in gone }))
                                                    erased = emptySet()
                                                    return@awaitEachGesture
                                                }
                                                val (x, y) = start.point(down.position)
                                                var last = down.position
                                                liveStroke = EditMark.Stroke(color, strokePoints * density.density / start.edge.toDouble(), listOf(x, y))
                                                drag(down.id) { change ->
                                                    if ((change.position - last).getDistance() >= sample) {
                                                        val (px, py) = start.point(change.position)
                                                        liveStroke = liveStroke?.let { it.copy(points = it.points + px + py) }
                                                        last = change.position
                                                    }
                                                    change.consume()
                                                }
                                                liveStroke?.let { stroke -> commit(history.current.let { it.copy(marks = it.marks + stroke) }) }
                                                liveStroke = null
                                            }
                                            EditTool.TEXT -> {
                                                if (!inside && field == null) return@awaitEachGesture
                                                down.consume()
                                                if (field != null) {
                                                    // A press outside the open field commits it, like losing focus.
                                                    commitField()
                                                    waitForUpOrCancellation()
                                                    return@awaitEachGesture
                                                }
                                                val edit = history.current
                                                val (x, y) = start.point(down.position)
                                                val index = textAt(edit, start, down.position, typeface, margin)
                                                val mark = index?.let { edit.marks[it] as? EditMark.Text }
                                                var dragged = false
                                                drag(down.id) { change ->
                                                    if (mark == null) return@drag
                                                    if (!dragged && (change.position - down.position).getDistance() > slop) dragged = true
                                                    if (dragged) {
                                                        val (px, py) = start.point(change.position)
                                                        liveText = MovedText(index, (mark.x + px - x).coerceIn(0.0, 1.0), (mark.y + py - y).coerceIn(0.0, 1.0))
                                                        change.consume()
                                                    }
                                                }
                                                val text = liveText
                                                when {
                                                    mark == null -> field = OpenText(null, x, y, TextFieldValue(""), color, textPoints)
                                                    dragged && text != null -> commit(edit.copy(marks = edit.marks.mapIndexed { i, it ->
                                                        if (i == text.index && it is EditMark.Text) it.copy(x = text.x, y = text.y) else it
                                                    }))
                                                    !dragged -> {
                                                        val points = (mark.size * start.edge / density.density).toFloat()
                                                        color = mark.color
                                                        textPoints = points.coerceIn(AttachmentEdits.TEXT_MIN, AttachmentEdits.TEXT_MAX)
                                                        field = OpenText(index, mark.x, mark.y, TextFieldValue(mark.text, TextRange(mark.text.length)), mark.color, points)
                                                    }
                                                }
                                                liveText = null
                                            }
                                        }
                                    }
                                },
                        ) {
                            drawIntoCanvas { canvas ->
                                val native = canvas.nativeCanvas
                                native.save()
                                native.clipRect(layout.left, layout.top, layout.left + layout.width, layout.top + layout.height)
                                native.translate(layout.left, layout.top)
                                native.scale(layout.width / regionWidth, layout.height / regionHeight)
                                native.translate((-region.x * turnedWidth).toFloat(), (-region.y * turnedHeight).toFloat())
                                EditPainter.drawTurned(native, bitmap, bitmap.width.toFloat(), bitmap.height.toFloat(), shown.rotation)
                                EditPainter.paintMarks(native, shown.marks, turnedWidth.toFloat(), turnedHeight.toFloat(), typeface)
                                native.restore()
                            }
                            if (tool == EditTool.CROP) drawCropFrame(shown.crop, layout)
                            if (ringShown) hover?.let {
                                drawBrushRing(it, brushRingDiameter(strokePoints, density.density) / 2f, if (erasing) Color.White.copy(alpha = 0.4f) else Color(AttachmentEdits.argb(color)))
                            }
                        }
                        if (open != null) {
                            val anchor = layout.offset(open.x, open.y)
                            val textFocus = remember { FocusRequester() }
                            var focused by remember { mutableStateOf(false) }
                            LaunchedEffect(open.index, open.x, open.y) { runCatching { textFocus.requestFocus() } }
                            BasicTextField(
                                open.value,
                                { value -> field = field?.copy(value = if (value.text.length <= AttachmentEdits.MAX_TEXT) value else value.copy(text = value.text.take(AttachmentEdits.MAX_TEXT))) },
                                Modifier.align(Alignment.TopStart)
                                    .layout { measurable, constraints ->
                                        val placeable = measurable.measure(constraints.copy(minWidth = 0, minHeight = 0))
                                        layout(placeable.width, placeable.height) {
                                            placeable.place((anchor.x - placeable.width / 2f).roundToInt(), (anchor.y - placeable.height / 2f).roundToInt())
                                        }
                                    }
                                    .widthIn(min = 48.dp)
                                    .border(1.dp, Text.copy(alpha = 0.7f), MaterialTheme.shapes.small)
                                    .padding(horizontal = 6.dp)
                                    .focusRequester(textFocus)
                                    .onFocusChanged { state ->
                                        // Losing focus commits, as Return does.
                                        if (focused && !state.isFocused) commitField()
                                        focused = state.isFocused
                                    }
                                    .semantics { contentDescription = if (open.index == null) "New text" else "Edit text" },
                                textStyle = TextStyle(
                                    color = Color(AttachmentEdits.argb(open.color)), fontFamily = EditFont, fontWeight = FontWeight.Bold,
                                    fontSize = with(density) { open.points.dp.toSp() }, textAlign = TextAlign.Center,
                                ),
                                singleLine = true,
                                cursorBrush = SolidColor(Text),
                                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                                keyboardActions = KeyboardActions(onDone = { commitField() }),
                            )
                        }
                    }
                }
            }
            if (kind == "video" && ready) Text("Drawings and text stay on screen for the whole video.", Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                color = TextMuted, fontSize = 12.sp, textAlign = TextAlign.Center)
            Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (tool == EditTool.CROP) {
                    OutlinedButton({ commit(AttachmentEdits.rotateLeft(history.current)) }, Modifier.align(Alignment.CenterHorizontally), enabled = ready,
                        shape = MaterialTheme.shapes.small, colors = neutralOutlinedButtonColors(), border = BorderStroke(1.dp, Border)) {
                        Icon(painterResource(R.drawable.lucide_rotate_ccw), null, Modifier.size(18.dp), tint = Text)
                        Spacer(Modifier.width(8.dp))
                        Text("Rotate left")
                    }
                } else {
                    val custom = color !in AttachmentEdits.COLORS
                    Row(Modifier.fillMaxWidth().selectableGroup().semantics { contentDescription = "Colour" }, horizontalArrangement = Arrangement.SpaceBetween) {
                        AttachmentEdits.COLORS.forEach { preset ->
                            Swatch(!erasing && color == preset, AttachmentEdits.COLOR_NAMES[preset] ?: preset, SolidColor(Color(AttachmentEdits.argb(preset)))) {
                                chooseColor(preset)
                            }
                        }
                        val spectrum = Brush.sweepGradient(listOf(0f, 60f, 120f, 180f, 240f, 300f, 360f).map { Color.hsv(it, 0.85f, 0.95f) })
                        Swatch(custom && !erasing, "Custom colour", if (custom) SolidColor(Color(AttachmentEdits.argb(color))) else spectrum) {
                            picking = true
                        }
                    }
                    val draw = tool == EditTool.DRAW
                    // The size is read inside the row, so dragging its slider recomposes the row, not the picture.
                    SizeRow(
                        draw, erasing, color, ready,
                        points = { if (draw) strokePoints else textPoints },
                        change = { value -> if (draw) strokePoints = value else { textPoints = value; field = field?.copy(points = value) } },
                    ) { if (draw) EraserButton(erasing, ready) { erasing = !erasing } }
                }
                Row(Modifier.fillMaxWidth().selectableGroup().semantics { contentDescription = "Editing tools" }, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    EditTool.entries.forEach { option ->
                        val selected = tool == option
                        Column(
                            Modifier.weight(1f).heightIn(min = 48.dp).clip(MaterialTheme.shapes.small)
                                .background(if (selected) TerracottaWash else Color.Transparent)
                                .border(1.dp, if (selected) Terracotta else Border, MaterialTheme.shapes.small)
                                .selectable(selected, enabled = ready, role = Role.Tab) { chooseTool(option) }
                                .padding(vertical = 6.dp),
                            horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center,
                        ) {
                            val tint = if (selected) Text else TextMuted
                            Icon(painterResource(option.icon), null, Modifier.size(18.dp), tint = tint)
                            Text(option.label, color = tint, fontSize = 12.sp, fontWeight = if (selected) FontWeight.Bold else FontWeight.Normal)
                        }
                    }
                }
            }
        }
        if (picking) ColorPickerDialog(color, dismiss = { picking = false }) { chosen -> picking = false; chooseColor(chosen) }
    }
}

/** The size slider between a live preview of the dot or text and its value in points. */
@Composable private fun SizeRow(
    draw: Boolean, erasing: Boolean, color: String, enabled: Boolean,
    points: () -> Float, change: (Float) -> Unit, leading: @Composable () -> Unit,
) {
    val density = LocalDensity.current
    val value = points()
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        leading()
        Box(Modifier.size(44.dp), contentAlignment = Alignment.Center) {
            when {
                draw && erasing -> Box(Modifier.size(value.dp).border(1.5.dp, TextMuted, CircleShape))
                draw -> Box(Modifier.size(value.dp).background(Color(AttachmentEdits.argb(color)), CircleShape).border(1.dp, Border, CircleShape))
                else -> Text("Aa", color = Color(AttachmentEdits.argb(color)), fontFamily = EditFont, fontWeight = FontWeight.Bold,
                    fontSize = with(density) { min(value, 36f).dp.toSp() }, maxLines = 1)
            }
        }
        DefaultSlider(
            value,
            change,
            Modifier.weight(1f).semantics { contentDescription = if (!draw) "Text size" else if (erasing) "Eraser size" else "Line size" },
            enabled = enabled,
            valueRange = if (draw) AttachmentEdits.STROKE_MIN..AttachmentEdits.STROKE_MAX else AttachmentEdits.TEXT_MIN..AttachmentEdits.TEXT_MAX,
            defaultValue = if (draw) 6f else 32f,
        )
        Text(value.roundToInt().toString(), Modifier.widthIn(min = 20.dp).clearAndSetSemantics {}, color = TextMuted, fontSize = 12.sp,
            style = TabularNumbers, textAlign = TextAlign.End)
    }
}

/** Draw's eraser: on, presses erase lines; a colour or another tool turns it off. */
@Composable private fun EraserButton(selected: Boolean, enabled: Boolean, toggle: () -> Unit) {
    Box(
        Modifier.size(44.dp).clip(MaterialTheme.shapes.small)
            .toggleable(selected, enabled = enabled, role = Role.Switch) { toggle() }
            .semantics { contentDescription = "Eraser" },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(36.dp).background(if (selected) TerracottaWash else Color.Transparent, MaterialTheme.shapes.small)
                .border(1.dp, if (selected) Terracotta else Border, MaterialTheme.shapes.small),
            contentAlignment = Alignment.Center,
        ) {
            Icon(painterResource(R.drawable.lucide_eraser), null, Modifier.size(18.dp), tint = if (selected) Text else TextMuted)
        }
    }
}

@Composable private fun BarIcon(icon: Int, label: String, enabled: Boolean, onClick: () -> Unit) {
    IconButton(onClick, Modifier.semantics { contentDescription = label }, enabled = enabled) {
        Icon(painterResource(icon), null, Modifier.size(20.dp), tint = if (enabled) Text else TextMuted.copy(alpha = 0.4f))
    }
}

/** A colour choice: a 28dp dot in a 44dp target, ringed in terracotta when selected. */
@Composable private fun Swatch(selected: Boolean, label: String, fill: Brush, onClick: () -> Unit) {
    Box(
        Modifier.size(44.dp).clip(CircleShape).selectable(selected, role = Role.RadioButton, onClick = onClick).semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(32.dp).border(if (selected) 2.dp else 1.dp, if (selected) Terracotta else Border, CircleShape).padding(3.dp).background(fill, CircleShape))
    }
}

/** The crop frame over the whole turned picture: the outside dimmed, a light outline, corner and edge handles. */
private fun DrawScope.drawCropFrame(crop: EditRect, layout: StageLayout) {
    val left = layout.left + (crop.x * layout.width).toFloat()
    val top = layout.top + (crop.y * layout.height).toFloat()
    val right = left + (crop.width * layout.width).toFloat()
    val bottom = top + (crop.height * layout.height).toFloat()
    val shade = Color.Black.copy(alpha = 0.55f)
    val pictureRight = layout.left + layout.width
    val pictureBottom = layout.top + layout.height
    drawRect(shade, Offset(layout.left, layout.top), Size(layout.width, top - layout.top))
    drawRect(shade, Offset(layout.left, bottom), Size(layout.width, pictureBottom - bottom))
    drawRect(shade, Offset(layout.left, top), Size(left - layout.left, bottom - top))
    drawRect(shade, Offset(right, top), Size(pictureRight - right, bottom - top))
    val line = 1.dp.toPx()
    drawRect(Text.copy(alpha = 0.9f), Offset(left, top), Size(right - left, bottom - top), style = Stroke(line))
    val handle = 3.dp.toPx()
    val reach = min(18.dp.toPx(), min(right - left, bottom - top) / 3f)
    val corners = listOf(Offset(left, top) to Offset(1f, 1f), Offset(right, top) to Offset(-1f, 1f), Offset(right, bottom) to Offset(-1f, -1f), Offset(left, bottom) to Offset(1f, -1f))
    corners.forEach { (corner, inward) ->
        drawLine(Text, corner, corner + Offset(reach * inward.x, 0f), handle, StrokeCap.Round)
        drawLine(Text, corner, corner + Offset(0f, reach * inward.y), handle, StrokeCap.Round)
    }
    val middleX = (left + right) / 2f
    val middleY = (top + bottom) / 2f
    drawLine(Text, Offset(middleX - reach / 2f, top), Offset(middleX + reach / 2f, top), handle, StrokeCap.Round)
    drawLine(Text, Offset(middleX - reach / 2f, bottom), Offset(middleX + reach / 2f, bottom), handle, StrokeCap.Round)
    drawLine(Text, Offset(left, middleY - reach / 2f), Offset(left, middleY + reach / 2f), handle, StrokeCap.Round)
    drawLine(Text, Offset(right, middleY - reach / 2f), Offset(right, middleY + reach / 2f), handle, StrokeCap.Round)
}

/** The Draw tool's pointer: the brush's size in its colour, ringed white between dark outlines so it reads on any picture. */
private fun DrawScope.drawBrushRing(center: Offset, radius: Float, color: Color) {
    drawCircle(color.copy(alpha = 0.3f), radius, center)
    drawCircle(Blackout.copy(alpha = 0.7f), radius, center, style = Stroke(3.5.dp.toPx()))
    drawCircle(Color.White, radius, center, style = Stroke(1.5.dp.toPx()))
}

/** Which part of the crop frame a press at [position] holds: a corner, an edge, inside to move it, or nothing. */
private fun cropHandle(position: Offset, crop: EditRect, layout: StageLayout, reach: Float): CropHandle? {
    val left = layout.left + (crop.x * layout.width).toFloat()
    val top = layout.top + (crop.y * layout.height).toFloat()
    val right = left + (crop.width * layout.width).toFloat()
    val bottom = top + (crop.height * layout.height).toFloat()
    // Small frames keep a middle to move by.
    val grip = min(reach, min(right - left, bottom - top) / 3f)
    val nearLeft = abs(position.x - left) <= grip
    val nearRight = abs(position.x - right) <= grip
    val nearTop = abs(position.y - top) <= grip
    val nearBottom = abs(position.y - bottom) <= grip
    val withinX = position.x in (left - grip)..(right + grip)
    val withinY = position.y in (top - grip)..(bottom + grip)
    return when {
        nearTop && nearLeft -> CropHandle.NW
        nearTop && nearRight -> CropHandle.NE
        nearBottom && nearRight -> CropHandle.SE
        nearBottom && nearLeft -> CropHandle.SW
        nearTop && withinX -> CropHandle.N
        nearBottom && withinX -> CropHandle.S
        nearLeft && withinY -> CropHandle.W
        nearRight && withinY -> CropHandle.E
        position.x in left..right && position.y in top..bottom -> CropHandle.MOVE
        else -> null
    }
}

/** The topmost text mark under [position], by its box on screen (with [margin] around it). */
private fun textAt(edit: AttachmentEdit, layout: StageLayout, position: Offset, typeface: android.graphics.Typeface, margin: Float): Int? {
    for (index in edit.marks.indices.reversed()) {
        val mark = edit.marks[index] as? EditMark.Text ?: continue
        val pixels = (mark.size * layout.edge).toFloat()
        val paint = EditPainter.textPaint(typeface, pixels, 0)
        val halfWidth = paint.measureText(mark.text) / 2f + margin
        val halfHeight = pixels * 0.6f + margin
        val anchor = layout.offset(mark.x, mark.y)
        if (abs(position.x - anchor.x) <= halfWidth && abs(position.y - anchor.y) <= halfHeight) return index
    }
    return null
}

/**
 * A small HSV colour picker (Compose has none): saturation across and brightness down a square
 * of the hue, and a hue bar below it.
 */
@Composable private fun ColorPickerDialog(initial: String, dismiss: () -> Unit, choose: (String) -> Unit) {
    val start = remember { FloatArray(3).also { android.graphics.Color.colorToHSV(AttachmentEdits.argb(initial), it) } }
    var hue by remember { mutableFloatStateOf(start[0]) }
    var saturation by remember { mutableFloatStateOf(start[1]) }
    var brightness by remember { mutableFloatStateOf(start[2]) }
    val chosen = android.graphics.Color.HSVToColor(floatArrayOf(hue, saturation, brightness))
    Dialog(onDismissRequest = dismiss) {
        Surface(Modifier.widthIn(max = 340.dp).swipeCancelsPresses(), shape = MaterialTheme.shapes.small, color = SurfaceRaised, border = BorderStroke(1.dp, Border)) {
            Column(Modifier.padding(16.dp).semantics { paneTitle = "Custom colour" }, verticalArrangement = Arrangement.spacedBy(14.dp)) {
                Text("Custom colour", Modifier.semantics { heading() }, color = Text, fontSize = 16.sp, fontWeight = FontWeight.Bold)
                Box(
                    Modifier.fillMaxWidth().aspectRatio(1.5f).clip(MaterialTheme.shapes.small)
                        .semantics {
                            contentDescription = "Saturation and brightness"
                            progressBarRangeInfo = ProgressBarRangeInfo(brightness, 0f..1f)
                            setProgress { value -> brightness = value.coerceIn(0f, 1f); true }
                        }
                        .pointerInput(Unit) {
                            fun pick(position: Offset) {
                                saturation = (position.x / size.width).coerceIn(0f, 1f)
                                brightness = 1f - (position.y / size.height).coerceIn(0f, 1f)
                            }
                            awaitEachGesture {
                                val down = awaitFirstDown()
                                pick(down.position)
                                down.consume()
                                drag(down.id) { change -> pick(change.position); change.consume() }
                            }
                        }
                        .drawBehind {
                            drawRect(Brush.horizontalGradient(listOf(Color.White, Color.hsv(hue, 1f, 1f))))
                            drawRect(Brush.verticalGradient(listOf(Color.Transparent, Color.Black)))
                            val thumb = Offset(saturation * size.width, (1f - brightness) * size.height)
                            drawCircle(Color.Black.copy(alpha = 0.6f), 9.dp.toPx(), thumb, style = Stroke(3.dp.toPx()))
                            drawCircle(Color.White, 9.dp.toPx(), thumb, style = Stroke(1.5.dp.toPx()))
                        },
                )
                Box(
                    Modifier.fillMaxWidth().height(28.dp).clip(MaterialTheme.shapes.small)
                        .semantics {
                            contentDescription = "Hue"
                            progressBarRangeInfo = ProgressBarRangeInfo(hue, 0f..360f)
                            setProgress { value -> hue = value.coerceIn(0f, 360f); true }
                        }
                        .pointerInput(Unit) {
                            fun pick(position: Offset) { hue = (position.x / size.width * 360f).coerceIn(0f, 360f) }
                            awaitEachGesture {
                                val down = awaitFirstDown()
                                pick(down.position)
                                down.consume()
                                drag(down.id) { change -> pick(change.position); change.consume() }
                            }
                        }
                        .drawBehind {
                            drawRect(Brush.horizontalGradient(listOf(0f, 60f, 120f, 180f, 240f, 300f, 360f).map { Color.hsv(it, 1f, 1f) }))
                            val x = hue / 360f * size.width
                            drawLine(Color.Black.copy(alpha = 0.6f), Offset(x, 0f), Offset(x, size.height), 4.dp.toPx())
                            drawLine(Color.White, Offset(x, 0f), Offset(x, size.height), 2.dp.toPx())
                        },
                )
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Box(Modifier.size(28.dp).background(Color(chosen), CircleShape).border(1.dp, Border, CircleShape))
                    Text(AttachmentEdits.hex(chosen), Modifier.weight(1f), color = TextMuted, fontSize = 13.sp, style = TabularNumbers)
                    TextButton(dismiss, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Cancel") }
                    Button({ choose(AttachmentEdits.hex(chosen)) }, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { Text("Use colour") }
                }
            }
        }
    }
}
