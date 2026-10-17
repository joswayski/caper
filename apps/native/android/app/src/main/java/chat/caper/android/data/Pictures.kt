package chat.caper.android.data

import chat.caper.android.model.PictureCompletion
import java.io.ByteArrayInputStream
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive

/**
 * Custom profile pictures and space icons: GIFs keep animating (unchanged when small enough,
 * else shrunk by [GifShrink]); every other picture becomes a centered square of at most 512 px,
 * encoded as WebP. The server
 * re-checks size, type and real pixel dimensions before applying one.
 */
object PicturePolicy {
    /** A GIF up to this size (and [MAX_SIDE]) uploads unchanged; a bigger one is shrunk to fit. */
    const val GIF_MAX_BYTES = 5L * 1024 * 1024
    /** Any other picture must end up within 1 MiB. */
    const val STILL_MAX_BYTES = 1024L * 1024
    /** Neither side of a stored picture may exceed this. */
    const val MAX_SIDE = 1024
    /** Re-encoded pictures are squares of at most this many pixels. */
    const val TARGET_EDGE = 512
    /** WebP quality, then fallbacks for the (unlikely) picture still over [STILL_MAX_BYTES]. */
    val QUALITIES = listOf(90, 75, 60)

    const val GIF_UNREADABLE = "That GIF couldn’t be opened. Try another one."
    const val UNUSABLE = "That picture couldn’t be used. Try another one."
}

/** A picture problem found on this device, before anything uploads. */
class PictureException(override val message: String) : Exception(message)

/** Bytes ready to upload as a picture, with their pixel size. */
class PreparedPicture(val bytes: ByteArray, val contentType: String, val width: Int, val height: Int)

/** What a picture is for: your profile picture, or the icon of a space you own. */
sealed interface PictureTarget {
    data object Avatar : PictureTarget
    data class SpaceIcon(val spaceId: String) : PictureTarget
}

private val GIF87A = "GIF87a".toByteArray(Charsets.US_ASCII)
private val GIF89A = "GIF89a".toByteArray(Charsets.US_ASCII)

/**
 * A GIF's logical screen width and height (bytes 6–9, little-endian), or null when [header]
 * does not start with the `GIF87a`/`GIF89a` magic. Detection never trusts the file name or type.
 */
fun gifSize(header: ByteArray): Pair<Int, Int>? {
    if (header.size < 10) return null
    val magic = header.copyOfRange(0, 6)
    if (!magic.contentEquals(GIF87A) && !magic.contentEquals(GIF89A)) return null
    val width = (header[6].toInt() and 0xFF) or ((header[7].toInt() and 0xFF) shl 8)
    val height = (header[8].toInt() and 0xFF) or ((header[9].toInt() and 0xFF) shl 8)
    return width to height
}

/** Whether a GIF of [byteSize] bytes and header size [width] × [height] uploads byte for byte; any other GIF is shrunk. */
fun gifUploadsUnchanged(byteSize: Long, width: Int, height: Int): Boolean =
    byteSize in 1..PicturePolicy.GIF_MAX_BYTES && width in 1..PicturePolicy.MAX_SIDE && height in 1..PicturePolicy.MAX_SIDE

/** The centered square of a picture: its [left] and [top] offsets and its [side]. */
data class SquareCrop(val left: Int, val top: Int, val side: Int)

fun squareCrop(width: Int, height: Int): SquareCrop {
    require(width > 0 && height > 0) { "Empty picture." }
    val side = minOf(width, height)
    return SquareCrop((width - side) / 2, (height - side) / 2, side)
}

/** The uploaded square's side: the picture's short edge, at most [PicturePolicy.TARGET_EDGE]. */
fun pictureEdge(width: Int, height: Int): Int = minOf(PicturePolicy.TARGET_EDGE, width, height).coerceAtLeast(1)

/** The largest power-of-two subsampling that still decodes at least [pictureEdge] pixels on the short edge. */
fun pictureSampleSize(width: Int, height: Int): Int {
    if (width < 1 || height < 1) return 1
    val short = minOf(width, height)
    val edge = pictureEdge(width, height)
    var sample = 1
    while (short / (sample * 2) >= edge) sample *= 2
    return sample
}

/** `avatar.<ext>` or `space-icon.<ext>` for the uploaded type. */
fun pictureFilename(target: PictureTarget, contentType: String): String {
    val extension = when (contentType) {
        "image/gif" -> "gif"
        "image/png" -> "png"
        "image/jpeg" -> "jpg"
        else -> "webp"
    }
    return (if (target is PictureTarget.SpaceIcon) "space-icon" else "avatar") + ".$extension"
}

/** Friendly copy for a failed picture change, upload or removal. */
fun pictureError(error: Throwable, target: PictureTarget): String = when (error) {
    is PictureException -> error.message
    is UploadException -> error.message
    is ApiException -> when {
        error.code == "picture_type" -> "Use a PNG, JPEG, WebP or GIF picture."
        error.code == "picture_too_large" || error.status == 413 -> "That picture is too large. Try another one."
        error.code == "picture_invalid" || error.status == 422 -> PicturePolicy.UNUSABLE
        error.code == "picture_rate_limited" || error.status == 429 -> "You’ve changed pictures too many times today. Try again later."
        error.status == 503 -> "Picture uploads aren’t available right now."
        error.status == 409 -> "The upload didn’t finish. Try again."
        error.status == 404 && target is PictureTarget.SpaceIcon -> "Only the space owner can change its icon."
        error.status == 400 -> "Pictures can be up to 1024 × 1024."
        else -> friendlyError(error)
    }
    else -> friendlyError(error)
}

/**
 * Reserve a picture upload, PUT the bytes straight to storage with exactly the presigned
 * headers, then confirm: the server checks the stored file and applies it at once, answering
 * with the account (profile picture) or the space (icon). A failure cancels the reservation.
 */
class PictureUploader(
    private val api: CaperApi,
    /** Waits between `/complete` attempts while storage reports the upload has not arrived (409). */
    private val completeRetryDelaysMs: List<Long> = listOf(500, 1_000, 2_000, 4_000),
) {
    suspend fun upload(token: String, target: PictureTarget, picture: PreparedPicture): PictureCompletion {
        val size = picture.bytes.size.toLong()
        if (size < 1) throw PictureException(PicturePolicy.UNUSABLE)
        val reservation = api.createPicture(
            token, (target as? PictureTarget.SpaceIcon)?.spaceId, pictureFilename(target, picture.contentType),
            picture.contentType, size, picture.width, picture.height,
        )
        try {
            currentCoroutineContext().ensureActive()
            api.putUpload(reservation.upload, size, { ByteArrayInputStream(picture.bytes) })
            val result = complete(token, reservation.id)
            currentCoroutineContext().ensureActive()
            val applied = when (target) {
                PictureTarget.Avatar -> result.account != null
                is PictureTarget.SpaceIcon -> result.space?.id == target.spaceId
            }
            if (result.id != reservation.id || !applied) throw UploadException("The upload service returned an invalid response.")
            return result
        } catch (error: Throwable) {
            api.cancelAsset(token, reservation.id)
            throw error
        }
    }

    private suspend fun complete(token: String, id: String): PictureCompletion {
        for (wait in completeRetryDelaysMs) {
            try {
                return api.completePicture(token, id)
            } catch (error: ApiException) {
                if (error.status != 409) throw error
            }
            delay(wait)
        }
        return api.completePicture(token, id)
    }
}
