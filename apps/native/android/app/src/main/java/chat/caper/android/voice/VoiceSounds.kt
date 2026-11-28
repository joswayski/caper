package chat.caper.android.voice

import android.content.Context
import android.media.AudioAttributes
import android.media.SoundPool

/** The voice chimes (`shared/sounds`, packaged as assets). */
internal enum class VoiceSound(val asset: String) { JOIN("voice-join.wav"), LEAVE("voice-leave.wav") }

/**
 * The chime one call update plays: yours when you get in or leave (a cancelled join stays silent),
 * and others' as they join or leave while you are in and not deafened. A reconnect keeps quiet.
 */
internal fun voiceSound(before: VoiceState, after: VoiceState): VoiceSound? {
    fun inCall(phase: VoiceState.Phase) = phase == VoiceState.Phase.CONNECTED || phase == VoiceState.Phase.RECONNECTING
    if (before.phase == VoiceState.Phase.CONNECTING && after.phase == VoiceState.Phase.CONNECTED) return VoiceSound.JOIN
    if (inCall(before.phase) && !inCall(after.phase)) return VoiceSound.LEAVE
    if (before.phase != VoiceState.Phase.CONNECTED || after.phase != VoiceState.Phase.CONNECTED || after.deafened) return null
    val self = after.selfId ?: return null
    if (before.selfId != self) return null
    val was = before.participants.map { it.id }.filter { it != self }.toSet()
    val now = after.participants.map { it.id }.filter { it != self }.toSet()
    return when {
        !was.containsAll(now) -> VoiceSound.JOIN
        !now.containsAll(was) -> VoiceSound.LEAVE
        else -> null
    }
}

/** Plays the chimes on the call's route (signalling, so they follow the earpiece or speaker). */
internal class VoiceSounds(context: Context) {
    private val pool = SoundPool.Builder().setMaxStreams(2).setAudioAttributes(
        AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION_SIGNALLING)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build(),
    ).build()
    private val ids = VoiceSound.entries.associateWith { sound -> context.assets.openFd(sound.asset).use { pool.load(it, 1) } }

    /** At the call's output volume (percent, capped at full volume). */
    fun play(sound: VoiceSound, volumePercent: Int) {
        val volume = volumePercent.coerceIn(0, 100) / 100f
        if (volume > 0f) pool.play(ids.getValue(sound), volume, volume, 1, 0, 1f)
    }
}
