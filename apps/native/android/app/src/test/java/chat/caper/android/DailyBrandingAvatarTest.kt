package chat.caper.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

class DailyBrandingAvatarTest {
    @Test fun `same UTC day keeps its assignment without drawing again`() {
        val previous = BrandingAvatarAssignment(20_000, 123)
        val result = brandingAvatarAssignment(previous, 20_000) { error("must not draw") }
        assertEquals(previous, result)
    }

    @Test fun `UTC day boundary selects a new assignment`() {
        val result = brandingAvatarAssignment(BrandingAvatarAssignment(20_000, 12), 20_001) { 4 }
        assertEquals(BrandingAvatarAssignment(20_001, 4), result)
    }

    @Test fun `first assignment can select the last avatar`() {
        assertEquals(799, brandingAvatarAssignment(null, 20_000) { bound -> bound - 1 }.avatarIndex)
    }

    @Test fun `subsequent selection covers the last index and cannot immediately repeat`() {
        val previous = BrandingAvatarAssignment(20_000, 798)
        val result = brandingAvatarAssignment(previous, 20_001) { bound -> bound - 1 }
        assertEquals(799, result.avatarIndex)
        assertNotEquals(previous.avatarIndex, result.avatarIndex)

        val fromLast = brandingAvatarAssignment(BrandingAvatarAssignment(20_001, 799), 20_002) { 798 }
        assertEquals(798, fromLast.avatarIndex)
    }
}
