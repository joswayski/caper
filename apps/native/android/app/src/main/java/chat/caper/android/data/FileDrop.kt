package chat.caper.android.data

import kotlin.math.PI
import kotlin.math.exp
import kotlin.math.max
import kotlin.math.pow
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * The overlay over a conversation while files are dragged onto it. Every client
 * produces the text in `shared/messages/drop-cases.json`, and the motion matches web
 * (`apps/web/src/chat/drop.ts`), Apple and Rust desktop (`file_drop.rs`). Change them together.
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
    val poses = listOf(Triple(-46.0, 10.0, -16.0), Triple(0.0, -26.0, 0.0), Triple(46.0, 10.0, 16.0))
    private const val GRAVITY = 420.0
    private const val CROUCH = 0.12

    /** Decelerates without overshooting, like the shared `--ease-out` entrances. */
    fun easeOut(t: Float): Float {
        val x = t.coerceIn(0f, 1f)
        return if (x >= 1f) 1f else 1f - 2f.pow(-10f * x)
    }

    fun progress(elapsed: Float, delay: Float, duration: Float): Float = ((elapsed - delay) / duration).coerceIn(0f, 1f)

    /*
     * The motion: each open picks a random seed, so the cards drift and the character hops
     * differently every time; every client derives the same motion from a seed
     * (shared/messages/drop-motion-cases.json). Times are seconds; lengths are dp.
     */

    /** mulberry32: small, and identical on every client's integer arithmetic. */
    class Random(seed: Int) {
        private var state = seed
        fun next(): Double {
            state += 0x6D2B79F5
            var t = (state xor (state ushr 15)) * (1 or state)
            t = (t + ((t xor (t ushr 7)) * (61 or t))) xor t
            return (t xor (t ushr 14)).toUInt().toDouble() / 4294967296.0
        }
    }

    /** [jitter] offsets the resting pose; [waves] holds two (period, phase) pairs each for x, y and turn. */
    class Card(val jitter: DoubleArray, val delay: Double, val duration: Double, val waves: DoubleArray)
    class Hop(val start: Double, val height: Double)
    class Plan(val cards: List<Card>, val tilt: Pair<Double, Double>, val hops: List<Hop>)

    /** Offset from the stage's bottom centre, turn in degrees, and opacity. */
    data class CardPose(val x: Double, val y: Double, val turn: Double, val opacity: Double)

    /** How far below its resting place, squash, turn and opacity. */
    data class CharacterPose(val y: Double, val scaleX: Double, val scaleY: Double, val turn: Double, val opacity: Double)
    data class Pose(val cards: List<CardPose>, val character: CharacterPose)

    fun plan(seed: Int): Plan {
        val random = Random(seed)
        fun spread(size: Double) = (random.next() * 2 - 1) * size
        val cards = poses.map {
            val jitter = doubleArrayOf(spread(3.0), spread(3.0), spread(3.0))
            val delay = 0.04 + random.next() * 0.14
            val duration = 1.3 + random.next() * 0.6
            val waves = DoubleArray(12)
            for (i in 0 until 6) {
                waves[2 * i] = 4 + random.next() * 5
                waves[2 * i + 1] = random.next() * 2 * PI
            }
            Card(jitter, delay, duration, waves)
        }
        val period = 5 + random.next() * 3
        val tilt = period to random.next() * 2 * PI
        val hops = mutableListOf<Hop>()
        var start = 0.75 + random.next() * 0.5
        repeat(32) {
            val height = 5 + random.next() * 7
            val quick = random.next() < 0.25
            val pause = random.next()
            hops += Hop(start, height)
            start += CROUCH + air(height) + air(height * 0.2) + if (quick) 0.06 + pause * 0.1 else 0.7 + pause * 1.8
        }
        return Plan(cards, tilt, hops)
    }

    /** Seconds in the air for a hop [height] high. */
    private fun air(height: Double) = 2 * sqrt(2 * height / GRAVITY)

    /** A damped spring's response [elapsed] after a kick, peaking near [size]. */
    private fun spring(size: Double, elapsed: Double) = if (elapsed <= 0) 0.0 else size * exp(-6 * elapsed) * sin(16 * elapsed) / 0.6

    private fun progress(elapsed: Double, delay: Double, duration: Double) = ((elapsed - delay) / duration).coerceIn(0.0, 1.0)

    private fun easeOutCubic(t: Double) = 1 - (1 - t).pow(3)

    /** Where everything is [elapsed] seconds in; [still] is the reduced-motion resting pose. */
    fun pose(plan: Plan, elapsed: Double, still: Boolean = false): Pose {
        val t = if (still) 0.0 else elapsed
        fun wave(waves: DoubleArray, at: Int) =
            0.6 * sin(2 * PI * t / waves[at] + waves[at + 1]) + 0.4 * sin(2 * PI * t / waves[at + 2] + waves[at + 3])
        val cards = plan.cards.mapIndexed { index, card ->
            val (x, y, turn) = poses[index]
            val out = easeOutCubic(if (still) 1.0 else progress(t, card.delay, card.duration))
            val drift = if (still) 0.0 else out
            CardPose(
                (x + card.jitter[0]) * out + 2.5 * wave(card.waves, 0) * drift,
                // Each card slides out and up from just below the middle.
                24 + (y + card.jitter[1] - 24) * out + 3.5 * wave(card.waves, 4) * drift,
                turn + card.jitter[2] + 2.5 * wave(card.waves, 8) * drift,
                if (still) 1.0 else progress(t, card.delay, 0.3),
            )
        }
        val rise = if (still) 1.0 else easeOutCubic(progress(t, 0.08, 0.5))
        var lift = 0.0
        var squash = 0.0
        if (!still) {
            for (hop in plan.hops) {
                val since = t - hop.start
                if (since < 0) break
                val up = air(hop.height)
                val rebound = air(hop.height * 0.2)
                val flying = since - CROUCH
                if (flying >= 0 && flying < up) {
                    lift = GRAVITY / 2 * flying * (up - flying)
                } else if (flying >= up && flying < up + rebound) {
                    lift = GRAVITY / 2 * (flying - up) * (up + rebound - flying)
                }
                // Crouch, spring up, land, and settle after the rebound; higher hops squash more.
                val size = hop.height / 9
                squash += spring(-0.05 * size, since) + spring(0.09 * size, flying) +
                    spring(-0.1 * size, flying - up) + spring(-0.035 * size, flying - up - rebound)
            }
        }
        val turn = if (still) 0.0 else 2 * sin(2 * PI * t / plan.tilt.first + plan.tilt.second) * rise
        return Pose(
            cards,
            CharacterPose(28 * (1 - rise) - lift, 1 - 0.8 * squash, 1 + squash, turn, if (still) 1.0 else progress(t, 0.08, 0.2)),
        )
    }
}
