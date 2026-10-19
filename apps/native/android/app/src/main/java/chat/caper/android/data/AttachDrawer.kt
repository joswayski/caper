package chat.caper.android.data

/** The attach drawer's capacity line, worded as on every client (web chat/attach.ts). */
object AttachCapacity {
    fun label(attached: Int, max: Int = AttachmentPolicy.MAX_ATTACHMENTS): String {
        val count = attached.coerceIn(0, max)
        return if (count == 0) "Up to $max files" else "$count of $max attached"
    }

    /** Shown once full: how to make room. */
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
