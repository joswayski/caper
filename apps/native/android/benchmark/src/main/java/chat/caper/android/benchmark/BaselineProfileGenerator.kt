package chat.caper.android.benchmark

import androidx.benchmark.macro.junit4.BaselineProfileRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Generates the app's Baseline Profile (`./gradlew :app:generateBaselineProfile` with a phone
 * connected and the bench fixture running): launch to the restored conversation, open the long
 * channel, read back through its history and type a reply.
 */
@RunWith(AndroidJUnit4::class)
class BaselineProfileGenerator {
    @get:Rule val rule = BaselineProfileRule()

    @Test fun generate() = rule.collect(PACKAGE, includeInStartupProfile = true) {
        pressHome()
        startActivityAndWait()
        signIn()
        openChannel(OTHER_CHANNEL)
        openChannel(LONG_CHANNEL)
        scrollHistory(pages = 3)
        typeMessage()
    }
}
