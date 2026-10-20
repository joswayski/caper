package chat.caper.android.data

/** The attach drawer's limit line, worded as on every client (web chat/attach.ts). Drafts below the limit show nothing. */
object AttachCapacity {
    fun label(attached: Int, max: Int = AttachmentPolicy.MAX_ATTACHMENTS): String? =
        if (attached >= max) "$max of $max attached" else null

    /** How to make room once full. */
    fun hint(attached: Int, max: Int = AttachmentPolicy.MAX_ATTACHMENTS): String? =
        if (attached >= max) "Remove a file to add another." else null
}

/** How much of the photo library the drawer can show. */
enum class LibraryAccess { FULL, PARTIAL, NONE }

/** Runtime permission names, as strings so the decisions below stay JVM-testable. */
object LibraryPermissions {
    const val READ_EXTERNAL_STORAGE = "android.permission.READ_EXTERNAL_STORAGE"
    const val READ_MEDIA_IMAGES = "android.permission.READ_MEDIA_IMAGES"
    const val READ_MEDIA_VIDEO = "android.permission.READ_MEDIA_VIDEO"
    const val READ_MEDIA_VISUAL_USER_SELECTED = "android.permission.READ_MEDIA_VISUAL_USER_SELECTED"

    /**
     * What the drawer asks for on [sdk]. Android 14+ shows "Select photos and videos" beside
     * "Allow all", and either answer lets the drawer list what was shared.
     */
    fun requested(sdk: Int): List<String> = when {
        sdk >= 34 -> listOf(READ_MEDIA_IMAGES, READ_MEDIA_VIDEO, READ_MEDIA_VISUAL_USER_SELECTED)
        sdk >= 33 -> listOf(READ_MEDIA_IMAGES, READ_MEDIA_VIDEO)
        else -> listOf(READ_EXTERNAL_STORAGE)
    }

    fun access(sdk: Int, granted: (String) -> Boolean): LibraryAccess = when {
        sdk >= 33 && granted(READ_MEDIA_IMAGES) && granted(READ_MEDIA_VIDEO) -> LibraryAccess.FULL
        // Allowing only photos or only videos still shows that part of the library.
        sdk >= 33 && (granted(READ_MEDIA_IMAGES) || granted(READ_MEDIA_VIDEO)) -> LibraryAccess.PARTIAL
        sdk >= 34 && granted(READ_MEDIA_VISUAL_USER_SELECTED) -> LibraryAccess.PARTIAL
        sdk < 33 && granted(READ_EXTERNAL_STORAGE) -> LibraryAccess.FULL
        else -> LibraryAccess.NONE
    }
}

/** Which visible gallery GIFs animate, as on iOS (`AttachGalleryAnimation`). */
object GalleryAnimation {
    /** Even a very tall tablet drawer never owns more than this many animated decoders. */
    const val MAX = 6

    /**
     * [visible] lists on-screen GIFs in grid order. While [scrolling], playing GIFs that stay on
     * screen keep playing but none start, so a fling opens no decoders. Once it settles, playing
     * ones continue and the first other visible ones fill the remaining slots.
     */
    fun <T> next(playing: Set<T>, visible: List<T>, scrolling: Boolean): Set<T> {
        val kept = visible.filter { it in playing }
        return if (scrolling) kept.toSet() else (kept + visible).distinct().take(MAX).toSet()
    }
}
