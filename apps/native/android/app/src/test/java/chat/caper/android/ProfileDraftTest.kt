package chat.caper.android

import chat.caper.android.data.PreparedPicture
import chat.caper.android.model.Account
import chat.caper.android.model.AvatarHistoryPicture
import java.nio.file.Files
import kotlin.random.Random
import org.junit.Assert.*
import org.junit.Test

class ProfileDraftTest {
    private val account = Account("account00001", "jose", "Jose", avatarId = 17, avatarAsset = "old", avatarUrl = "https://caper.chat/old")

    @Test fun `removal and history selection are private and restoring baseline is unchanged`() {
        val draft = ProfileDraft(account)
        assertFalse(draft.pictureChanged)
        assertNull(draft.avatar())
        draft.remove()
        assertNull(draft.avatarAsset)
        assertEquals(17, draft.avatar()!!.avatarId)
        assertEquals("old", draft.avatar()!!.expectedAvatarAsset)
        assertEquals(17, draft.avatar()!!.expectedAvatarId)
        assertEquals("old", account.avatarAsset)
        draft.select(AvatarHistoryPicture("previous", "https://caper.chat/previous"))
        assertEquals("previous", draft.avatar()!!.avatarAsset)
        draft.select(AvatarHistoryPicture("old", "https://caper.chat/resigned"))
        assertFalse(draft.pictureChanged)
        assertNull(draft.avatar())
    }

    @Test fun `regeneration is local bounded and distinct from the currently previewed mascot`() {
        val draft = ProfileDraft(account.copy(avatarAsset = null, avatarUrl = null))
        val random = Random(42)
        repeat(10_000) {
            val previous = draft.avatarId
            draft.regenerate(random)
            assertTrue(draft.avatarId!! in 0..799)
            assertNotEquals(previous, draft.avatarId)
        }
        assertEquals(17, account.avatarId)
        assertEquals(17, draft.avatar()!!.expectedAvatarId)
        assertNull(draft.avatar()!!.expectedAvatarAsset)
    }

    @Test fun `prepared photo stays local and keeps staged id until replaced or discarded`() {
        val directory = Files.createTempDirectory("profile-draft-test").toFile()
        try {
            val file = directory.resolve("preview").apply { writeBytes(byteArrayOf(1, 2, 3)) }
            val picture = PreparedPicture(file.readBytes(), "image/webp", 1, 1)
            val draft = ProfileDraft(account)
            draft.usePrepared(picture, file, directory)
            assertTrue(draft.pictureChanged)
            assertSame(picture, draft.prepared)
            assertTrue(draft.avatarUrl!!.startsWith("file:"))
            assertNull(draft.stagedAsset)
            draft.stagedAsset = "verified"
            assertEquals("verified", draft.avatar()!!.avatarAsset)
            // Reading/submitting the mutation doesn't consume its local preview or verified ID.
            assertEquals(draft.avatar(), draft.avatar())
            assertTrue(file.exists())
            assertSame(picture, draft.prepared)
            assertEquals("old", account.avatarAsset)
            draft.remove()
            assertNull(draft.prepared)
            assertNull(draft.stagedAsset)
            assertFalse(directory.exists())
            assertNull(draft.avatar()!!.avatarAsset)
        } finally { directory.deleteRecursively() }
    }
}
