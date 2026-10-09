package chat.caper.android.data

/**
 * Plain-text link detection for message text, shared by every client.
 *
 * Messages stay plain text on the wire (`content.text`); links are found at
 * render time, like mentions. The rules follow GitHub Flavored Markdown's
 * autolink literals, with quotes and unbalanced `]` also treated as trailing
 * punctuation, so a later Markdown renderer can reuse this for bare URLs.
 *
 * This is the same algorithm as web's `apps/web/src/chat/links.ts`; the unit
 * tests run every case in `shared/messages/link-cases.json`. Change them together.
 */

/** One run of text; [href] is set for an `http(s)://` or `www.` link and is always http or https. */
internal data class LinkSegment(val text: String, val href: String? = null)

/** A link inside a longer text: characters [start] until [end] open [href]. */
internal data class LinkRange(val start: Int, val end: Int, val href: String)

/** Unicode White_Space; links end at the first of these or `<`. */
private fun Char.isLinkSpace(): Boolean =
    this in '\t'..'\r' || this == ' ' || this == '\u0085' || this == ' ' || this == ' ' ||
        this in ' '..' ' || this == ' ' || this == ' ' || this == ' ' ||
        this == ' ' || this == '　'

/** A link may start at the beginning, after whitespace, or after one of these. */
private const val OPENERS = "([{<\"'*_~"
private const val TRAILING = "?!.,:;*_~\"'>"

private fun Char.isAsciiLetterOrDigit(): Boolean = this in 'a'..'z' || this in 'A'..'Z' || this in '0'..'9'
private fun Char.isHostCharacter(): Boolean = isAsciiLetterOrDigit() || this == '_' || this == '.' || this == '-'
private fun Char.asciiLowercase(): Char = if (this in 'A'..'Z') this + ('a' - 'A') else this

/** ASCII case-insensitively: Kotlin's `ignoreCase` would also match letters such as `ſ`. */
private fun String.startsAtIgnoringAsciiCase(index: Int, prefix: String): Boolean =
    index + prefix.length <= length && prefix.indices.all { this[index + it].asciiLowercase() == prefix[it] }

/** The scheme's length (0 for `www.`), or -1 when no candidate starts at [index]. */
private fun schemeAt(text: String, index: Int): Int = when {
    text.startsAtIgnoringAsciiCase(index, "https://") -> 8
    text.startsAtIgnoringAsciiCase(index, "http://") -> 7
    text.startsAtIgnoringAsciiCase(index, "www.") -> 0
    else -> -1
}

/** Where a trailing `&entity;` starts, or -1. */
private fun trailingEntity(value: String): Int {
    if (!value.endsWith(';')) return -1
    var start = value.length - 1
    while (start > 0 && value[start - 1].isAsciiLetterOrDigit()) start--
    return if (start < value.length - 1 && start > 0 && value[start - 1] == '&') start - 1 else -1
}

/** Drops trailing punctuation, unbalanced closers and a trailing `&entity;`. */
private fun trim(candidate: String): String {
    var value = candidate
    while (value.isNotEmpty()) {
        val last = value.last()
        value = when {
            last in TRAILING -> trailingEntity(value).let { entity -> if (entity >= 0) value.substring(0, entity) else value.dropLast(1) }
            last == ')' && value.count { it == ')' } > value.count { it == '(' } -> value.dropLast(1)
            last == ']' && value.count { it == ']' } > value.count { it == '[' } -> value.dropLast(1)
            else -> return value
        }
    }
    return value
}

/** At least two non-empty labels (three for `www.`), no `_` in the last two. */
private fun validHost(host: String, www: Boolean): Boolean {
    val labels = host.split('.')
    if (labels.size < (if (www) 3 else 2) || labels.any { it.isEmpty() }) return false
    return labels.takeLast(2).none { '_' in it }
}

/** Splits plain text into text runs and `http(s)://` / `www.` links; the runs rebuild [text] exactly. */
internal fun linkSegments(text: String): List<LinkSegment> {
    val segments = mutableListOf<LinkSegment>()
    var plainStart = 0
    var index = 0
    while (index < text.length) {
        val previous = if (index > 0) text[index - 1] else null
        val scheme = if (previous == null || previous.isLinkSpace() || previous in OPENERS) schemeAt(text, index) else -1
        if (scheme < 0) { index++; continue }
        var end = index
        while (end < text.length && !text[end].isLinkSpace() && text[end] != '<') end++
        val candidate = trim(text.substring(index, end))
        var hostEnd = scheme
        while (hostEnd < candidate.length && candidate[hostEnd].isHostCharacter()) hostEnd++
        if (scheme > candidate.length || !validHost(candidate.substring(scheme, hostEnd), scheme == 0)) { index++; continue }
        if (index > plainStart) segments += LinkSegment(text.substring(plainStart, index))
        val href = if (scheme == 0) "https://$candidate"
            else candidate.substring(0, scheme).map { it.asciiLowercase() }.joinToString("") + candidate.substring(scheme)
        segments += LinkSegment(candidate, href)
        index += candidate.length
        plainStart = index
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
