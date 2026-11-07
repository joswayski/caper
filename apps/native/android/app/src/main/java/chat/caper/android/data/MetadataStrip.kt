package chat.caper.android.data

import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/** Shared Rust container cleaning; originals and encoded outputs take the same upload path. */
internal object MetadataStrip {
    const val REFUSED = "Caper can't remove hidden details (like location) from this file, so it can't be sent."

    /** [output] must not exist. The Rust bridge streams into it without reading media into RAM. */
    fun clean(input: File, output: File, contentType: String) {
        when (CaperMobile.cleanMetadata(input.path, output.path, contentType)) {
            0 -> Unit
            1, 2 -> throw UploadException(REFUSED)
            else -> throw UploadException("This file couldn't be prepared. Check available storage and try again.")
        }
    }

    /** Keep one immutable cleaned file for upload retries and multipart ranges, then remove it. */
    suspend fun <T> withCleanFile(input: File, contentType: String, action: suspend (File) -> T): T {
        val output = File(input.parentFile, ".clean-${UUID.randomUUID()}")
        try {
            withContext(Dispatchers.IO) { clean(input, output, contentType) }
            currentCoroutineContext().ensureActive()
            return action(output)
        } finally {
            output.delete()
        }
    }

    /** Pictures are already bounded to 5 MiB by PicturePolicy; audio/video never use this path. */
    suspend fun cleanPicture(bytes: ByteArray, contentType: String): ByteArray = withContext(Dispatchers.IO) {
        val input = File.createTempFile("caper-picture-", ".input")
        try {
            input.writeBytes(bytes)
            withCleanFile(input, contentType) { it.readBytes() }
        } finally {
            input.delete()
        }
    }
}
