package chat.caper.android.data

/**
 * Plain-text link detection for message text, shared by every client.
 *
 * Messages stay plain text on the wire (`content.text`); links are found at
 * render time, like mentions. The rules follow GitHub Flavored Markdown's
 * autolink literals, with quotes and unbalanced `]` also treated as trailing
 * punctuation, so a later Markdown renderer can reuse this for bare URLs.
 *
 * The rules run in Rust (`shared/messages/links.rs`, the same algorithm as web's
 * `apps/web/src/chat/links.ts`); the unit tests run every case in
 * `shared/messages/link-cases.json`. Change them together.
 */

/** One run of text; [href] is set for an `http(s)://` or `www.` link and is always http or https. */
internal data class LinkSegment(val text: String, val href: String? = null)

/** A link inside a longer text: characters [start] until [end] open [href]. */
internal data class LinkRange(val start: Int, val end: Int, val href: String)

/** Splits plain text into text runs and `http(s)://` / `www.` links; the runs rebuild [text] exactly. */
internal fun linkSegments(text: String): List<LinkSegment> {
    val spans = CaperMobile.linkSpans(text)
    val segments = mutableListOf<LinkSegment>()
    var plainStart = 0
    for (span in spans.indices step 3) {
        val start = spans[span]
        val end = spans[span + 1]
        if (start > plainStart) segments += LinkSegment(text.substring(plainStart, start))
        val link = text.substring(start, end)
        // The scheme's length: the link opens it lowercased, and `www.` opens as https.
        val href = when (spans[span + 2]) {
            8 -> "https://" + link.substring(8)
            7 -> "http://" + link.substring(7)
            else -> "https://$link"
        }
        segments += LinkSegment(link, href)
        plainStart = end
    }
    if (plainStart < text.length) segments += LinkSegment(text.substring(plainStart))
    return segments
}

/**
 * Links in [text] outside [excluded] (mentions: sorted, non-overlapping `start until end` ranges).
 * Each run between them is scanned on its own, as its own text, so a link never swallows a mention.
 */
internal fun linkRanges(text: String, excluded: List<IntRange> = emptyList()): List<LinkRange> {
    val links = mutableListOf<LinkRange>()
    var runStart = 0
    fun scan(runEnd: Int) {
        if (runEnd <= runStart) return
        var offset = runStart
        linkSegments(text.substring(runStart, runEnd)).forEach { segment ->
            segment.href?.let { links += LinkRange(offset, offset + segment.text.length, it) }
            offset += segment.text.length
        }
    }
    excluded.forEach { range ->
        scan(range.first.coerceIn(runStart, text.length))
        runStart = maxOf(runStart, (range.last + 1).coerceAtMost(text.length))
    }
    scan(text.length)
    return links
}
