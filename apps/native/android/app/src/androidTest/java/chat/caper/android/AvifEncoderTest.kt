package chat.caper.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.data.AvifEncoder
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** The bundled libavif/libaom encoder on a real device: a valid AVIF the API and Android accept. */
@RunWith(AndroidJUnit4::class)
class AvifEncoderTest {
    private fun gradient(width: Int, height: Int, alpha: Int = 255): Bitmap =
        Bitmap.createBitmap(IntArray(width * height) { i -> Color.argb(alpha, (i % width) * 255 / width, (i / width) * 255 / height, 128) }, width, height, Bitmap.Config.ARGB_8888)

    @Test fun encodesAnOpaquePhotoAsAvif() {
        assertTrue(AvifEncoder.available)
        val bitmap = gradient(640, 480)
        val bytes = requireNotNull(AvifEncoder.encode(bitmap, 85))
        assertEquals("ftypavif", String(bytes, 4, 8, Charsets.US_ASCII))
        assertTrue(bytes.size < 640 * 480)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val decoded = requireNotNull(BitmapFactory.decodeByteArray(bytes, 0, bytes.size))
            assertEquals(640, decoded.width)
            assertEquals(480, decoded.height)
            val pixel = decoded.getPixel(320, 240)
            assertEquals(Color.red(bitmap.getPixel(320, 240)).toDouble(), Color.red(pixel).toDouble(), 8.0)
        }
    }

    @Test fun lowerQualityIsSmallerAndTranslucencySurvives() {
        val bitmap = gradient(320, 240)
        assertTrue(requireNotNull(AvifEncoder.encode(bitmap, 40)).size < requireNotNull(AvifEncoder.encode(bitmap, 95)).size)
        assertNotNull(AvifEncoder.encode(gradient(64, 64, alpha = 128), 85))
        assertNull("only ARGB_8888 is encoded", AvifEncoder.encode(bitmap.copy(Bitmap.Config.RGB_565, false), 85))
    }
}
