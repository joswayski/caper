package chat.caper.android.data

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.double
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
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

    @Test fun `entrances settle without overshooting`() {
        assertEquals(0f, FileDrop.easeOut(0f), 1e-6f)
        assertEquals(1f, FileDrop.easeOut(1f), 1e-6f)
        for (step in 0..100) assertTrue("entrances never overshoot", FileDrop.easeOut(step / 100f) <= 1f)
        assertTrue("entrances decelerate", FileDrop.easeOut(0.25f) > 0.5f)
    }

    /** Every client derives the same motion from a seed; see shared/messages/drop-motion-cases.json. */
    @Test fun `shared drop motion matches`() {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/messages/drop-motion-cases.json") }.first { it.isFile }
        val motion = Json.parseToJsonElement(file.readText()).jsonObject
        fun JsonElement.number(key: String) = jsonObject.getValue(key).jsonPrimitive.double
        val sample = motion.getValue("random").jsonObject
        val random = FileDrop.Random(sample.getValue("seed").jsonPrimitive.long.toInt())
        sample.getValue("values").jsonArray.forEach { assertEquals(it.jsonPrimitive.double, random.next(), 1e-6) }
        val plan = FileDrop.plan(motion.getValue("seed").jsonPrimitive.long.toInt())
        val cases = motion.getValue("cases").jsonArray
        assertTrue("expected the full shared case list", cases.size >= 5)
        cases.forEach { case ->
            val elapsed = case.number("elapsed")
            val pose = FileDrop.pose(plan, elapsed, case.jsonObject.getValue("still").jsonPrimitive.boolean)
            case.jsonObject.getValue("cards").jsonArray.forEachIndexed { index, card ->
                val actual = pose.cards[index]
                listOf("x" to actual.x, "y" to actual.y, "turn" to actual.turn, "opacity" to actual.opacity).forEach { (key, value) ->
                    assertEquals("$elapsed card $index $key", card.number(key), value, 1e-4)
                }
            }
            val character = case.jsonObject.getValue("character")
            val actual = pose.character
            listOf("y" to actual.y, "scaleX" to actual.scaleX, "scaleY" to actual.scaleY, "turn" to actual.turn, "opacity" to actual.opacity)
                .forEach { (key, value) -> assertEquals("$elapsed character $key", character.number(key), value, 1e-4) }
        }
    }
}
