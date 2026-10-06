package chat.caper.android.data

import android.content.ContentResolver
import android.content.res.AssetFileDescriptor
import android.net.Uri
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import java.io.InputStream
import java.util.Locale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * A picked photo, video or document streamed straight from its `content://` URI. Nothing is
 * copied or re-encoded on the device; the server's media worker compresses the original.
 */
class PickedContent private constructor(
    private val resolver: ContentResolver,
    val uri: Uri,
    override val name: String,
    override val contentType: String,
    override val size: Long,
) : UploadSource {
    override fun open(): InputStream = resolver.openInputStream(uri) ?: throw UploadException("This file could not be read.")

    companion object {
        /** Reads the display name, declared type and exact byte size of [uri]. */
        suspend fun resolve(resolver: ContentResolver, uri: Uri): PickedContent = withContext(Dispatchers.IO) {
            var displayName: String? = null
            var columnSize: Long? = null
            runCatching {
                resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)
                        if (nameIndex >= 0 && !cursor.isNull(nameIndex)) displayName = cursor.getString(nameIndex)
                        if (sizeIndex >= 0 && !cursor.isNull(sizeIndex)) columnSize = cursor.getLong(sizeIndex)
                    }
                }
            }
            val name = (displayName ?: uri.lastPathSegment ?: "file").substringAfterLast('/').filterNot { it.isISOControl() }
                .trim().ifEmpty { "file" }.take(255)
            val extension = name.substringAfterLast('.', "").lowercase(Locale.ROOT)
            val type = AttachmentPolicy.normalizedType(
                resolver.getType(uri) ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension),
            )
            // The presigned PUT signs the exact length: prefer the descriptor's length, then the
            // provider's SIZE column, and count the bytes once only when neither is known.
            val descriptorSize = runCatching {
                resolver.openAssetFileDescriptor(uri, "r")?.use { it.length.takeIf { length -> length != AssetFileDescriptor.UNKNOWN_LENGTH } }
            }.getOrNull()
            val size = descriptorSize ?: columnSize?.takeIf { it >= 0 } ?: run {
                val input = resolver.openInputStream(uri) ?: throw UploadException("This file could not be read.")
                input.use { stream ->
                    val buffer = ByteArray(64 * 1024)
                    var total = 0L
                    while (true) { val read = stream.read(buffer); if (read < 0) break; total += read }
                    total
                }
            }
            PickedContent(resolver, uri, name, type, size)
        }
    }
}
