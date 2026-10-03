package chat.caper.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AvatarRenderingTest {
    @Test fun shippedAvatarsResolveAndRenderOriginalColors() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        assertEquals(800, caperAvatarResources.size)
        caperAvatarResources.forEachIndexed { index, resource ->
            assertNotNull("Missing bundled avatar $index", context.getDrawable(resource))
        }
        listOf(0, 31, 32, 799).forEach { index ->
            val image = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888)
            try {
                val drawable = requireNotNull(context.getDrawable(caperAvatarResources[index]))
                drawable.setBounds(0, 0, 64, 64)
                drawable.draw(Canvas(image))
                val pixels = IntArray(64 * 64)
                image.getPixels(pixels, 0, 64, 0, 0, 64, 64)
                val colors = pixels.filter { Color.alpha(it) == 255 }.toSet()
                assertTrue("Avatar $index must contain original multicolor artwork", colors.size > 3)
                assertEquals("The corner must remain transparent", 0, Color.alpha(image.getPixel(0, 0)))
                assertTrue("The center cannot be blank", Color.alpha(image.getPixel(32, 32)) > 0)
            } finally {
                image.recycle()
            }
        }
    }
}
