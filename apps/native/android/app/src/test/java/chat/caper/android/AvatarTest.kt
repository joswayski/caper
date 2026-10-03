package chat.caper.android

import chat.caper.android.model.caperAvatarIndex
import chat.caper.android.model.Account
import chat.caper.android.model.SpectatorParticipant
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AvatarTest {
    @Test fun persistedIndicesAndFallback() {
        listOf(0, 31, 32, 255, 256, 799).forEach { assertEquals(it, caperAvatarIndex(it)) }
        listOf(null, -1, 800).forEach { assertNull(caperAvatarIndex(it)) }
    }

    @Test fun avatarIdIsOptionalJson() {
        assertNull(Json.decodeFromString<Account>("""{"id":"old"}""").avatarId)
        assertEquals(16, Json.decodeFromString<Account>("""{"id":"saved","avatarId":16}""").avatarId)
        assertEquals(799, SpectatorParticipant("voice", "Saved", muted = false, deafened = false, avatarId = 799).asParticipant().avatarId)
    }
}
