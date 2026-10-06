package chat.caper.android.data

import chat.caper.android.model.Reactor
import chat.caper.android.model.ReactorGroup
import chat.caper.android.model.ReactorList
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class ReactorsTest {
    private val json = Json { ignoreUnknownKeys = true }
    private fun person(id: String, name: String? = null, username: String? = id) = Reactor(id, username, name, 100)
    private val alice = person("alice", "Alice A")
    private val bob = person("bob", "Bob B")
    private val carol = person("carol", "Carol C")
    private val dave = person("dave", "Dave D")
    private val erin = person("erin", "Erin E")
    private val me = person("me", "Jose")

    @Test fun `summary names up to three people and moves you to the front`() {
        assertEquals("You reacted with :thumbs-up:", reactionSummary(listOf(me), "me", "thumbs-up", "👍"))
        assertEquals("You and Bob B reacted with :thumbs-up:", reactionSummary(listOf(bob, me), "me", "thumbs-up", "👍"))
        assertEquals("Alice A and Bob B reacted with :thumbs-up:", reactionSummary(listOf(alice, bob), "me", "thumbs-up", "👍"))
        assertEquals(
            "Alice A, Bob B and Carol C reacted with :party-popper:",
            reactionSummary(listOf(alice, bob, carol), "me", "party-popper", "🎉"),
        )
        assertEquals(
            "You, Alice A, Bob B and 2 others reacted with :thumbs-up:",
            reactionSummary(listOf(alice, bob, carol, me, dave), "me", "thumbs-up", "👍"),
        )
        assertEquals(
            "Alice A, Bob B, Carol C and 1 other reacted with :thumbs-up:",
            reactionSummary(listOf(alice, bob, carol, dave), "me", "thumbs-up", "👍"),
        )
        assertEquals(
            "Alice A, Bob B, Carol C and 2 others reacted with :thumbs-up:",
            reactionSummary(listOf(alice, bob, carol, dave, erin), null, "thumbs-up", "👍"),
        )
    }

    @Test fun `summary names fall back to username then someone and the emoji to its glyph`() {
        val usernameOnly = person("u1", name = null, username = "mira")
        val blank = person("u2", name = " ", username = null)
        assertEquals("mira and Someone reacted with 🫠", reactionSummary(listOf(usernameOnly, blank), "me", null, "🫠"))
        assertEquals("Someone", reactorName(Reactor("u3")))
        assertEquals(":red-heart:", reactionEmojiLabel("red-heart", "❤️"))
        assertEquals("❤️", reactionEmojiLabel(null, "❤️"))
    }

    @Test fun `fallback summary uses snapshot ids before names load`() {
        assertEquals("You reacted with :thumbs-up:", fallbackReactionSummary(listOf("me"), "me", "thumbs-up", "👍"))
        assertEquals("1 person reacted with :thumbs-up:", fallbackReactionSummary(listOf("bob"), "me", "thumbs-up", "👍"))
        assertEquals("1 person reacted with :thumbs-up:", fallbackReactionSummary(listOf("me"), null, "thumbs-up", "👍"))
        assertEquals("3 people reacted with 🫠", fallbackReactionSummary(listOf("me", "bob", "alice"), "me", null, "🫠"))
    }

    @Test fun `server response decodes with nullable names in reaction order`() {
        val list = json.decodeFromString<ReactorList>(
            """{"messageId":"message00000001","reactionSeq":"12","reactions":[{"emoji":"👍","authors":[""" +
                """{"id":"bob","username":"bob","displayName":"Bob B","avatarId":101},""" +
                """{"id":"alice","username":null,"displayName":null,"avatarId":100}]}]}""",
        ).validated("message00000001")
        assertEquals("12", list.reactionSeq)
        val group = list.reactions.single()
        assertEquals("👍", group.emoji)
        assertEquals(listOf("bob", "alice"), group.authors.map { it.id })
        assertEquals(101, group.authors[0].avatarId)
        assertNull(group.authors[1].displayName)
        assertEquals("Bob B and Someone reacted with :thumbs-up:", reactionSummary(group.authors, "me", "thumbs-up", "👍"))
    }

    @Test fun `validator rejects mismatched messages, bad revisions and repeated people`() {
        val valid = ReactorList("message00000001", "3", listOf(ReactorGroup("👍", listOf(alice, bob))))
        assertSame(valid, valid.validated("message00000001"))
        assertThrows(IllegalArgumentException::class.java) { valid.validated("message00000002") }
        assertThrows(IllegalArgumentException::class.java) { valid.copy(reactionSeq = "03").validated("message00000001") }
        assertThrows(IllegalArgumentException::class.java) { valid.copy(reactionSeq = "-1").validated("message00000001") }
        assertThrows(IllegalArgumentException::class.java) {
            valid.copy(reactions = listOf(ReactorGroup("👍", listOf(alice, alice)))).validated("message00000001")
        }
        assertThrows(IllegalArgumentException::class.java) {
            valid.copy(reactions = listOf(ReactorGroup("👍", emptyList()))).validated("message00000001")
        }
    }

    @Test fun `cache is reused only while the snapshot revision matches`() {
        val cache = ReactorCache(limit = 2)
        val first = ReactorList("message00000001", "7", listOf(ReactorGroup("👍", listOf(alice))))
        cache.put(first)
        assertSame(first, cache.get("message00000001", "7"))
        assertNull(cache.get("message00000001", "8"))
        assertNull(cache.get("message00000001", null))

        val untouched = ReactorList("message00000002", "0", emptyList())
        cache.put(untouched)
        assertSame(untouched, cache.get("message00000002", null))

        cache.put(ReactorList("message00000003", "1", emptyList()))
        assertNull("least recently used entry is evicted", cache.get("message00000001", "7"))
        cache.clear()
        assertNull(cache.get("message00000002", null))
    }
}
