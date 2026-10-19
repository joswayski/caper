package chat.caper.android.data

import kotlin.math.max
import kotlin.math.pow

/**
 * The overlay over a conversation while files are dragged onto it. Every client
 * produces the text in `shared/messages/drop-cases.json`, and the motion matches web
 * (chat.css `.chat-drop*`), Apple and Rust desktop (`file_drop.rs`). Change them together.
 */
object FileDrop {
    fun text(channel: String, direct: Boolean, attached: Int): Pair<String, String> {
        val room = max(0, AttachmentPolicy.MAX_ATTACHMENTS - attached)
        val detail = when (room) {
            0 -> "${AttachmentPolicy.MAX_ATTACHMENTS} files attached. Remove one to add another."
            AttachmentPolicy.MAX_ATTACHMENTS -> "Up to ${AttachmentPolicy.MAX_ATTACHMENTS} files. Add a message before sending."
            else -> "$room more file${if (room == 1) "" else "s"}. Add a message before sending."
        }
        return "Attach to ${if (direct) "" else "#"}$channel" to detail
    }

    /**
     * Whether a drag offers files. Android describes a drag only by MIME type until the drop,
     * so plain text, HTML, links and intents (text being dragged) don't count; a dropped
     * `.txt` file is therefore attached only through the attach button.
     */
    fun carriesFiles(mimeTypes: Collection<String>): Boolean = mimeTypes.any { it.lowercase() !in TEXT_DRAGS }

    private val TEXT_DRAGS = setOf("text/plain", "text/html", "text/uri-list", "text/vnd.android.intent")

    /** Resting pose per card, left to right: offset in dp and turn in degrees. */
    val poses = listOf(Triple(-46f, 10f, -16f), Triple(0f, -26f, 0f), Triple(46f, 10f, 16f))

    /** Decelerates without overshooting, like the shared `--ease-out` entrances. */
    fun easeOut(t: Float): Float {
        val x = t.coerceIn(0f, 1f)
        return if (x >= 1f) 1f else 1f - 2f.pow(-10f * x)
    }

    fun easeInOut(t: Float): Float {
        val x = t.coerceIn(0f, 1f)
        return x * x * (3f - 2f * x)
    }

    fun progress(elapsed: Float, delay: Float, duration: Float): Float = ((elapsed - delay) / duration).coerceIn(0f, 1f)

    /** 0 to 1 and back over 2.4 s each way, after the cards have fanned out. */
    fun sway(elapsed: Float, delay: Float): Float {
        if (elapsed < delay) return 0f
        val phase = ((elapsed - delay) / 2.4f) % 2f
        return easeInOut(if (phase < 1f) phase else 2f - phase)
    }

    /** The character's hop over 1.6 s: lift in dp, and horizontal/vertical squash. */
    fun hop(elapsed: Float): Triple<Float, Float, Float> {
        if (elapsed < 0.5f) return Triple(0f, 1f, 1f)
        val t = ((elapsed - 0.5f) / 1.6f) % 1f
        val frames = listOf(
            floatArrayOf(0f, 0f, 1f, 1f), floatArrayOf(0.25f, -9f, 0.97f, 1.03f), floatArrayOf(0.52f, 0f, 1.05f, 0.95f),
            floatArrayOf(0.6f, 0f, 1f, 1f), floatArrayOf(1f, 0f, 1f, 1f),
        )
        val next = frames.indexOfFirst { it[0] > t }.let { if (it < 1) frames.size - 1 else it }
        val from = frames[next - 1]
        val to = frames[next]
        val k = easeInOut((t - from[0]) / (to[0] - from[0]))
        fun lerp(i: Int) = from[i] + (to[i] - from[i]) * k
        return Triple(lerp(1), lerp(2), lerp(3))
    }
}
