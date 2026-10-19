package chat.caper.android.data

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FileDropTest {
    /** Every client runs these same cases; see shared/messages/drop-cases.json. */
    @Test fun `shared drop overlay text matches`() {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/messages/drop-cases.json") }.first { it.isFile }
        val cases = Json.parseToJsonElement(file.readText()).jsonObject.getValue("cases").jsonArray
        assertTrue("expected the full shared case list", cases.size >= 5)
        cases.forEach { case ->
            val fields = case.jsonObject
            fun text(key: String) = fields.getValue(key).jsonPrimitive.content
            assertEquals(
                text("note"),
                text("title") to text("detail"),
                FileDrop.text(text("channel"), fields.getValue("direct").jsonPrimitive.boolean, fields.getValue("attached").jsonPrimitive.int),
            )
        }
    }

    @Test fun `only drags offering files open the overlay`() {
        assertTrue(FileDrop.carriesFiles(listOf("image/png")))
        assertTrue(FileDrop.carriesFiles(listOf("text/plain", "application/pdf")))
        assertFalse(FileDrop.carriesFiles(listOf("text/plain", "text/html")))
        assertFalse(FileDrop.carriesFiles(listOf("text/uri-list")))
        assertFalse(FileDrop.carriesFiles(emptyList()))
    }

    @Test fun `motion settles at the resting pose`() {
        assertEquals(0f, FileDrop.easeOut(0f), 1e-6f)
        assertEquals(1f, FileDrop.easeOut(1f), 1e-6f)
        for (step in 0..100) assertTrue("entrances never overshoot", FileDrop.easeOut(step / 100f) <= 1f)
        assertTrue("entrances decelerate", FileDrop.easeOut(0.25f) > 0.5f)
        assertEquals(Triple(0f, 1f, 1f), FileDrop.hop(0f))
        assertEquals(-9f, FileDrop.hop(0.5f + 1.6f * 0.25f).first, 1e-3f)
        assertEquals(0f, FileDrop.sway(0.2f, 0.48f), 0f)
        assertEquals(1f, FileDrop.sway(0.48f + 2.4f, 0.48f), 1e-5f)
    }
}
