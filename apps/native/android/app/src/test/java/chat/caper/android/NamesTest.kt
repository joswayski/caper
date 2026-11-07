package chat.caper.android

import chat.caper.android.model.hasHiddenNameCharacters
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Every client runs these same cases; see shared/names/cases.json. */
class NamesTest {
    @Test fun `names follow the shared visible and hidden cases`() {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/names/cases.json") }.first { it.isFile }
        val cases = Json.parseToJsonElement(file.readText()).jsonObject
        for ((key, hidden) in listOf("visible" to false, "hidden" to true)) {
            val names = cases.getValue(key).jsonArray.map { it.jsonPrimitive.content }
            assertTrue(names.size > 10)
            names.forEach { assertEquals(it, hidden, hasHiddenNameCharacters(it)) }
        }
    }
}
