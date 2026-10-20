package chat.caper.android.data

internal data class FormattedRun(val text: String, val bold: Boolean, val italic: Boolean)
internal data class FormattedMessage(val runs: List<FormattedRun>, val sourceIndices: List<Int>) {
    val text: String get() = runs.joinToString("") { it.text }
    fun displayed(offset: Int): Int = sourceIndices.count { it < offset }
}

private fun formatSpace(c: Char) = c in "\t\n\u000b\u000c\r \u00a0\u1680\u2028\u2029\u202f\u205f\u3000\ufeff" || c in '\u2000'..'\u200a'

/** Deliberately small Markdown subset, aligned with web and the shared fixtures. */
internal fun formattedMessage(text: String): FormattedMessage {
    val links = mutableListOf<IntRange>()
    var offset = 0
    linkSegments(text).forEach {
        if (it.href != null) links += offset until offset + it.text.length
        offset += it.text.length
    }
    val runs = mutableListOf<FormattedRun>()
    val indices = mutableListOf<Int>()
    fun word(c: Char?) = c != null && (c in 'A'..'Z' || c in 'a'..'z' || c in '0'..'9' || c == '_' || c == '@')
    fun markerAt(index: Int, marker: String): Boolean {
        if (!text.startsWith(marker, index) || links.any { index in it }) return false
        var slashes = 0
        var i = index - 1
        while (i >= 0 && text[i] == '\\') { slashes++; i-- }
        if (slashes % 2 != 0) return false
        return marker != "**" || (text.getOrNull(index - 1) != '*' && text.getOrNull(index + 2) != '*')
    }
    fun append(start: Int, end: Int, bold: Boolean, italic: Boolean) {
        if (start >= end) return
        indices.addAll(start until end)
        val value = text.substring(start, end)
        val last = runs.lastOrNull()
        if (last?.bold == bold && last.italic == italic) runs[runs.lastIndex] = last.copy(text = last.text + value)
        else runs += FormattedRun(value, bold, italic)
    }
    fun parse(start: Int, end: Int, bold: Boolean, italic: Boolean) {
        var plain = start
        var index = start
        while (index < end) {
            val marker = when {
                !bold && markerAt(index, "**") -> "**"
                !italic && markerAt(index, "_") -> "_"
                else -> null
            }
            if (marker != null && !(marker == "_" && word(text.getOrNull(index - 1)))) {
                val inner = index + marker.length
                if (inner < end && !formatSpace(text[inner])) {
                    var close = inner
                    while (close + marker.length <= end) {
                        if (close != inner && markerAt(close, marker) && !formatSpace(text[close - 1]) &&
                            !(marker == "_" && word(text.getOrNull(close + 1)))) {
                            append(plain, index, bold, italic)
                            parse(inner, close, bold || marker == "**", italic || marker == "_")
                            index = close + marker.length - 1
                            plain = index + 1
                            break
                        }
                        close++
                    }
                }
            }
            index++
        }
        append(plain, end, bold, italic)
    }
    parse(0, text.length, false, false)
    return FormattedMessage(runs, indices)
}

internal data class FormatSelection(val text: String, val start: Int, val end: Int)
internal fun toggleFormat(text: String, selectionStart: Int, selectionEnd: Int, marker: String): FormatSelection {
    var start = selectionStart
    var end = selectionEnd
    if (start != end) {
        while (start < end && formatSpace(text[start])) start++
        while (end > start && formatSpace(text[end - 1])) end--
        if (start == end) return FormatSelection(text, start, end)
    }
    val width = marker.length
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
    return FormatSelection(text.substring(0, start) + marker + selected + marker + text.substring(end), start + width, start + width + selected.length)
}
