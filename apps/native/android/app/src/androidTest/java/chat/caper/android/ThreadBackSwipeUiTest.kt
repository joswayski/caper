package chat.caper.android

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ThreadBackSwipeUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun movingContentTracksStationaryCoordinatesAndSettlesOnReleaseOrCancel() {
        var offset by mutableFloatStateOf(0f)
        var released: Boolean? = null
        var density = 1f
        compose.setContent {
            density = LocalDensity.current.density
            Box(Modifier.fillMaxSize().testTag("stationary")) {
                Box(Modifier.fillMaxSize().graphicsLayer { translationX = offset }) {
                    Box(Modifier.fillMaxSize().threadBackSwipe(true, { offset = it }, {
                        released = it
                        offset = 0f
                    }))
                }
            }
        }
        val root = compose.onNodeWithTag("stationary")
        val start = Offset(12f * density, 100f * density)
        // Split events across frames so the next event sees an actually translated layer.
        root.performTouchInput { down(start); moveTo(start + Offset(36f * density, 0f)) }
        compose.runOnIdle { assertEquals(36f * density, offset, 1f); assertNull(released) }
        root.performTouchInput { moveTo(start + Offset(80f * density, 0f)) }
        compose.runOnIdle { assertEquals(80f * density, offset, 1f) }
        root.performTouchInput { up() }
        compose.runOnIdle { assertEquals(true, released); assertEquals(0f, offset) }

        root.performTouchInput { down(start); moveTo(start + Offset(36f * density, 0f)); up() }
        compose.runOnIdle { assertEquals(false, released); assertEquals(0f, offset) }
        root.performTouchInput { down(start); moveTo(start + Offset(80f * density, 0f)); cancel() }
        compose.runOnIdle { assertEquals(false, released); assertEquals(0f, offset) }
    }
}
