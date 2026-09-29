package chat.caper.android.data

import chat.caper.android.model.SpaceDetail
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class FeedbackModelTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test fun feedbackPagingAndDefaultsDecode() {
        val detail = json.decodeFromString<SpaceDetail>("""{"space":{"id":"space0000001","name":"Feedback","ownerId":"owner","feedback":true},"channels":[{"id":"channel00001","spaceId":"space0000001","name":"Jose","private":true,"feedbackUserId":"member","unread":true}],"members":[],"nextFeedbackBefore":"channel00001"}""")
        assertTrue(detail.space.feedback)
        assertEquals("channel00001", detail.nextFeedbackBefore)
        assertTrue(detail.channels.single().unread)
        val old = json.decodeFromString<SpaceDetail>("""{"space":{"id":"space0000001","name":"Old","ownerId":"owner"},"channels":[],"members":[]}""")
        assertFalse(old.space.feedback)
        assertNull(old.nextFeedbackBefore)
    }
}
