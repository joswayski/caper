package chat.caper.android

import chat.caper.android.data.AttachCapacity
import chat.caper.android.data.GalleryAnimation
import chat.caper.android.data.LibraryAccess
import chat.caper.android.data.LibraryPermissions
import chat.caper.android.data.LibraryPermissions.READ_EXTERNAL_STORAGE
import chat.caper.android.data.LibraryPermissions.READ_MEDIA_IMAGES
import chat.caper.android.data.LibraryPermissions.READ_MEDIA_VIDEO
import chat.caper.android.data.LibraryPermissions.READ_MEDIA_VISUAL_USER_SELECTED
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AttachDrawerTest {
    @Test fun capacityShowsOnlyAtTheLimitAsOnWeb() {
        assertNull(AttachCapacity.label(0))
        assertNull(AttachCapacity.label(9))
        assertEquals("10 of 10 attached", AttachCapacity.label(10))
        assertEquals("10 of 10 attached", AttachCapacity.label(12))
        assertNull(AttachCapacity.hint(9))
        assertEquals("Remove a file to add another.", AttachCapacity.hint(10))
    }

    @Test fun requestsTheNarrowestPermissionsForEachRelease() {
        assertEquals(listOf(READ_EXTERNAL_STORAGE), LibraryPermissions.requested(26))
        assertEquals(listOf(READ_EXTERNAL_STORAGE), LibraryPermissions.requested(32))
        assertEquals(listOf(READ_MEDIA_IMAGES, READ_MEDIA_VIDEO), LibraryPermissions.requested(33))
        assertEquals(listOf(READ_MEDIA_IMAGES, READ_MEDIA_VIDEO, READ_MEDIA_VISUAL_USER_SELECTED), LibraryPermissions.requested(34))
    }

    @Test fun readsFullPartialAndNoAccess() {
        fun access(sdk: Int, vararg granted: String) = LibraryPermissions.access(sdk) { it in granted }
        assertEquals(LibraryAccess.FULL, access(34, READ_MEDIA_IMAGES, READ_MEDIA_VIDEO, READ_MEDIA_VISUAL_USER_SELECTED))
        // "Select photos and videos" on Android 14+ grants only the user-selected permission.
        assertEquals(LibraryAccess.PARTIAL, access(34, READ_MEDIA_VISUAL_USER_SELECTED))
        assertEquals(LibraryAccess.PARTIAL, access(33, READ_MEDIA_IMAGES))
        assertEquals(LibraryAccess.NONE, access(33, READ_MEDIA_VISUAL_USER_SELECTED))
        assertEquals(LibraryAccess.NONE, access(34))
        assertEquals(LibraryAccess.FULL, access(30, READ_EXTERNAL_STORAGE))
        // Storage access no longer covers media on Android 13+.
        assertEquals(LibraryAccess.NONE, access(33, READ_EXTERNAL_STORAGE))
    }

    @Test fun galleryGifsStartOnlyOnceScrollingSettles() {
        // A fling starts nothing and stops what scrolled away.
        assertEquals(emptySet<Int>(), GalleryAnimation.next(emptySet(), listOf(1, 2), scrolling = true))
        assertEquals(setOf(2), GalleryAnimation.next(setOf(1, 2), listOf(2, 3), scrolling = true))
        // Settled: playing GIFs continue, then the first visible ones fill up to six.
        assertEquals(setOf(2, 3, 4, 5, 6, 7), GalleryAnimation.next(setOf(2), listOf(3, 4, 5, 6, 7, 8, 2), scrolling = false))
        assertEquals(setOf(8, 1, 2, 3, 4, 5), GalleryAnimation.next(setOf(8), (1..8).toList(), scrolling = false))
    }
}
