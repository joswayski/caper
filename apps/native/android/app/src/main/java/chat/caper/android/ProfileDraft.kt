package chat.caper.android

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import chat.caper.android.data.PreparedPicture
import chat.caper.android.data.ProfileAvatar
import chat.caper.android.model.Account
import chat.caper.android.model.AvatarHistoryPicture
import java.io.File
import kotlin.random.Random

/** Private to one editor visit; incoming account events never overwrite its baseline or edits. */
internal class ProfileDraft(val baseline: Account) {
    var avatarId by mutableStateOf(baseline.avatarId)
        private set
    var avatarAsset by mutableStateOf(baseline.avatarAsset)
        private set
    var avatarUrl by mutableStateOf(baseline.avatarUrl)
        private set
    var prepared by mutableStateOf<PreparedPicture?>(null)
        private set
    /** Opening, cropping or preparing a chosen picture must finish/cancel before profile Save. */
    var picturePending by mutableStateOf(false)
    private var directory: File? = null
    // Retain an already verified upload after a rejected profile save; retry does not re-upload.
    var stagedAsset: String? = null
    val pictureChanged: Boolean get() = prepared != null || avatarId != baseline.avatarId || avatarAsset != baseline.avatarAsset

    fun regenerate(random: Random = Random.Default) {
        val current = avatarId
        val next = if (current != null && current in 0..799) {
            random.nextInt(799).let { if (it >= current) it + 1 else it }
        } else random.nextInt(800)
        remove()
        avatarId = next
    }

    fun remove() {
        releasePicture()
        avatarAsset = null
        avatarUrl = null
    }

    fun select(picture: AvatarHistoryPicture) {
        releasePicture()
        avatarAsset = picture.avatarAsset
        avatarUrl = picture.avatarUrl
    }

    fun usePrepared(picture: PreparedPicture, preview: File, ownedDirectory: File) {
        releasePicture()
        prepared = picture
        directory = ownedDirectory
        avatarAsset = preview.name
        avatarUrl = preview.toURI().toString()
    }

    fun avatar(): ProfileAvatar? = if (!pictureChanged) null else ProfileAvatar(
        avatarId ?: 0, if (prepared != null) requireNotNull(stagedAsset) else avatarAsset,
        baseline.avatarId, baseline.avatarAsset,
    )

    fun releasePicture() {
        directory?.deleteRecursively()
        directory = null
        prepared = null
        stagedAsset = null
    }
}
