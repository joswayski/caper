package chat.caper.android.data

import android.graphics.Bitmap
import android.graphics.ColorSpace
import android.util.Log

/**
 * AVIF photo encoding (docs/media.md "Client compression and previews") through
 * `libcaper_avif.so`: libavif 1.4.2 over libaom 3.15.1 (BSD-2-Clause, AOM patent licence;
 * prepare-avif.sh), 8-bit 4:2:0. Android's `Bitmap.compress` cannot write AVIF.
 *
 * `quality` is libavif's quality scale, passed straight through. libavif 1.4 defaults stills to
 * libaom's `tune=iq`, which uses its own quality-to-quantizer table, so the native side pins
 * `tune=ssim` (avifenc 1.0's default) to keep the scale the server's `avifQuality` is defined
 * on. Measured on the five benchmark photos (9–14 MP, host build of this exact code) against
 * `avifenc -q 85 -s 6 -y 420` (1.0.4, 83.27 SSIMULACRA2 mean): q85 speed 6 scored 83.35 (each
 * photo within 0.2) at 99.9% of its bytes, and speed 8 ([SPEED]) 83.18 at 100.4%. With tune=iq
 * q85 scored 85.60 at 102% of the bytes and about 3x the encode time, i.e. a different scale.
 */
object AvifEncoder {
    /**
     * libaom speed. On the host, 4 threads, 9–14 MP: speed 6 took 1.9 s, speed 8 0.9 s for 0.5%
     * more bytes and -0.17 SSIMULACRA2; mid-range phones are a few times slower, so 8 keeps a
     * 12 MP photo to a few seconds.
     */
    const val SPEED = 8
    private const val MAX_THREADS = 4

    // AV1 colour primaries (ISO/IEC 23091-2): BT.709/sRGB and SMPTE EG 432-1 (Display P3).
    internal const val PRIMARIES_SRGB = 1
    internal const val PRIMARIES_DISPLAY_P3 = 12

    private val loaded: Boolean by lazy {
        runCatching { System.loadLibrary("caper_avif") }.onFailure { Log.w("AvifEncoder", "AVIF encoder unavailable", it) }.isSuccess
    }

    val available: Boolean get() = loaded

    /**
     * Encoded AVIF bytes, or null when this bitmap cannot be encoded (no native library, not
     * ARGB_8888, a colour space other than sRGB/Display P3, or an encoder error).
     */
    fun encode(bitmap: Bitmap, quality: Int): ByteArray? {
        if (!loaded || bitmap.config != Bitmap.Config.ARGB_8888) return null
        val primaries = primaries(bitmap.colorSpace) ?: return null
        val threads = Runtime.getRuntime().availableProcessors().coerceIn(1, MAX_THREADS)
        return runCatching { nativeEncode(bitmap, bitmap.isPremultiplied, primaries, quality.coerceIn(1, 100), SPEED, threads) }
            .getOrNull()?.takeIf { isAvif(it) }
    }

    /**
     * The WebP path tags the bitmap's own colour space; AVIF does the same with CICP for the two
     * spaces photos decode into. Anything else (Adobe RGB, linear, HDR) keeps the WebP path.
     */
    internal fun primaries(colorSpace: ColorSpace?): Int? = when (colorSpace) {
        null, ColorSpace.get(ColorSpace.Named.SRGB) -> PRIMARIES_SRGB
        ColorSpace.get(ColorSpace.Named.DISPLAY_P3) -> PRIMARIES_DISPLAY_P3
        else -> null
    }

    /** ISO BMFF `ftyp` with a major brand the API's `complete` sniffing accepts. */
    internal fun isAvif(bytes: ByteArray): Boolean =
        bytes.size >= 12 && String(bytes, 4, 4, Charsets.US_ASCII) == "ftyp" && String(bytes, 8, 4, Charsets.US_ASCII) in setOf("avif", "avis", "mif1", "msf1")

    @JvmStatic private external fun nativeEncode(bitmap: Bitmap, premultiplied: Boolean, colorPrimaries: Int, quality: Int, speed: Int, threads: Int): ByteArray?
}
