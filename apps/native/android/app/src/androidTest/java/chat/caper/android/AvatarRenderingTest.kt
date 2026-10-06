package chat.caper.android

import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.time.Clock
import java.time.Instant
import java.time.ZoneOffset
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AvatarRenderingTest {
    @Test fun legacyLauncherAliasesKeepTheOriginalIconWithoutSwitchingEntries() = runBlocking {
        // Component mutations belong only to the disposable fixture APK.
        assertTrue(BuildConfig.FIXTURE_MODE)
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val manager = context.packageManager
        val default = ComponentName(context, "chat.caper.android.launcher.Default")
        val legacy = (0 until 800).map { ComponentName(context, "chat.caper.android.launcher.Avatar$it") }
        val components = listOf(default) + legacy
        val originalStates = components.associateWith(manager::getComponentEnabledSetting)
        components.forEach { component ->
            assertEquals("Legacy shortcut ${component.className} must use the plain mascot",
                R.drawable.ic_caper_app, manager.getActivityInfo(component, PackageManager.GET_DISABLED_COMPONENTS).icon)
        }
        val preferences = context.getSharedPreferences("launcher_avatar", Context.MODE_PRIVATE)
        val savedDay = preferences.all["utc_day"] as? Long
        val savedIndex = preferences.all["avatar_index"] as? Int
        val flags = PackageManager.DONT_KILL_APP
        try {
            // Model an upgrade with the last legacy alias enabled and today's choice saved.
            manager.setComponentEnabledSetting(legacy[799], PackageManager.COMPONENT_ENABLED_STATE_ENABLED, flags)
            manager.setComponentEnabledSetting(default, PackageManager.COMPONENT_ENABLED_STATE_DISABLED, flags)
            val upgradedStates = components.associateWith(manager::getComponentEnabledSetting)
            preferences.edit().putLong("utc_day", 20_000).putInt("avatar_index", 799).commit()
            val now = Instant.ofEpochSecond(20_000L * 86_400)
            val sameDay = DailyBrandingAvatar(context, Clock.fixed(now, ZoneOffset.UTC)) { error("must not draw again") }
            assertEquals(799, sameDay.update())
            assertEquals(upgradedStates, components.associateWith(manager::getComponentEnabledSetting))

            val nextDay = DailyBrandingAvatar(context, Clock.fixed(now.plusSeconds(86_400), ZoneOffset.UTC)) { 46 }
            assertEquals("Only the in-app character rotates", 46, nextDay.update())
            assertEquals("A day rollover must not switch or disable the existing launcher entry",
                upgradedStates, components.associateWith(manager::getComponentEnabledSetting))
        } finally {
            // Keep an entry enabled while restoring the fixture's original state.
            manager.setComponentEnabledSetting(default, PackageManager.COMPONENT_ENABLED_STATE_ENABLED, flags)
            originalStates.forEach { (component, state) -> manager.setComponentEnabledSetting(component, state, flags) }
            preferences.edit().apply {
                if (savedDay == null) remove("utc_day") else putLong("utc_day", savedDay)
                if (savedIndex == null) remove("avatar_index") else putInt("avatar_index", savedIndex)
            }.commit()
        }
    }

    @Test fun brandingResourcesRemoveTheTileWithoutChangingProfileResources() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        assertEquals(800, caperBrandingResources.size)
        caperBrandingResources.forEachIndexed { index, resource ->
            assertNotNull("Missing bundled branding $index", context.getDrawable(resource))
        }
        listOf(0, 46, 80, 537, 799).forEach { index ->
            val image = Bitmap.createBitmap(256, 256, Bitmap.Config.ARGB_8888)
            try {
                val branding = requireNotNull(context.getDrawable(caperBrandingResources[index]))
                branding.setBounds(0, 0, 256, 256)
                branding.draw(Canvas(image))
                assertEquals("Branding $index must not have a circular tile", 0, Color.alpha(image.getPixel(252, 128)))
                val pixels = IntArray(256 * 256)
                image.getPixels(pixels, 0, 256, 0, 0, 256, 256)
                assertTrue("Branding $index must keep its colors", pixels.filter { Color.alpha(it) == 255 }.toSet().size > 3)
                image.eraseColor(Color.TRANSPARENT)
                val avatar = requireNotNull(context.getDrawable(caperAvatarResources[index]))
                avatar.setBounds(0, 0, 256, 256)
                avatar.draw(Canvas(image))
                assertEquals("Profile $index must retain its tile", 255, Color.alpha(image.getPixel(252, 128)))
            } finally {
                image.recycle()
            }
        }
    }

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
