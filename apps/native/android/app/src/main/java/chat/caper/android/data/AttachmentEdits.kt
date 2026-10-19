package chat.caper.android.data

import chat.caper.android.data.AttachmentPolicy.StillClass
import chat.caper.android.model.CompressionSettings
import java.util.Locale
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

// Edits to an attachment before it is sent: crop, quarter turns, drawing and text (docs/media.md,
// "Editing attachments before sending"; web `edit.ts`). The draft keeps the original file and this
// description, so reopening the editor can change any part of it; preparing the upload applies it
// to the original. Geometry is shared with every client through shared/messages/edit-cases.json.

/** Fractions of the rotated, uncropped image's width and height. */
data class EditRect(val x: Double, val y: Double, val width: Double, val height: Double) {
    companion object { val FULL = EditRect(0.0, 0.0, 1.0, 1.0) }
}

/** Painted in order over the image. Positions are fractions of the rotated, uncropped image. */
sealed interface EditMark {
    /** `#RRGGBB`. */
    val color: String

    /** A freehand line; [width] is a fraction of the rotated image's longer edge, [points] flat x, y pairs. */
    data class Stroke(override val color: String, val width: Double, val points: List<Double>) : EditMark

    /** One line of text centred on [x], [y]; [size] is its font size as a fraction of the longer edge. It stays upright. */
    data class Text(override val color: String, val size: Double, val x: Double, val y: Double, val text: String) : EditMark
}

/** [rotation]: clockwise quarter turns (0, 90, 180, 270) of the upright original (EXIF applied). */
data class AttachmentEdit(val rotation: Int = 0, val crop: EditRect = EditRect.FULL, val marks: List<EditMark> = emptyList()) {
    companion object { val NONE = AttachmentEdit() }
}

data class PixelRect(val left: Int, val top: Int, val width: Int, val height: Int)

/** Which crop frame part a drag holds: the whole frame, or the edges named. */
enum class CropHandle(val left: Boolean = false, val top: Boolean = false, val right: Boolean = false, val bottom: Boolean = false) {
    MOVE, N(top = true), NE(top = true, right = true), E(right = true), SE(right = true, bottom = true),
    S(bottom = true), SW(left = true, bottom = true), W(left = true), NW(left = true, top = true),
}

object AttachmentEdits {
    /** Square in physical image pixels, centred on the requested crop and bounded by the image. */
    fun pictureCrop(crop: EditRect, width: Int, height: Int): EditRect {
        val side = round(min(crop.width * width, crop.height * height)).coerceIn(1, min(width, height)).toDouble()
        val w = side / width
        val h = side / height
        return EditRect(round((crop.x + crop.width / 2 - w / 2) * width).toDouble().coerceIn(0.0, width - side) / width,
            round((crop.y + crop.height / 2 - h / 2) * height).toDouble().coerceIn(0.0, height - side) / height, w, h)
    }

    /** Uploads fail rather than send the unedited original: people crop to hide things. */
    const val FAILED = "Couldn’t apply your edits."
    /** Markup colours offered before the custom picker; the first is the default. */
    val COLORS = listOf("#E5484D", "#F5C542", "#4CAF50", "#3B82F6", "#FFFFFF", "#000000")
    val COLOR_NAMES = mapOf("#E5484D" to "Red", "#F5C542" to "Yellow", "#4CAF50" to "Green", "#3B82F6" to "Blue", "#FFFFFF" to "White", "#000000" to "Black")
    /** Stroke widths and text sizes in on-screen points (dp), converted to fractions as they are drawn. */
    const val STROKE_MIN = 2f
    const val STROKE_MAX = 40f
    const val STROKE_INITIAL = 6f
    const val TEXT_MIN = 14f
    const val TEXT_MAX = 96f
    const val TEXT_INITIAL = 32f
    const val MAX_TEXT = 200
    const val MIN_CROP = 0.02
    private const val EPSILON = 1e-6

    fun isEdited(edit: AttachmentEdit?): Boolean {
        if (edit == null) return false
        val crop = edit.crop
        return edit.rotation != 0 || edit.marks.isNotEmpty() || abs(crop.x) > EPSILON || abs(crop.y) > EPSILON ||
            abs(crop.width - 1) > EPSILON || abs(crop.height - 1) > EPSILON
    }

    /** The same picture as [b] would give; both absent or unedited count as the original. */
    fun same(a: AttachmentEdit?, b: AttachmentEdit?): Boolean = (a ?: AttachmentEdit.NONE) == (b ?: AttachmentEdit.NONE)

    fun rotatedSize(width: Int, height: Int, rotation: Int): Pair<Int, Int> =
        if (rotation == 90 || rotation == 270) height to width else width to height

    /** Turns the picture a quarter anticlockwise, carrying the crop and marks with it. */
    fun rotateLeft(edit: AttachmentEdit): AttachmentEdit {
        val (x, y, width, height) = edit.crop
        return AttachmentEdit(
            rotation = (edit.rotation + 270) % 360,
            crop = EditRect(y, 1 - (x + width), height, width),
            marks = edit.marks.map { mark ->
                when (mark) {
                    is EditMark.Stroke -> mark.copy(points = mark.points.mapIndexed { index, _ ->
                        if (index % 2 == 0) mark.points.getOrElse(index + 1) { 0.0 } else 1 - mark.points[index - 1]
                    })
                    is EditMark.Text -> mark.copy(x = mark.y, y = 1 - mark.x)
                }
            },
        )
    }

    /**
     * The crop in pixels of the image [width] × [height] turned by the edit. Video crops start and
     * measure on even pixels (4:2:0 chroma); every crop keeps at least one pixel (two for video).
     */
    fun cropPixels(edit: AttachmentEdit, width: Int, height: Int, even: Boolean = false): PixelRect {
        val (rotatedWidth, rotatedHeight) = rotatedSize(width, height, edit.rotation)
        val crop = edit.crop
        val left = min(max(round(crop.x * rotatedWidth), 0), rotatedWidth - 1)
        val top = min(max(round(crop.y * rotatedHeight), 0), rotatedHeight - 1)
        val right = max(left + 1, min(rotatedWidth, round((crop.x + crop.width) * rotatedWidth)))
        val bottom = max(top + 1, min(rotatedHeight, round((crop.y + crop.height) * rotatedHeight)))
        if (!even) return PixelRect(left, top, right - left, bottom - top)
        val evenLeft = left - left % 2
        val evenTop = top - top % 2
        val evenWidth = max(2, right - evenLeft - (right - evenLeft) % 2)
        val evenHeight = max(2, bottom - evenTop - (bottom - evenTop) % 2)
        return PixelRect(evenLeft, evenTop, evenWidth, evenHeight)
    }

    /** Half away from zero, like JavaScript's `Math.round` for the non-negative values used here. */
    private fun round(value: Double): Int = Math.round(value).coerceIn(Int.MIN_VALUE.toLong(), Int.MAX_VALUE.toLong()).toInt()

    /** Keeps a crop inside the image and at least [minimum] (a fraction) on each side. */
    fun clampCrop(crop: EditRect, minimum: Double = MIN_CROP): EditRect {
        val width = min(1.0, max(minimum, crop.width))
        val height = min(1.0, max(minimum, crop.height))
        return EditRect(min(1 - width, max(0.0, crop.x)), min(1 - height, max(0.0, crop.y)), width, height)
    }

    /**
     * A crop dragged by [dx], [dy] (fractions of the image) from [start] by one of its handles,
     * kept inside the image and at least [minWidth] × [minHeight] (fractions).
     */
    fun dragCrop(start: EditRect, handle: CropHandle, dx: Double, dy: Double, minWidth: Double, minHeight: Double): EditRect {
        fun clamp(value: Double, low: Double, high: Double) = min(max(value, low), max(low, high))
        if (handle == CropHandle.MOVE) return start.copy(x = clamp(start.x + dx, 0.0, 1 - start.width), y = clamp(start.y + dy, 0.0, 1 - start.height))
        var left = start.x
        var top = start.y
        var right = start.x + start.width
        var bottom = start.y + start.height
        if (handle.left) left = clamp(left + dx, 0.0, right - minWidth)
        if (handle.right) right = clamp(right + dx, left + minWidth, 1.0)
        if (handle.top) top = clamp(top + dy, 0.0, bottom - minHeight)
        if (handle.bottom) bottom = clamp(bottom + dy, top + minHeight, 1.0)
        return EditRect(left, top, right - left, bottom - top)
    }

    /** Resize an already pixel-square crop about its opposite corner or edge midpoint, without recentering it. */
    fun dragSquareCrop(start: EditRect, handle: CropHandle, dx: Double, dy: Double, minWidth: Double, minHeight: Double): EditRect {
        if (handle == CropHandle.MOVE) return dragCrop(start, handle, dx, dy, minWidth, minHeight)
        val ax = if (handle.left) 1.0 else if (handle.right) 0.0 else 0.5
        val ay = if (handle.top) 1.0 else if (handle.bottom) 0.0 else 0.5
        val x = start.x + ax * start.width
        val y = start.y + ay * start.height
        val sx = dx * (1 - 2 * ax) / start.width
        val sy = dy * (1 - 2 * ay) / start.height
        // Corners follow the larger relative motion; edges ignore motion along the edge.
        val change = if (abs(sx) > abs(sy)) sx else sy
        fun limit(anchor: Double, fraction: Double, size: Double): Double {
            val before = if (fraction > 0) anchor / fraction else Double.POSITIVE_INFINITY
            val after = if (fraction < 1) (1 - anchor) / (1 - fraction) else Double.POSITIVE_INFINITY
            return min(before, after) / size
        }
        val maximum = min(limit(x, ax, start.width), limit(y, ay, start.height))
        val minimum = max(minWidth / start.width, minHeight / start.height)
        val scale = min(max(1 + change, minimum), maximum)
        val width = start.width * scale
        val height = start.height * scale
        return EditRect(x - ax * width, y - ay * height, width, height)
    }

    /**
     * [edit] with a text field committed (web `withField`): a new mark at [x], [y], or mark [index]
     * changed. The text is trimmed and at most [MAX_TEXT] characters; an empty one removes the
     * mark (or adds nothing). [size] is the font size as a fraction of the longer edge.
     */
    fun withText(edit: AttachmentEdit, index: Int?, x: Double, y: Double, value: String, color: String, size: Double): AttachmentEdit {
        val text = value.trim().take(MAX_TEXT)
        val marks = edit.marks.toMutableList()
        if (index != null) {
            val mark = marks.getOrNull(index) as? EditMark.Text ?: return edit
            when {
                text.isEmpty() -> marks.removeAt(index)
                text != mark.text || color != mark.color || abs(size - mark.size) > 1e-9 -> marks[index] = mark.copy(text = text, color = color, size = size)
                else -> return edit
            }
        } else if (text.isNotEmpty()) marks += EditMark.Text(color, size, x, y, text)
        else return edit
        return edit.copy(marks = marks)
    }

    /**
     * Still formats the editor opens where this device decodes them. Callers also rule out
     * animation (GIFs and animated WebP/PNG would keep one frame) and undecodable files.
     */
    fun editableImageType(contentType: String): Boolean = AttachmentPolicy.normalizedType(contentType) in setOf(
        "image/jpeg", "image/png", "image/webp", "image/avif", "image/heic", "image/heif", "image/bmp", "image/x-ms-bmp", "image/tiff",
    )

    /**
     * How an edited still is encoded: by its source's class, so photos take the photo path and
     * screenshots stay lossless. Formats that upload unchanged when unedited (AVIF) are photos.
     */
    fun stillClass(contentType: String, header: ByteArray): StillClass = when (val still = AttachmentPolicy.stillClass(contentType, header)) {
        StillClass.KEEP -> if (AttachmentPolicy.normalizedType(contentType) == "image/png") StillClass.LOSSLESS else StillClass.PHOTO
        else -> still
    }

    /**
     * An edited video's output: its cropped (display) size with the short edge capped at
     * `videoMaxHeight` (0 keeps it), both edges even.
     */
    fun videoSize(cropWidth: Int, cropHeight: Int, settings: CompressionSettings): Pair<Int, Int> {
        val height = AttachmentPolicy.videoTargetHeight(cropWidth, cropHeight, settings) ?: cropHeight
        val width = if (height == cropHeight) cropWidth else (cropWidth.toLong() * height / cropHeight).toInt()
        return even(width) to even(height)
    }

    private fun even(value: Int) = max(2, value - value % 2)

    /** Media3 turns counterclockwise; the model turns clockwise. */
    fun media3Rotation(rotation: Int): Float = ((360 - rotation) % 360).toFloat()

    /** [crop] of a [width] × [height] frame as Media3 `Crop` arguments: left, right, bottom, top in NDC (-1..1, y up). */
    fun cropNdc(crop: PixelRect, width: Int, height: Int): FloatArray = floatArrayOf(
        (2.0 * crop.left / width - 1).toFloat(),
        (2.0 * (crop.left + crop.width) / width - 1).toFloat(),
        (1 - 2.0 * (crop.top + crop.height) / height).toFloat(),
        (1 - 2.0 * crop.top / height).toFloat(),
    )

    /**
     * [crop] (pixels of the picture turned [rotation] clockwise) as the matching rectangle of the
     * unturned [width] × [height] picture, for decoding only that region.
     */
    fun unrotatedRect(crop: PixelRect, width: Int, height: Int, rotation: Int): PixelRect = when (rotation) {
        90 -> PixelRect(crop.top, height - (crop.left + crop.width), crop.height, crop.width)
        180 -> PixelRect(width - (crop.left + crop.width), height - (crop.top + crop.height), crop.width, crop.height)
        270 -> PixelRect(width - (crop.top + crop.height), crop.left, crop.height, crop.width)
        else -> crop
    }

    /** `#RRGGBB` as an opaque ARGB colour; anything else is the default colour. */
    fun argb(color: String): Int {
        val digits = color.removePrefix("#")
        val value = if (digits.length == 6) digits.toIntOrNull(16) else null
        return (value ?: 0xE5484D) or (0xFF shl 24)
    }

    /** An opaque ARGB colour as `#RRGGBB`. */
    fun hex(argb: Int): String = "#" + (argb and 0xFFFFFF).toString(16).padStart(6, '0').uppercase(Locale.ROOT)
}

/**
 * The editor's undo history: every committed change (stroke, text, crop, turn, reset) is one
 * step. Undoing and then committing drops the redo steps; a commit that changes nothing is no step.
 */
data class EditHistory(val states: List<AttachmentEdit>, val index: Int = 0) {
    constructor(start: AttachmentEdit) : this(listOf(start))

    val current: AttachmentEdit get() = states[index]
    val canUndo: Boolean get() = index > 0
    val canRedo: Boolean get() = index < states.lastIndex

    fun commit(next: AttachmentEdit): EditHistory = if (next == current) this else EditHistory(states.subList(0, index + 1) + next, index + 1)
    fun undo(): EditHistory = if (canUndo) copy(index = index - 1) else this
    fun redo(): EditHistory = if (canRedo) copy(index = index + 1) else this
}
