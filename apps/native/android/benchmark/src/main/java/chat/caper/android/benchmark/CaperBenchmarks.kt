package chat.caper.android.benchmark

import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.benchmark.macro.FrameTimingMetric
import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.StartupTimingMetric
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.Parameterized

/**
 * Cold and warm start, opening the long conversation, scrolling its history, typing, and typing into a
 * long formatted draft, each without ahead-of-time compilation ("before") and with the Baseline Profile
 * ("after"). Run on a phone or an arm64 emulator with `./gradlew :benchmark:connectedBenchmarkReleaseAndroidTest`; see ../README.md.
 */
@RunWith(Parameterized::class)
class CaperBenchmarks(private val compilation: CompilationMode) {
    companion object {
        @JvmStatic @Parameterized.Parameters(name = "{0}")
        fun modes() = listOf(CompilationMode.None(), CompilationMode.Partial(BaselineProfileMode.Require))
    }

    @get:Rule val rule = MacrobenchmarkRule()

    /** Time to first frame, and to the restored conversation (`ReportDrawnWhen` in the app). */
    @Test fun coldStart() = rule.measureRepeated(
        PACKAGE, listOf(StartupTimingMetric()), compilation, startupMode = StartupMode.COLD, iterations = 10,
        setupBlock = { pressHome(); startActivityAndWait(); signIn(); openChannel(LONG_CHANNEL); pressHome() },
    ) { startActivityAndWait() }

    /** Returning to the app with its process still alive: the activity and its UI are recreated. */
    @Test fun warmStart() = rule.measureRepeated(
        PACKAGE, listOf(StartupTimingMetric()), compilation, startupMode = StartupMode.WARM, iterations = 10,
        setupBlock = { pressHome(); startActivityAndWait(); signIn(); openChannel(LONG_CHANNEL); pressHome() },
    ) { startActivityAndWait() }

    /** Switching from a short channel to the 3,000-message one. */
    @Test fun openConversation() = rule.measureRepeated(
        PACKAGE, listOf(FrameTimingMetric()), compilation, startupMode = StartupMode.WARM, iterations = 10,
        setupBlock = { startActivityAndWait(); signIn(); openChannel(OTHER_CHANNEL) },
    ) { openChannel(LONG_CHANNEL) }

    /** Reading back through four pages of history and returning to the latest message. */
    @Test fun scrollLongConversation() = rule.measureRepeated(
        PACKAGE, listOf(FrameTimingMetric()), compilation, startupMode = StartupMode.WARM, iterations = 5,
        setupBlock = { startActivityAndWait(); signIn(); openChannel(LONG_CHANNEL) },
    ) { scrollHistory() }

    /** Typing a sentence into the long conversation's composer. */
    @Test fun typing() = rule.measureRepeated(
        PACKAGE, listOf(FrameTimingMetric()), compilation, startupMode = StartupMode.WARM, iterations = 10,
        setupBlock = { startActivityAndWait(); signIn(); openChannel(LONG_CHANNEL) },
    ) { typeMessage() }

    /** Typing into [LONG_DRAFT]: the composer formats and finds links in the whole draft on every key. */
    @Test fun typingLongDraft() = rule.measureRepeated(
        PACKAGE, listOf(FrameTimingMetric()), compilation, startupMode = StartupMode.WARM, iterations = 5,
        setupBlock = { startActivityAndWait(); signIn(); openChannel(LONG_CHANNEL); enterDraft(LONG_DRAFT) },
    ) { typeMessage() }
}
