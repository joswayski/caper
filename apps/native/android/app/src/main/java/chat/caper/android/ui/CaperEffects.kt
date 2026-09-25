package chat.caper.android.ui

import android.content.Context
import android.media.AudioAttributes
import android.media.SoundPool
import android.os.SystemClock

/**
 * Decorative UI feedback with the web assets, gain, pitch and timing limits
 * (apps/web/src/audio/effects.ts). Sonification mixes with other apps' audio
 * and never takes audio focus or changes the call route.
 */
object CaperEffects {
    enum class Effect(val asset: String) {
        ToggleOff("toggle-off"), ToggleOn("toggle-on"), Slider("slider-tick"), Leave("channel-leave"),
        Disconnect("disconnect"), Warning("warning"), Join("channel-join"), Message("new-message"), Delete("delete"),
    }

    private const val PREFERENCE = "soundEffects"
    private var pool: SoundPool? = null
    private val sounds = mutableMapOf<Effect, Int>()
    private val loaded = mutableSetOf<Int>()
    private var preferences: android.content.SharedPreferences? = null
    private var lastSlider = Long.MIN_VALUE / 2

    var enabled: Boolean
        get() = preferences?.getBoolean(PREFERENCE, true) ?: false
        set(value) {
            preferences?.edit()?.putBoolean(PREFERENCE, value)?.apply()
            if (!value) pool?.autoPause()
        }

    fun init(context: Context) {
        if (pool != null) return
        preferences = context.getSharedPreferences("audio", Context.MODE_PRIVATE)
        // Parity fixtures stay silent, like the Apple parity runs.
        if (chat.caper.android.BuildConfig.FIXTURE_MODE) return
        val next = SoundPool.Builder().setMaxStreams(4).setAudioAttributes(
            AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ASSISTANCE_SONIFICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build(),
        ).build()
        next.setOnLoadCompleteListener { _, id, status -> if (status == 0) synchronized(loaded) { loaded += id } }
        Effect.entries.forEach { effect ->
            runCatching { context.assets.openFd("effects/${effect.asset}.wav").use { sounds[effect] = next.load(it, 1) } }
        }
        pool = next
    }

    /** Web: gain is volume × 0.6 (default volume 0.45), playback rate clamped to 0.5–2. */
    fun play(effect: Effect, volume: Float = 0.45f, rate: Float = 1f) {
        if (!enabled) return
        val current = pool ?: return
        val id = sounds[effect] ?: return
        if (synchronized(loaded) { id !in loaded }) return
        val gain = volume.coerceIn(0f, 1f) * 0.6f
        current.play(id, gain, gain, 1, 0, rate.coerceIn(0.5f, 2f))
    }

    fun toggle(on: Boolean) = play(if (on) Effect.ToggleOn else Effect.ToggleOff)

    /** Slider feedback rises in pitch and loudness, at most every 40 ms. */
    fun slider(normalized: Float) {
        val now = SystemClock.elapsedRealtime()
        if (now - lastSlider < 40) return
        lastSlider = now
        val value = normalized.coerceIn(0f, 1f)
        play(Effect.Slider, 0.1f + value * 0.22f, 0.75f + value * 0.6f)
    }
}
