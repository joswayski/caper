package chat.caper.android.data

import java.io.ByteArrayOutputStream
import java.io.File
import java.io.RandomAccessFile

/**
 * Lossless metadata removal for files uploaded as originals (not re-encoded), so location and
 * camera metadata do not leave the device. Media data is never decoded or changed: only
 * container metadata is dropped or neutralised. Every function returns false (and leaves
 * [output] absent) when the input is not the expected format or holds nothing to remove.
 */
internal object MetadataStrip {
    // --- MP4 / QuickTime ------------------------------------------------------------------------

    private class Box(val start: Long, val headerSize: Int, val size: Long, val type: String) {
        val contentStart get() = start + headerSize
        val end get() = start + size
    }

    private fun RandomAccessFile.boxes(from: Long, to: Long): List<Box>? {
        val result = mutableListOf<Box>()
        var at = from
        val header = ByteArray(16)
        while (at + 8 <= to) {
            seek(at); readFully(header, 0, 8)
            var size = u32(header, 0)
            val type = String(header, 4, 4, Charsets.ISO_8859_1)
            var headerSize = 8
            when (size) {
                1L -> {
                    if (at + 16 > to) return null
                    readFully(header, 8, 8)
                    size = (u32(header, 8) shl 32) or u32(header, 12)
                    headerSize = 16
                }
                0L -> size = to - at
            }
            if (size < headerSize || at + size > to) return null
            result += Box(at, headerSize, size, type)
            at += size
        }
        return result
    }

    private fun u32(bytes: ByteArray, at: Int): Long =
        ((bytes[at].toLong() and 0xFF) shl 24) or ((bytes[at + 1].toLong() and 0xFF) shl 16) or
            ((bytes[at + 2].toLong() and 0xFF) shl 8) or (bytes[at + 3].toLong() and 0xFF)

    /** Every `udta`/`meta` box directly under `moov` and each `moov/trak`; null when not an ISO-BMFF file. */
    private fun mp4MetadataBoxes(file: File): List<Box>? = runCatching {
        RandomAccessFile(file, "r").use { raf ->
            val top = raf.boxes(0, raf.length()) ?: return@use null
            if (top.none { it.type == "ftyp" || it.type == "moov" }) return@use null
            val found = mutableListOf<Box>()
            for (moov in top.filter { it.type == "moov" }) {
                val children = raf.boxes(moov.contentStart, moov.end) ?: return@use null
                for (child in children) {
                    if (child.type == "udta" || child.type == "meta") found += child
                    if (child.type == "trak") {
                        val trak = raf.boxes(child.contentStart, child.end) ?: return@use null
                        found += trak.filter { it.type == "udta" || it.type == "meta" }
                    }
                }
            }
            found
        }
    }.getOrNull()

    /**
     * Copies an MP4/MOV to [output] with every `udta` and `meta` box under `moov` (and under each
     * track) turned into a zero-filled `free` box of the same size, so no sample offsets move and
     * media bytes are untouched. Removes `©xyz` and `com.apple.quicktime.location.ISO6709`
     * locations (zeroing matters: a renamed box would still carry the coordinates as bytes).
     */
    fun mp4(input: File, output: File): Boolean {
        val boxes = mp4MetadataBoxes(input)?.takeIf { it.isNotEmpty() } ?: return false
        return runCatching {
            input.copyTo(output, overwrite = true)
            RandomAccessFile(output, "rw").use { raf ->
                val zeros = ByteArray(64 * 1024)
                for (box in boxes) {
                    raf.seek(box.start + 4); raf.write("free".toByteArray(Charsets.ISO_8859_1))
                    var at = box.contentStart
                    raf.seek(at)
                    while (at < box.end) {
                        val count = minOf(zeros.size.toLong(), box.end - at).toInt()
                        raf.write(zeros, 0, count); at += count
                    }
                }
            }
            true
        }.getOrElse { output.delete(); false }
    }

    // --- JPEG -----------------------------------------------------------------------------------

    private fun ByteArray.startsWithAscii(at: Int, text: String) =
        size >= at + text.length && text.indices.all { this[at + it] == text[it].code.toByte() }

    /** EXIF orientation (1–8) from an APP1 `Exif\0\0` payload, or null when absent/unreadable. */
    internal fun exifOrientation(payload: ByteArray): Int? {
        if (!payload.startsWithAscii(0, "Exif\u0000\u0000")) return null
        val tiff = 6
        if (payload.size < tiff + 8) return null
        val little = when {
            payload.startsWithAscii(tiff, "II") -> true
            payload.startsWithAscii(tiff, "MM") -> false
            else -> return null
        }
        fun u16(at: Int): Int = if (little) (payload[at].toInt() and 0xFF) or ((payload[at + 1].toInt() and 0xFF) shl 8)
            else ((payload[at].toInt() and 0xFF) shl 8) or (payload[at + 1].toInt() and 0xFF)
        fun u32(at: Int): Long = if (little) (u16(at).toLong() or (u16(at + 2).toLong() shl 16)) else ((u16(at).toLong() shl 16) or u16(at + 2).toLong())
        val ifd = tiff + u32(tiff + 4).toInt()
        if (ifd < tiff || ifd + 2 > payload.size) return null
        val count = u16(ifd)
        for (index in 0 until count) {
            val entry = ifd + 2 + index * 12
            if (entry + 12 > payload.size) return null
            if (u16(entry) == 0x0112) return u16(entry + 8).takeIf { it in 1..8 }
        }
        return null
    }

    /** A minimal APP1 segment carrying only the Orientation tag (big-endian TIFF). */
    internal fun orientationSegment(orientation: Int): ByteArray {
        val payload = byteArrayOf(
            'E'.code.toByte(), 'x'.code.toByte(), 'i'.code.toByte(), 'f'.code.toByte(), 0, 0,
            'M'.code.toByte(), 'M'.code.toByte(), 0, 42, 0, 0, 0, 8, // TIFF header, IFD0 at 8
            0, 1, // one entry
            0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, orientation.toByte(), 0, 0, // Orientation, SHORT, 1
            0, 0, 0, 0, // no next IFD
        )
        val length = payload.size + 2
        return byteArrayOf(0xFF.toByte(), 0xE1.toByte(), (length ushr 8).toByte(), length.toByte()) + payload
    }

    /**
     * Rewrites a JPEG without APPn segments except APP0 (JFIF), APP2 `ICC_PROFILE` and APP14
     * (Adobe), so Exif (GPS, camera), XMP and maker data are dropped. A non-default orientation
     * is kept in a minimal Exif segment. Everything from the start of scan is copied unchanged.
     */
    fun jpeg(input: File, output: File): Boolean = runCatching {
        val bytes = input.readBytes()
        val stripped = jpeg(bytes) ?: return false
        output.writeBytes(stripped)
        true
    }.getOrElse { output.delete(); false }

    internal fun jpeg(bytes: ByteArray): ByteArray? {
        if (bytes.size < 4 || bytes[0] != 0xFF.toByte() || bytes[1] != 0xD8.toByte()) return null
        val out = ByteArrayOutputStream(bytes.size)
        out.write(bytes, 0, 2)
        var at = 2
        var removed = false
        var orientation: Int? = null
        var orientationAt = -1 // where in `out` the orientation segment goes
        var first = true
        while (true) {
            if (at + 4 > bytes.size || bytes[at] != 0xFF.toByte()) return null
            var markerAt = at
            while (markerAt + 1 < bytes.size && bytes[markerAt + 1] == 0xFF.toByte()) markerAt++ // fill bytes
            val marker = bytes[markerAt + 1].toInt() and 0xFF
            if (marker == 0xD9 || marker in 0xD0..0xD7 || marker == 0x01) return null // no scan before EOI: not a normal JPEG
            if (markerAt + 4 > bytes.size) return null
            val length = ((bytes[markerAt + 2].toInt() and 0xFF) shl 8) or (bytes[markerAt + 3].toInt() and 0xFF)
            val end = markerAt + 2 + length
            if (length < 2 || end > bytes.size) return null
            if (marker == 0xDA) {
                if (orientationAt < 0) orientationAt = out.size()
                if (!removed) return null
                val head = out.toByteArray()
                val result = ByteArrayOutputStream(bytes.size)
                result.write(head, 0, orientationAt)
                orientation?.takeIf { it != 1 }?.let { result.write(orientationSegment(it)) }
                result.write(head, orientationAt, head.size - orientationAt)
                result.write(bytes, at, bytes.size - at) // scan data and the rest, unchanged
                return result.toByteArray()
            }
            val payload = bytes.copyOfRange(markerAt + 4, end)
            val keep = when (marker) {
                0xE0, 0xEE -> true
                0xE2 -> payload.startsWithAscii(0, "ICC_PROFILE\u0000")
                in 0xE1..0xEF -> false
                else -> true
            }
            if (marker == 0xE1 && orientation == null) orientation = exifOrientation(payload)
            if (keep) {
                out.write(bytes, at, end - at)
                // The orientation segment follows a leading JFIF APP0, as JFIF requires APP0 first.
                if (first && marker == 0xE0) orientationAt = out.size()
            } else removed = true
            if (first && marker != 0xE0) orientationAt = 2
            first = false
            at = end
        }
    }

    // --- PNG ------------------------------------------------------------------------------------

    private val pngSignature = byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte(), 13, 10, 26, 10)
    private val pngMetadata = setOf("eXIf", "tEXt", "zTXt", "iTXt")

    /** Copies a PNG without its `eXIf`, `tEXt`, `zTXt` and `iTXt` chunks; image chunks are untouched. */
    fun png(input: File, output: File): Boolean = runCatching {
        val bytes = input.readBytes()
        val stripped = png(bytes) ?: return false
        output.writeBytes(stripped)
        true
    }.getOrElse { output.delete(); false }

    internal fun png(bytes: ByteArray): ByteArray? {
        if (bytes.size < 8 || !bytes.copyOf(8).contentEquals(pngSignature)) return null
        val out = ByteArrayOutputStream(bytes.size)
        out.write(bytes, 0, 8)
        var at = 8
        var removed = false
        while (at + 12 <= bytes.size) {
            val length = u32(bytes, at)
            val type = String(bytes, at + 4, 4, Charsets.ISO_8859_1)
            val end = at + 12 + length
            if (end > bytes.size) return null
            if (type in pngMetadata) removed = true else out.write(bytes, at, (end - at).toInt())
            at = end.toInt()
            if (type == "IEND") break
        }
        if (at < bytes.size) out.write(bytes, at, bytes.size - at)
        return if (removed) out.toByteArray() else null
    }

}
