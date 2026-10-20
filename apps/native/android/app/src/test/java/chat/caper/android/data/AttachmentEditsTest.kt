package chat.caper.android.data

import chat.caper.android.data.AttachmentPolicy.StillClass
import chat.caper.android.model.CompressionSettings
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.double
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AttachmentEditsTest {
    @Test fun pictureCropIsSquareAndCanBeRepositionedWithoutRecentering() {
        val center = AttachmentEdits.pictureCrop(EditRect.FULL, 800, 400)
        assertEquals(EditRect(0.25, 0.0, 0.5, 1.0), center)
        val moved = AttachmentEdits.dragCrop(center, CropHandle.MOVE, 0.25, 0.0, 0.02, 0.02)
        val saved = AttachmentEdits.pictureCrop(moved, 800, 400)
        assertEquals(PixelRect(400, 0, 400, 400), AttachmentEdits.cropPixels(AttachmentEdit(crop = saved), 800, 400))
    }

    @Test fun pictureCropRemainsSquareAfterResizeRotationAndReset() {
        val resized = AttachmentEdits.pictureCrop(EditRect(0.1, 0.2, 0.3, 0.5), 800, 400)
        assertEquals(resized.width * 800, resized.height * 400, 0.000001)
        val turned = AttachmentEdits.rotateLeft(AttachmentEdit(crop = resized))
        val square = AttachmentEdits.pictureCrop(turned.crop, 400, 800)
        assertEquals(square.width * 400, square.height * 800, 0.000001)
        assertEquals(EditRect(0.0, 0.25, 1.0, 0.5), AttachmentEdits.pictureCrop(EditRect.FULL, 400, 800))
    }

    @Test fun pictureCropQuantizesOddSizesToAnExactlySquareSavedRegion() {
        for ((width, height) in listOf(803 to 401, 401 to 803, 513 to 512)) {
            val crop = AttachmentEdits.pictureCrop(EditRect(0.12, 0.07, 0.61, 0.79), width, height)
            val pixels = AttachmentEdits.cropPixels(AttachmentEdit(crop = crop), width, height)
            assertEquals(pixels.width, pixels.height)
            assertEquals(crop, AttachmentEdits.pictureCrop(crop, width, height))
            assertTrue(pixels.left >= 0 && pixels.top >= 0 && pixels.left + pixels.width <= width && pixels.top + pixels.height <= height)
        }
    }

    private val cases: JsonObject by lazy {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/messages/edit-cases.json") }.first { it.isFile }
        Json.parseToJsonElement(file.readText()).jsonObject
    }

    private fun rect(element: JsonElement) = element.jsonObject.let {
        EditRect(it.getValue("x").jsonPrimitive.double, it.getValue("y").jsonPrimitive.double,
            it.getValue("width").jsonPrimitive.double, it.getValue("height").jsonPrimitive.double)
    }

    private fun edit(element: JsonElement): AttachmentEdit {
        val fields = element.jsonObject
        val marks = fields["marks"]?.jsonArray.orEmpty().map { item ->
            val mark = item.jsonObject
            fun text(key: String) = mark.getValue(key).jsonPrimitive.content
            fun number(key: String) = mark.getValue(key).jsonPrimitive.double
            when (text("kind")) {
                "stroke" -> EditMark.Stroke(text("color"), number("width"), mark.getValue("points").jsonArray.map { it.jsonPrimitive.double })
                "text" -> EditMark.Text(text("color"), number("size"), number("x"), number("y"), text("text"))
                else -> error("Unknown mark ${text("kind")}")
            }
        }
        return AttachmentEdit(fields.getValue("rotation").jsonPrimitive.int, rect(fields.getValue("crop")), marks)
    }

    private fun assertSame(note: String, expected: AttachmentEdit, actual: AttachmentEdit) {
        assertEquals(note, expected.rotation, actual.rotation)
        assertRect(note, expected.crop, actual.crop)
        assertEquals(note, expected.marks.size, actual.marks.size)
        expected.marks.zip(actual.marks).forEach { (want, got) ->
            when (want) {
                is EditMark.Stroke -> {
                    got as EditMark.Stroke
                    assertEquals(note, want.color, got.color)
                    assertEquals(note, want.width, got.width, 1e-9)
                    assertArrayEquals(note, want.points.toDoubleArray(), got.points.toDoubleArray(), 1e-9)
                }
                is EditMark.Text -> {
                    got as EditMark.Text
                    assertEquals(note, want.color, got.color)
                    assertEquals(note, want.text, got.text)
                    assertEquals(note, want.size, got.size, 1e-9)
                    assertEquals(note, want.x, got.x, 1e-9)
                    assertEquals(note, want.y, got.y, 1e-9)
                }
            }
        }
    }

    private fun assertRect(note: String, expected: EditRect, actual: EditRect) {
        assertEquals(note, expected.x, actual.x, 1e-9)
        assertEquals(note, expected.y, actual.y, 1e-9)
        assertEquals(note, expected.width, actual.width, 1e-9)
        assertEquals(note, expected.height, actual.height, 1e-9)
    }

    /** Every client runs these same cases; see shared/messages/edit-cases.json. */
    @Test fun `shared turns match`() {
        val list = cases.getValue("rotateLeft").jsonArray
        assertTrue("expected the full shared case list", list.size >= 4)
        list.forEach { case ->
            val fields = case.jsonObject
            assertSame(fields.getValue("note").jsonPrimitive.content, edit(fields.getValue("after")), AttachmentEdits.rotateLeft(edit(fields.getValue("before"))))
        }
    }

    @Test fun `shared crops in pixels match`() {
        val list = cases.getValue("cropPixels").jsonArray
        assertTrue("expected the full shared case list", list.size >= 7)
        list.forEach { case ->
            val fields = case.jsonObject
            fun int(key: String) = fields.getValue(key).jsonPrimitive.int
            val expected = fields.getValue("expected").jsonObject.let {
                PixelRect(it.getValue("left").jsonPrimitive.int, it.getValue("top").jsonPrimitive.int, it.getValue("width").jsonPrimitive.int, it.getValue("height").jsonPrimitive.int)
            }
            val edit = AttachmentEdit(int("rotation"), rect(fields.getValue("crop")))
            assertEquals(fields.getValue("note").jsonPrimitive.content, expected,
                AttachmentEdits.cropPixels(edit, int("width"), int("height"), fields.getValue("even").jsonPrimitive.boolean))
        }
    }

    @Test fun `shared erase cases match`() {
        val list = cases.getValue("erase").jsonObject.getValue("cases").jsonArray
        assertTrue("expected the full shared case list", list.size >= 6)
        list.forEach { case ->
            val fields = case.jsonObject
            fun number(key: String) = fields.getValue(key).jsonPrimitive.double
            val marks = edit(buildJsonObject {
                put("rotation", 0)
                put("crop", buildJsonObject { put("x", 0); put("y", 0); put("width", 1); put("height", 1) })
                put("marks", fields.getValue("marks"))
            }).marks
            assertEquals(fields.getValue("note").jsonPrimitive.content, fields.getValue("touched").jsonArray.map { it.jsonPrimitive.int },
                AttachmentEdits.strokesTouched(marks, number("x"), number("y"), number("radius"), number("width"), number("height")))
        }
    }

    @Test fun `shared edited states match`() {
        val list = cases.getValue("edited").jsonArray
        assertTrue("expected the full shared case list", list.size >= 4)
        list.forEach { case ->
            val fields = case.jsonObject
            assertEquals(fields.getValue("note").jsonPrimitive.content, fields.getValue("edited").jsonPrimitive.boolean, AttachmentEdits.isEdited(edit(fields.getValue("edit"))))
        }
        assertFalse(AttachmentEdits.isEdited(null))
        assertFalse(AttachmentEdits.isEdited(AttachmentEdit(crop = EditRect(1e-7, 0.0, 1 - 1e-7, 1.0))))
    }

    @Test fun `four turns left return to the start`() {
        val start = AttachmentEdit(90, EditRect(0.1, 0.2, 0.5, 0.3), listOf(
            EditMark.Stroke("#E5484D", 0.01, listOf(0.1, 0.2, 0.3, 0.4)),
            EditMark.Text("#FFFFFF", 0.05, 0.25, 0.75, "Look"),
        ))
        var turned = start
        repeat(4) { turned = AttachmentEdits.rotateLeft(turned) }
        assertSame("four quarter turns", start, turned)
        assertEquals(0, AttachmentEdits.rotateLeft(start).rotation)
    }

    @Test fun `crops clamp inside the picture`() {
        assertRect("too small", EditRect(0.0, 0.98, 0.02, 0.02), AttachmentEdits.clampCrop(EditRect(-0.5, 1.2, 0.0, 0.001)))
        assertRect("too large", EditRect.FULL, AttachmentEdits.clampCrop(EditRect(0.3, 0.3, 2.0, 1.5)))
        assertRect("moved off the edge", EditRect(0.5, 0.0, 0.5, 0.5), AttachmentEdits.clampCrop(EditRect(0.9, -0.2, 0.5, 0.5)))
    }

    @Test fun `dragging the crop frame keeps it inside and above the minimum`() {
        val start = EditRect(0.2, 0.2, 0.5, 0.5)
        assertRect("move", EditRect(0.5, 0.0, 0.5, 0.5), AttachmentEdits.dragCrop(start, CropHandle.MOVE, 0.6, -0.4, 0.05, 0.05))
        assertRect("north-west", EditRect(0.1, 0.0, 0.6, 0.7), AttachmentEdits.dragCrop(start, CropHandle.NW, -0.1, -0.3, 0.05, 0.05))
        assertRect("east past the left edge", EditRect(0.2, 0.2, 0.05, 0.5), AttachmentEdits.dragCrop(start, CropHandle.E, -0.9, 0.0, 0.05, 0.05))
        assertRect("south only moves the bottom", EditRect(0.2, 0.2, 0.5, 0.8), AttachmentEdits.dragCrop(start, CropHandle.S, 0.3, 0.5, 0.05, 0.05))
    }

    @Test fun `square crop handles resize without drifting`() {
        // A 300px square on a 1000 × 500 image, deliberately off center.
        val start = EditRect(0.2, 0.15, 0.3, 0.6)
        val cases = listOf(
            CropHandle.NW to listOf(-0.06, -0.09, 0.14, 0.03),
            CropHandle.N to listOf(0.1, -0.12, 0.17, 0.03),
            CropHandle.NE to listOf(0.06, -0.09, 0.2, 0.03),
            CropHandle.E to listOf(0.06, 0.3, 0.2, 0.09),
            CropHandle.SE to listOf(0.06, 0.09, 0.2, 0.15),
            CropHandle.S to listOf(0.1, 0.12, 0.17, 0.15),
            CropHandle.SW to listOf(-0.06, 0.09, 0.14, 0.15),
            CropHandle.W to listOf(-0.06, 0.3, 0.14, 0.09),
        )
        for ((handle, values) in cases) {
            val (dx, dy, x, y) = values
            assertRect("$handle grows", EditRect(x, y, 0.36, 0.72), AttachmentEdits.dragSquareCrop(start, handle, dx, dy, 0.04, 0.1))
            assertRect("$handle shrinks", EditRect(0.4 - x, 0.3 - y, 0.24, 0.48), AttachmentEdits.dragSquareCrop(start, handle, -dx, -dy, 0.04, 0.1))
        }
        assertRect("bounded", EditRect(0.2, 0.15, 0.425, 0.85), AttachmentEdits.dragSquareCrop(start, CropHandle.SE, 2.0, 3.0, 0.04, 0.1))
        assertRect("minimum", EditRect(0.2, 0.15, 0.05, 0.1), AttachmentEdits.dragSquareCrop(start, CropHandle.SE, -2.0, -3.0, 0.04, 0.1))
        assertRect("along edge", start, AttachmentEdits.dragSquareCrop(start, CropHandle.E, 0.0, 0.2, 0.04, 0.1))
        assertRect("move", EditRect(0.7, 0.0, 0.3, 0.6), AttachmentEdits.dragSquareCrop(start, CropHandle.MOVE, 2.0, -3.0, 0.04, 0.1))
    }

    @Test fun `regions map back to the unturned picture`() {
        // 40 × 30 picture; a crop of its rotated pixel space maps to the same pixels unturned.
        val crop = PixelRect(5, 7, 10, 12)
        assertEquals(crop, AttachmentEdits.unrotatedRect(crop, 40, 30, 0))
        // Turned 90° clockwise the picture is 30 × 40: rotated (X, Y) is unturned (Y, 30 - X).
        assertEquals(PixelRect(7, 15, 12, 10), AttachmentEdits.unrotatedRect(crop, 40, 30, 90))
        assertEquals(PixelRect(25, 11, 10, 12), AttachmentEdits.unrotatedRect(crop, 40, 30, 180))
        // Turned 270°: rotated (X, Y) is unturned (40 - Y, X).
        assertEquals(PixelRect(21, 5, 12, 10), AttachmentEdits.unrotatedRect(crop, 40, 30, 270))
    }

    @Test fun `video crops become Media3 arguments`() {
        assertArrayEquals(floatArrayOf(-1f, 1f, -1f, 1f), AttachmentEdits.cropNdc(PixelRect(0, 0, 1920, 1080), 1920, 1080), 1e-6f)
        // The left half, below the top quarter: y is up in NDC.
        assertArrayEquals(floatArrayOf(-1f, 0f, -1f, 0.5f), AttachmentEdits.cropNdc(PixelRect(0, 270, 960, 810), 1920, 1080), 1e-6f)
        assertEquals(270f, AttachmentEdits.media3Rotation(90), 0f)
        assertEquals(0f, AttachmentEdits.media3Rotation(0), 0f)
        assertEquals(90f, AttachmentEdits.media3Rotation(270), 0f)
    }

    @Test fun `edited videos keep their crop within the short-edge limit`() {
        val settings = CompressionSettings(videoMaxHeight = 1080)
        assertEquals(540 to 1920, AttachmentEdits.videoSize(540, 1920, settings))
        assertEquals(1920 to 1080, AttachmentEdits.videoSize(3840, 2160, settings))
        assertEquals(1080 to 1350, AttachmentEdits.videoSize(2160, 2700, settings))
        assertEquals(1000 to 2, AttachmentEdits.videoSize(1001, 3, settings))
        assertEquals(3840 to 2160, AttachmentEdits.videoSize(3840, 2160, CompressionSettings(videoMaxHeight = 0)))
    }

    @Test fun `editable stills and their encoding class`() {
        listOf("image/jpeg", "image/png", "image/webp", "image/avif", "image/heic", "image/heif", "image/bmp", "image/tiff", "IMAGE/JPEG; q=1")
            .forEach { assertTrue(it, AttachmentEdits.editableImageType(it)) }
        listOf("image/gif", "image/svg+xml", "application/pdf", "video/mp4").forEach { assertFalse(it, AttachmentEdits.editableImageType(it)) }
        val none = ByteArray(0)
        assertEquals(StillClass.PHOTO, AttachmentEdits.stillClass("image/avif", none))
        assertEquals(StillClass.PHOTO, AttachmentEdits.stillClass("image/heic", none))
        assertEquals(StillClass.PHOTO, AttachmentEdits.stillClass("image/jpeg", none))
        assertEquals(StillClass.LOSSLESS, AttachmentEdits.stillClass("image/png", none))
        assertEquals(StillClass.LOSSLESS, AttachmentEdits.stillClass("image/tiff", none))
        assertEquals(StillClass.LOSSLESS, AttachmentEdits.stillClass("image/webp", none))
    }

    @Test fun `colours read and write as hex`() {
        assertEquals(0xFFE5484D.toInt(), AttachmentEdits.argb("#E5484D"))
        assertEquals(0xFF000000.toInt(), AttachmentEdits.argb("#000000"))
        assertEquals(0xFFE5484D.toInt(), AttachmentEdits.argb("red"))
        assertEquals("#0A0B0C", AttachmentEdits.hex(0xFF0A0B0C.toInt()))
        assertEquals(AttachmentEdits.COLORS.toSet(), AttachmentEdits.COLOR_NAMES.keys)
    }

    @Test fun `history steps through commits`() {
        var history = EditHistory(AttachmentEdit.NONE)
        assertFalse(history.canUndo)
        val turned = AttachmentEdit(rotation = 270)
        history = history.commit(turned)
        assertEquals("a commit that changes nothing is no step", history, history.commit(turned))
        assertTrue(history.canUndo)
        history = history.undo()
        assertEquals(AttachmentEdit.NONE, history.current)
        assertTrue(history.canRedo)
        val cropped = AttachmentEdit(crop = EditRect(0.0, 0.0, 0.5, 1.0))
        history = history.commit(cropped)
        assertFalse("a new step drops the redo steps", history.canRedo)
        assertEquals(2, history.states.size)
        history = history.undo().redo()
        assertEquals(cropped, history.current)
        assertEquals(history, history.redo())
        assertTrue(AttachmentEdits.same(null, AttachmentEdit.NONE))
        assertFalse(AttachmentEdits.same(null, cropped))
    }

    @Test fun `text fields commit as marks`() {
        val start = AttachmentEdit()
        val added = AttachmentEdits.withText(start, null, 0.5, 0.25, "  Hi  ", "#FFFFFF", 0.04)
        assertEquals(listOf<EditMark>(EditMark.Text("#FFFFFF", 0.04, 0.5, 0.25, "Hi")), added.marks)
        assertEquals("empty new text adds nothing", start, AttachmentEdits.withText(start, null, 0.5, 0.5, "   ", "#FFFFFF", 0.04))
        assertEquals("unchanged text is no change", added, AttachmentEdits.withText(added, 0, 0.0, 0.0, "Hi", "#FFFFFF", 0.04))
        val recoloured = AttachmentEdits.withText(added, 0, 0.0, 0.0, "Hi", "#000000", 0.04)
        assertEquals(EditMark.Text("#000000", 0.04, 0.5, 0.25, "Hi"), recoloured.marks.single())
        assertTrue("emptied text removes the mark", AttachmentEdits.withText(added, 0, 0.0, 0.0, "", "#FFFFFF", 0.04).marks.isEmpty())
        assertEquals(200, (AttachmentEdits.withText(start, null, 0.5, 0.5, "x".repeat(300), "#FFFFFF", 0.04).marks.single() as EditMark.Text).text.length)
        val stroke = AttachmentEdit(marks = listOf(EditMark.Stroke("#000000", 0.01, listOf(0.5, 0.5))))
        assertEquals("a stroke is not text", stroke, AttachmentEdits.withText(stroke, 0, 0.0, 0.0, "Hi", "#FFFFFF", 0.04))
    }
}
