package chat.caper.android

import chat.caper.android.model.channelNameError
import chat.caper.android.model.hasHiddenNameCharacters
import chat.caper.android.model.normalizeChannelName
import chat.caper.android.model.spaceName
import chat.caper.android.model.spaceNameError
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.contentOrNull
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

    /** Generated from web (shared/names/channel-cases.json). */
    @Test fun `space and channel names follow the shared cases`() {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/names/channel-cases.json") }.first { it.isFile }
        val cases = Json.parseToJsonElement(file.readText()).jsonObject
        fun field(case: kotlinx.serialization.json.JsonElement, key: String) = case.jsonObject.getValue(key).jsonPrimitive.contentOrNull
        cases.getValue("normalize").jsonArray.forEach {
            val input = field(it, "input")!!
            assertEquals(input, field(it, "output"), normalizeChannelName(input))
        }
        cases.getValue("channelErrors").jsonArray.forEach {
            val name = field(it, "name")!!
            assertEquals(name, field(it, "error"), channelNameError(name))
        }
        cases.getValue("spaceNames").jsonArray.forEach {
            val name = field(it, "name")!!
            assertEquals(name, field(it, "trimmed"), spaceName(name))
            assertEquals(name, field(it, "error"), spaceNameError(name))
        }
    }
}
