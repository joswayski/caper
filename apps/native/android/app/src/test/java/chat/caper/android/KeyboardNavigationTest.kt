package chat.caper.android

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.int
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Test

/** Every client runs these same cases; see shared/shortcuts/switcher-cases.json. */
class KeyboardNavigationTest {
    private val cases: JsonObject by lazy {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/shortcuts/switcher-cases.json") }.first { it.isFile }
        Json.parseToJsonElement(file.readText()).jsonObject
    }

    @Test fun `matching follows the shared cases`() {
        cases.getValue("match").jsonArray.map { it.jsonObject }.forEach { case ->
            val query = case.getValue("query").jsonPrimitive.content
            val text = case.getValue("text").jsonPrimitive.content
            assertEquals("$query in $text", case.getValue("score").jsonPrimitive.intOrNull, KeyboardNavigation.score(query, text))
        }
    }

    @Test fun `ranking and stepping follow the shared cases`() {
        cases.getValue("rank").jsonArray.map { it.jsonObject }.forEach { case ->
            val items = case.getValue("items").jsonArray.map { it.jsonObject }.map { item ->
                (listOf(item.getValue("name").jsonPrimitive.content) + item.getValue("alternates").jsonArray.map { it.jsonPrimitive.content }) to
                    item.getValue("unread").jsonPrimitive.boolean
            }
            val expected = case.getValue("order").jsonArray.map { it.jsonPrimitive.int }
            assertEquals(case.getValue("note").jsonPrimitive.content, expected, KeyboardNavigation.rank(items, case.getValue("query").jsonPrimitive.content))
        }
        cases.getValue("adjacent").jsonArray.map { it.jsonObject }.forEach { case ->
            val unread = case.getValue("unread").jsonArray.map { it.jsonPrimitive.boolean }
            val result = KeyboardNavigation.adjacent(unread, case.getValue("current").jsonPrimitive.intOrNull,
                case.getValue("forward").jsonPrimitive.boolean, case.getValue("unreadOnly").jsonPrimitive.boolean)
            assertEquals(case.toString(), case.getValue("result").jsonPrimitive.intOrNull, result)
        }
    }
}
