package chat.caper.android.data

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Typeface
import android.os.Build
import chat.caper.android.R
import java.io.File
import java.io.FileOutputStream

/**
 * Paints attachment edits with android.graphics, for the editor's stage, the composer thumbnail,
 * the uploaded still and a video's overlay alike (web `paintEdit`), so what you see is what is sent.
 */
internal object EditPainter {
    @Volatile private var bold: Typeface? = null

    /** The app's bundled Satoshi Bold, which text marks use everywhere. */
    fun typeface(context: Context): Typeface = bold ?: (runCatching { context.resources.getFont(R.font.satoshi_bold) }.getOrNull()
        ?: Typeface.DEFAULT_BOLD).also { bold = it }

    /**
     * Turns the canvas so a [width] × [height] picture drawn at the origin lands turned
     * [rotation]° clockwise, at the origin of the turned picture's space.
     */
    fun turn(canvas: Canvas, width: Float, height: Float, rotation: Int) {
        when (rotation) {
            90 -> { canvas.translate(height, 0f); canvas.rotate(90f) }
            180 -> { canvas.translate(width, height); canvas.rotate(180f) }
            270 -> { canvas.translate(0f, width); canvas.rotate(270f) }
        }
    }

    /** Draws the upright [bitmap], stretched to [width] × [height], turned [rotation]° into the turned picture's space. */
    fun drawTurned(canvas: Canvas, bitmap: Bitmap, width: Float, height: Float, rotation: Int) {
        canvas.save()
        turn(canvas, width, height, rotation)
        canvas.drawBitmap(bitmap, null, RectF(0f, 0f, width, height), Paint(Paint.FILTER_BITMAP_FLAG))
        canvas.restore()
    }

    /** Text paint for a mark: Satoshi Bold at [pixels], centred horizontally on its anchor. */
    fun textPaint(typeface: Typeface, pixels: Float, color: Int): Paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        this.typeface = typeface
        textSize = pixels
        textAlign = Paint.Align.CENTER
        this.color = color
    }

    /** The baseline that puts the middle of the line (between ascent and descent) on [centerY]. */
    fun baseline(paint: Paint, centerY: Float): Float {
        val metrics = paint.fontMetrics
        return centerY - (metrics.ascent + metrics.descent) / 2f
    }

    /**
     * Paints [marks] in the turned picture's pixel space ([width] × [height]): round-capped
     * polylines (a single point is a dot), and upright text centred on its point.
     */
    fun paintMarks(canvas: Canvas, marks: List<EditMark>, width: Float, height: Float, typeface: Typeface) {
        val edge = maxOf(width, height)
        for (mark in marks) {
            val color = AttachmentEdits.argb(mark.color)
            when (mark) {
                is EditMark.Text -> {
                    if (mark.text.isEmpty()) continue
                    val paint = textPaint(typeface, (mark.size * edge).toFloat(), color)
                    canvas.drawText(mark.text, (mark.x * width).toFloat(), baseline(paint, (mark.y * height).toFloat()), paint)
                }
                is EditMark.Stroke -> {
                    if (mark.points.size < 2) continue
                    val lineWidth = (mark.width * edge).toFloat()
                    val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { this.color = color }
                    if (mark.points.size < 4) {
                        canvas.drawCircle((mark.points[0] * width).toFloat(), (mark.points[1] * height).toFloat(), lineWidth / 2f, paint)
                        continue
                    }
                    paint.style = Paint.Style.STROKE
                    paint.strokeWidth = lineWidth
                    paint.strokeCap = Paint.Cap.ROUND
                    paint.strokeJoin = Paint.Join.ROUND
                    val path = Path().apply {
                        moveTo((mark.points[0] * width).toFloat(), (mark.points[1] * height).toFloat())
                        var index = 2
                        while (index + 1 < mark.points.size) {
                            lineTo((mark.points[index] * width).toFloat(), (mark.points[index + 1] * height).toFloat())
                            index += 2
                        }
                    }
                    canvas.drawPath(path, paint)
                }
            }
        }
    }

    /**
     * Paints the edited picture's [region] (pixels of the turned picture) scaled to fill the
     * canvas's [outWidth] × [outHeight]: [source] (the upright original, or a downscaled copy of
     * it, which is stretched to [width] × [height]) turned by the edit, then its marks. Without a
     * source only the marks are painted, as a video's overlay.
     */
    fun paintEdit(
        canvas: Canvas, source: Bitmap?, width: Int, height: Int, edit: AttachmentEdit, region: PixelRect,
        outWidth: Int, outHeight: Int, typeface: Typeface,
    ) {
        val (turnedWidth, turnedHeight) = AttachmentEdits.rotatedSize(width, height, edit.rotation)
        canvas.save()
        canvas.scale(outWidth.toFloat() / region.width, outHeight.toFloat() / region.height)
        canvas.translate(-region.left.toFloat(), -region.top.toFloat())
        if (source != null) drawTurned(canvas, source, width.toFloat(), height.toFloat(), edit.rotation)
        paintMarks(canvas, edit.marks, turnedWidth.toFloat(), turnedHeight.toFloat(), typeface)
        canvas.restore()
    }

    /** Writes [bitmap] as a lossy WebP thumbnail; false (and no file) when it could not be written. */
    fun writePreview(bitmap: Bitmap, file: File): Boolean {
        @Suppress("DEPRECATION")
        val format = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) Bitmap.CompressFormat.WEBP_LOSSY else Bitmap.CompressFormat.WEBP
        val written = runCatching { FileOutputStream(file).use { bitmap.compress(format, 90, it) } }.getOrDefault(false)
        return (written && file.length() > 0).also { if (!it) file.delete() }
    }

    /**
     * A small render of [edit] over the upright [picture] for the composer's thumbnail and the
     * pending message, at most [edge] pixels on its longer side.
     */
    fun preview(picture: Bitmap, edit: AttachmentEdit, edge: Int, typeface: Typeface): Bitmap {
        val region = AttachmentEdits.cropPixels(edit, picture.width, picture.height)
        val (width, height) = AttachmentPolicy.fitWithin(region.width, region.height, edge)
        val output = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        paintEdit(Canvas(output), picture, picture.width, picture.height, edit, region, width, height, typeface)
        return output
    }
}
