package chat.caper.android.data

internal data class FormattedRun(val text: String, val bold: Boolean, val italic: Boolean)
internal data class FormattedMessage(val runs: List<FormattedRun>, val sourceIndices: List<Int>) {
    val text: String get() = runs.joinToString("") { it.text }
    fun displayed(offset: Int): Int = sourceIndices.count { it < offset }
    // At the trailing edge stay inside the closing delimiter, so typing extends the style.
    fun original(offset: Int): Int = sourceIndices.getOrNull(offset) ?: ((sourceIndices.lastOrNull() ?: -1) + 1)
}

private fun formatSpace(c: Char) = c in "\t\n\u000b\u000c\r \u00a0\u1680\u2028\u2029\u202f\u205f\u3000\ufeff" || c in '\u2000'..'\u200a'

/** Deliberately small Markdown subset, aligned with web and the shared fixtures (shared/messages/formatting.rs, through Rust). */
internal fun formattedMessage(text: String): FormattedMessage {
    val spans = CaperMobile.formatSpans(text)
    val runs = mutableListOf<FormattedRun>()
    val indices = mutableListOf<Int>()
    for (span in spans.indices step 3) {
        val start = spans[span]
        val end = spans[span + 1]
        val bold = spans[span + 2] and 1 != 0
        val italic = spans[span + 2] and 2 != 0
        indices.addAll(start until end)
        val value = text.substring(start, end)
        val last = runs.lastOrNull()
        if (last?.bold == bold && last.italic == italic) runs[runs.lastIndex] = last.copy(text = last.text + value)
        else runs += FormattedRun(value, bold, italic)
    }
    return FormattedMessage(runs, indices)
}

internal data class FormatSelection(val text: String, val start: Int, val end: Int)

/** Native undo restores snapshots, not a new edit to repair against the latest draft. */
internal class FormatEditHistory {
    private val snapshots = ArrayDeque<String>()
    fun remember(text: String) {
        if (text !in snapshots) snapshots.addLast(text)
        while (snapshots.size > 100) snapshots.removeFirst()
    }
    fun edit(previous: String, next: FormatSelection, restoringHistory: Boolean = false): FormatSelection {
        remember(previous)
        val result = if (restoringHistory || next.text in snapshots) next else repairFormatEdit(previous, next)
        remember(result.text)
        return result
    }
}

internal fun copiedFormatText(text: String, start: Int, end: Int): String {
    val projection = formattedMessage(text)
    return projection.text.substring(projection.displayed(start), projection.displayed(end))
}

/** Remove formerly hidden delimiters that an edit orphaned, never literal user punctuation. */
internal fun repairFormatEdit(previous: String, next: FormatSelection): FormatSelection {
    if (previous == next.text) return next
    val before = formattedMessage(previous).sourceIndices.toSet()
    if (before.size == previous.length) return next
    // At the end of a styled draft the caret sits inside the closing delimiters, so typing extends
    // the style. Whitespace can't end a style, so it goes after them, as web serializes it:
    // "**bold**" + " and" keeps "**bold** and" rather than losing both markers.
    val typed = next.text.length - previous.length
    if (typed > 0 && next.start == next.end) {
        val at = next.start - typed
        if (at > 0 && next.text.startsWith(previous.substring(0, at)) && next.text.endsWith(previous.substring(at)) &&
            next.text.substring(at, next.start).all(::formatSpace) && (at until previous.length).none { it in before }) {
            val moved = previous + next.text.substring(at, next.start)
            if (formattedMessage(moved).sourceIndices.size == before.size + typed) return FormatSelection(moved, moved.length, moved.length)
        }
    }
    var prefix = 0
    while (prefix < minOf(previous.length, next.text.length) && previous[prefix] == next.text[prefix]) prefix++
    var suffix = 0
    while (suffix < minOf(previous.length, next.text.length) - prefix && previous[previous.lastIndex - suffix] == next.text[next.text.lastIndex - suffix]) suffix++
    val visible = formattedMessage(next.text).sourceIndices.toSet()
    val removed = previous.indices.filter { it !in before }.mapNotNull { index ->
        when {
            index < prefix -> index
            index >= previous.length - suffix -> index + next.text.length - previous.length
            else -> null
        }
    }.filter { it in visible }.toSet()
    if (removed.isEmpty()) return next
    fun offset(value: Int) = value - removed.count { it < value }
    return repairFormatEdit(previous, FormatSelection(next.text.filterIndexed { index, _ -> index !in removed }, offset(next.start), offset(next.end)))
}

internal fun toggleFormat(text: String, selectionStart: Int, selectionEnd: Int, marker: String): FormatSelection {
    var start = selectionStart
    var end = selectionEnd
    if (start == end) return FormatSelection(text, start, end)
    var offset = 0
    for (segment in linkSegments(text)) {
        val limit = offset + segment.text.length
        if (segment.href != null && (start > offset && start < limit || end > offset && end < limit)) {
            return FormatSelection(text, start, end)
        }
        offset = limit
    }
    if (start != end) {
        while (start < end && formatSpace(text[start])) start++
        while (end > start && formatSpace(text[end - 1])) end--
        if (start == end) return FormatSelection(text, start, end)
    }
    val width = marker.length
    if (start != end) {
        val visible = formattedMessage(text).sourceIndices.filter { it >= start && it < end }
        val first = visible.firstOrNull()
        val limit = visible.lastOrNull()?.plus(1)
        if (first != null && limit != null && first >= width && text.substring(first - width, first) == marker && text.substring(limit).startsWith(marker) &&
            formattedMessage(text.substring(first - width, limit + width)).runs.all { if (marker == "**") it.bold else it.italic }) {
            start = first; end = limit
        }
    }
    if (start >= width && text.substring(start - width, start) == marker && text.substring(end).startsWith(marker)) {
        return FormatSelection(text.substring(0, start - width) + text.substring(start, end) + text.substring(end + width), start - width, end - width)
    }
    var selected = text.substring(start, end)
    val runs = formattedMessage(selected).runs
    fun hasStyle(run: FormattedRun) = if (marker == "**") run.bold else run.italic
    if (selected.length >= width * 2 && selected.startsWith(marker) && selected.endsWith(marker) && runs.all(::hasStyle)) {
        return FormatSelection(text.substring(0, start) + selected.substring(width, selected.length - width) + text.substring(end), start, end - width * 2)
    }
    if (runs.any(::hasStyle)) {
        val remaining = mutableListOf<Pair<String, Boolean>>()
        for (run in runs) {
            val styled = if (marker == "**") run.italic else run.bold
            val last = remaining.lastOrNull()
            if (last?.second == styled) remaining[remaining.lastIndex] = (last.first + run.text) to styled
            else remaining += run.text to styled
        }
        val other = if (marker == "**") "_" else "**"
        selected = remaining.joinToString("") { (value, styled) -> if (styled) other + value + other else value }
    }
    val result = FormatSelection(text.substring(0, start) + marker + selected + marker + text.substring(end), start + width, start + width + selected.length)
    // Do not promise formatting the wire grammar cannot represent (e.g. italics inside an identifier).
    if (start != end && formattedMessage(result.text).text != formattedMessage(text).text) return FormatSelection(text, selectionStart, selectionEnd)
    return result
}

/** Some of the selected source text is already bold ([bold]) or italic: the toolbar shows that button pressed (web). */
internal fun selectionHasStyle(text: String, start: Int, end: Int, bold: Boolean): Boolean {
    val formatted = formattedMessage(text)
    val from = formatted.displayed(start)
    val to = formatted.displayed(end)
    var offset = 0
    return formatted.runs.any { run ->
        val hit = offset < to && offset + run.text.length > from && if (bold) run.bold else run.italic
        offset += run.text.length
        hit
    }
}
