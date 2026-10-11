package chat.caper.android.data

import java.time.Instant
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Request

internal const val WHATS_NEW_MAX_BYTES = 64 * 1024
private const val MAX_SEQUENCE = 9007199254740991L
internal fun isReleaseRevision(value: String) = Regex("^[0-9a-f]{40}$").matches(value)

@Serializable data class ReleaseHighlight(val title: String, val body: String)
@Serializable data class PlatformRelease(
    val sequence: Long, val revision: String, val publishedAt: String, val highlights: List<ReleaseHighlight>,
)
@Serializable data class WhatsNewSnapshot(
    val schema: Int, val platform: String, val revision: String, val sequence: Long, val releases: List<PlatformRelease>,
) {
    fun validated(expectedPlatform: String, expectedRevision: String): WhatsNewSnapshot = apply {
        require(schema == 1 && platform == expectedPlatform && platform in setOf("android", "ios", "macos"))
        require(isReleaseRevision(revision) && revision == expectedRevision && sequence in 1..MAX_SEQUENCE)
        require(releases.size in 1..30)
        var previousSequence = 0L
        var previousDate: Instant? = null
        val revisions = mutableSetOf<String>()
        releases.forEach { release ->
            require(release.sequence in 1..MAX_SEQUENCE && release.sequence > previousSequence)
            require(isReleaseRevision(release.revision) && revisions.add(release.revision))
            require(release.publishedAt.length <= 40 && release.publishedAt.endsWith("Z"))
            val date = Instant.parse(release.publishedAt)
            require(previousDate == null || date >= previousDate)
            require(release.highlights.size <= 5)
            release.highlights.forEach {
                require(validReleaseText(it.title, 100) && validReleaseText(it.body, 500))
            }
            previousSequence = release.sequence
            previousDate = date
        }
        require(releases.last().sequence == sequence && releases.last().revision == revision)
    }
}

private fun validReleaseText(value: String, limit: Int) =
    value.isNotBlank() && value.codePointCount(0, value.length) <= limit &&
        value.none { it.isISOControl() && it != '\n' && it != '\t' }

/** The acknowledgement belongs to an installation/origin/platform, not an account. */
class WhatsNewHistory(private val read: () -> Long?, private val write: (Long) -> Unit) {
    var snapshot: WhatsNewSnapshot? = null
        private set
    val unread: List<PlatformRelease> get() {
        val ack = read() ?: return emptyList()
        return snapshot?.releases.orEmpty().filter { it.sequence > ack && it.highlights.isNotEmpty() }.asReversed()
    }
    val retained: List<PlatformRelease> get() = snapshot?.releases.orEmpty().filter { it.highlights.isNotEmpty() }.asReversed()

    fun receive(value: WhatsNewSnapshot, platform: String, revision: String) {
        value.validated(platform, revision)
        snapshot = value
        if (read() == null || unread.isEmpty()) acknowledge(value.sequence)
    }

    /** A displayed snapshot stays fixed while a newer fetch or rollback completes. */
    fun acknowledge(sequence: Long) {
        require(sequence in 1..MAX_SEQUENCE)
        write(maxOf(read() ?: 0L, sequence))
    }
}

/** Public, bounded and deliberately unauthenticated, unlike account requests. */
suspend fun CaperApi.whatsNew(revision: String): WhatsNewSnapshot {
    require(isReleaseRevision(revision))
    val request = Request.Builder().url("$baseUrl/api/whats-new?platform=android&revision=$revision").build()
    return client.newCall(request).awaitDecoded { response ->
        if (!response.isSuccessful) throw ApiException(response.code, "Release notes are not available yet.")
        val body = response.body
        require(body.contentLength() <= WHATS_NEW_MAX_BYTES)
        val source = body.source()
        source.request(WHATS_NEW_MAX_BYTES.toLong() + 1)
        require(source.buffer.size <= WHATS_NEW_MAX_BYTES)
        val root = json().parseToJsonElement(source.readUtf8()).jsonObject
        // Kotlin's numeric serializers also accept quoted numbers; the wire
        // contract requires JSON numbers, as do the other clients.
        require(root.getValue("schema").jsonPrimitive.isString.not() && root.getValue("sequence").jsonPrimitive.isString.not())
        require(root.getValue("releases").jsonArray.all { it.jsonObject.getValue("sequence").jsonPrimitive.isString.not() })
        json().decodeFromJsonElement<WhatsNewSnapshot>(root).validated("android", revision)
    }
}
