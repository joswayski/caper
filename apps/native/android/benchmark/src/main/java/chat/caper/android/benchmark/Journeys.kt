package chat.caper.android.benchmark

import androidx.benchmark.macro.MacrobenchmarkScope
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Until

/** The app's `benchmarkRelease` and `nonMinifiedRelease` builds install beside the shipped app. */
internal const val PACKAGE = "chat.caper.android.benchmark"

/** The bench fixture's long conversation (`CAPER_BENCH_MESSAGES=3000`). */
internal const val LONG_CHANNEL = "general"
internal const val OTHER_CHANNEL = "design"

private const val WAIT_MS = 15_000L

private fun MacrobenchmarkScope.waitFor(text: String): Boolean = device.wait(Until.hasObject(By.text(text)), WAIT_MS)

/** Signs in with the fixture's account when the app shows its login screen; otherwise does nothing. */
internal fun MacrobenchmarkScope.signIn() {
    // Launch shows the brand screen until it knows whether a session is stored.
    val deadline = System.currentTimeMillis() + WAIT_MS
    while (System.currentTimeMillis() < deadline && !device.hasObject(By.text("Email me a code")) && !device.hasObject(By.text("Browse spaces")) &&
        !device.hasObject(By.textStartsWith("Message #"))) Thread.sleep(100)
    if (!device.hasObject(By.text("Email me a code"))) return
    device.findObject(By.clazz("android.widget.EditText")).text = "fixture@example.test"
    device.findObject(By.text("Email me a code")).click()
    check(waitFor("Check your email.")) { "The fixture didn't send a code. Is it running, with adb reverse tcp:3001 tcp:3001?" }
    device.findObject(By.clazz("android.widget.EditText")).text = "ABC234"
    device.findObject(By.text("Continue")).click()
    device.wait(Until.gone(By.text("Continue")), WAIT_MS)
}

/**
 * Opens a Fixture Studio channel from the sidebar, or from Browse on narrow screens: the open
 * conversation's back arrow, or an empty channel's "Browse spaces".
 */
internal fun MacrobenchmarkScope.openChannel(name: String) {
    if (device.wait(Until.hasObject(By.text("Message #$name")), 2_000)) return
    if (!device.hasObject(By.text(name))) (device.findObject(By.desc("Back to Browse")) ?: device.findObject(By.text("Browse spaces")))?.click()
    check(waitFor(name)) { "Channel #$name isn't listed." }
    device.findObject(By.text(name)).click()
    check(waitFor("Message #$name")) { "#$name didn't open." }
    device.waitForIdle()
}

/**
 * Scrolls up through [pages] pages of history, loading each older page as it is reached, then
 * back down to the latest message: what reading back through a busy channel does.
 */
internal fun MacrobenchmarkScope.scrollHistory(pages: Int = 4) {
    val width = device.displayWidth
    val height = device.displayHeight
    // Right of a wide layout's sidebars, and clear of a phone's edge swipes.
    val x = (width * 0.62f).toInt()
    repeat(pages) {
        repeat(6) { device.swipe(x, (height * 0.35f).toInt(), x, (height * 0.8f).toInt(), 8) }
        device.findObject(By.text("Load older messages"))?.let { it.click(); device.wait(Until.hasObject(By.text("Load older messages")), WAIT_MS) }
    }
    repeat(pages * 8) { device.swipe(x, (height * 0.8f).toInt(), x, (height * 0.35f).toInt(), 8) }
    device.waitForIdle()
}

/** A bold, italic and link-heavy draft of 3,575 characters, near the 4,000-character limit. */
internal val LONG_DRAFT = "**Release** notes for _today_, see https://example.com/notes and ".repeat(55)

/** Puts [text] in the open channel's composer in one edit, as a paste would. */
internal fun MacrobenchmarkScope.enterDraft(text: String) {
    device.findObject(By.clazz("android.widget.EditText")).text = text
    device.waitForIdle()
}

/** Types into the open channel's composer one key at a time, then clears the draft. */
internal fun MacrobenchmarkScope.typeMessage(text: String = "the release notes look great, shipping after lunch") {
    val field = device.findObject(By.clazz("android.widget.EditText"))
    field.click()
    device.waitForIdle()
    // `input text` sends a key event per character, as a keyboard would.
    text.split(' ').forEachIndexed { index, word ->
        if (index > 0) device.executeShellCommand("input keyevent KEYCODE_SPACE")
        device.executeShellCommand("input text $word")
    }
    device.waitForIdle()
    field.clear()
    device.pressBack()
}
