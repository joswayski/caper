package chat.caper.android

import android.os.Build
import android.os.SystemClock
import android.view.HapticFeedbackConstants
import android.view.View

/**
 * Caper's small set of haptics, kept light so they confirm an action rather than announce it.
 * They go through [View.performHapticFeedback], so they follow the system's touch feedback setting.
 */
internal object Haptics {
    private var lastArrival = 0L

    /** A send, a reaction, play or pause, a copy. */
    fun tap(view: View) {
        view.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
    }

    /** A switch such as mute or deafen, or joining and leaving a call. */
    fun toggle(view: View, on: Boolean) {
        view.performHapticFeedback(
            if (Build.VERSION.SDK_INT >= 34) if (on) HapticFeedbackConstants.TOGGLE_ON else HapticFeedbackConstants.TOGGLE_OFF
            else HapticFeedbackConstants.KEYBOARD_TAP,
        )
    }

    /**
     * Someone else's message arriving while the reader watches the conversation: the faintest tick,
     * at most once every 1.5 seconds, and only while Caper's window has focus (not behind a sheet,
     * the notification shade or another app).
     */
    fun arrival(view: View) {
        val now = SystemClock.elapsedRealtime()
        if (now - lastArrival < 1_500 || !view.hasWindowFocus()) return
        lastArrival = now
        view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
    }
}
