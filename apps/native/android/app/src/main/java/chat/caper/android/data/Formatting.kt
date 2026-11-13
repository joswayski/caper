package chat.caper.android.data

/** Style bits of formatted text (shared/messages/formatting.rs). */
internal object FormatStyle {
    const val BOLD = 1
    const val ITALIC = 2
    const val UNDERLINE = 4
    const val STRIKETHROUGH = 8
    const val SPOILER = 16
    const val QUOTE = 32
}

internal data class FormattedRun(val text: String, val style: Int) {
    val bold get() = style and FormatStyle.BOLD != 0
    val italic get() = style and FormatStyle.ITALIC != 0
    val underline get() = style and FormatStyle.UNDERLINE != 0
    val strikethrough get() = style and FormatStyle.STRIKETHROUGH != 0
    val spoiler get() = style and FormatStyle.SPOILER != 0
    val quote get() = style and FormatStyle.QUOTE != 0
}
internal data class FormattedMessage(val runs: List<FormattedRun>, val sourceIndices: List<Int>) {
    val text: String get() = runs.joinToString("") { it.text }
    /**
     * Displayed ranges of neighbouring runs with style [bit]: one quote block (its inner newlines are
     * quoted, the ones at its edges plain), or one spoiler that reveals together.
     */
    fun ranges(bit: Int): List<IntRange> = buildList {
        var offset = 0
        var start = -1
        for (run in runs) {
            if (run.style and bit != 0) { if (start < 0) start = offset }
            else if (start >= 0) { add(start until offset); start = -1 }
            offset += run.text.length
        }
        if (start >= 0) add(start until offset)
    }
    fun displayed(offset: Int): Int = sourceIndices.count { it < offset }
    // At the trailing edge stay inside the closing delimiter, so typing extends the style.
    fun original(offset: Int): Int = sourceIndices.getOrNull(offset) ?: ((sourceIndices.lastOrNull() ?: -1) + 1)
}

/**
 * The last few results of a parse, keyed on the exact text. One composer keystroke parses the same draft
 * from its visual transformation, autocomplete, repair and toolbar state; results are read-only.
 */
internal class RecentParses<T : Any>(private val parse: (String) -> T) {
    private val entries = ArrayDeque<Pair<String, T>>()
    operator fun invoke(text: String): T = synchronized(this) {
        entries.firstOrNull { it.first == text }?.second ?: parse(text).also {
            entries.addFirst(text to it)
            if (entries.size > 4) entries.removeLast()
        }
    }
}

private val recentFormats = RecentParses(::parseFormattedMessage)

/** Deliberately small Markdown subset, aligned with web and the shared fixtures (shared/messages/formatting.rs, through Rust). */
internal fun formattedMessage(text: String): FormattedMessage = recentFormats(text)

private fun parseFormattedMessage(text: String): FormattedMessage {
    val spans = CaperMobile.formatSpans(text)
    val runs = mutableListOf<FormattedRun>()
    val indices = mutableListOf<Int>()
    for (span in spans.indices step 3) {
        val start = spans[span]
        val end = spans[span + 1]
        val style = spans[span + 2]
        indices.addAll(start until end)
        val value = text.substring(start, end)
        val last = runs.lastOrNull()
        if (last?.style == style) runs[runs.lastIndex] = last.copy(text = last.text + value)
        else runs += FormattedRun(value, style)
    }
    return FormattedMessage(runs, indices)
}

/**
 * Links in message [text] as web finds them: in the displayed text of each formatted run, outside
 * [excluded] (sorted source ranges, the mentions), so hidden delimiters such as `||` never start, join
 * or end a link. Ranges are in [text]'s source coordinates.
 */
internal fun messageLinkRanges(text: String, excluded: List<IntRange> = emptyList()): List<LinkRange> {
    val formatted = formattedMessage(text)
    val cuts = excluded.map { formatted.displayed(it.first) until formatted.displayed(it.last + 1) }
    val links = mutableListOf<LinkRange>()
    var offset = 0
    for (run in formatted.runs) {
        val end = offset + run.text.length
        val inside = cuts.filter { it.first < end && it.last + 1 > offset }
            .map { maxOf(it.first, offset) - offset until minOf(it.last + 1, end) - offset }
        for (link in linkRanges(run.text, inside)) {
            links += LinkRange(formatted.sourceIndices[offset + link.start], formatted.sourceIndices[offset + link.end - 1] + 1, link.href)
        }
        offset = end
    }
    return links
}

internal data class FormatSelection(val text: String, val start: Int, val end: Int)

/**
 * Native undo restores snapshots, not a new edit to repair against the latest draft. It also keeps the
 * style web carries from one key to the next one typed at the caret it left, in the text it left
 * (shared/messages/composer.rs): an edit made elsewhere, such as an IME composition, ends it.
 */
internal class FormatEditHistory {
    private val snapshots = ArrayDeque<String>()
    private var pending = -1
    private var caret = -1
    private var left: String? = null
    fun remember(text: String) {
        if (text !in snapshots) snapshots.addLast(text)
        while (snapshots.size > 100) snapshots.removeFirst()
    }
    fun edit(previous: String, next: FormatSelection, restoringHistory: Boolean = false): FormatSelection {
        remember(previous)
        val typed = next.text.length - previous.length
        val continuing = if (typed > 0 && next.start == next.end && next.start - typed == caret && previous == left) pending else -1
        val result = when {
            previous == next.text -> next.also { if (it.start != caret || it.end != caret) pending = -1 }
            restoringHistory || next.text in snapshots -> next.also { pending = -1 }
            else -> composerEdit(previous, next, continuing).let { pending = it.pending; it.selection }
        }
        caret = if (result.start == result.end) result.start else -1
        left = result.text
        remember(result.text)
        return result
    }

    /** Ends the carried style: the field shows another draft or conversation, even an identical one. */
    fun endTyping() {
        pending = -1
        left = null
    }
}

internal fun copiedFormatText(text: String, start: Int, end: Int): String {
    val projection = formattedMessage(text)
    return projection.text.substring(projection.displayed(start), projection.displayed(end))
}

internal data class FormatEdit(val selection: FormatSelection, val pending: Int)

/**
 * One composer edit as web makes it (shared/messages/composer.rs, through Rust). [continuing] is the
 * previous edit's [FormatEdit.pending] when this one types at the caret it left, otherwise -1.
 */
internal fun composerEdit(previous: String, next: FormatSelection, continuing: Int): FormatEdit {
    val result = CaperMobile.composerEdit(previous, next.text, next.start, next.end, continuing)
    val text = String(CharArray(result.size - 3) { result[it + 3].toChar() })
    return FormatEdit(FormatSelection(text, result[0], result[1]), result[2])
}

/** An edit without a style carried from the previous key: autocomplete and other replacements. */
internal fun repairFormatEdit(previous: String, next: FormatSelection): FormatSelection =
    if (previous == next.text) next else composerEdit(previous, next, -1).selection

/**
 * The toolbar's [style] toggle (a [FormatStyle] bit) for the source selection, as web's toolbar makes
 * it (shared/messages/composer.rs, through Rust); null when refused, e.g. it would split a link or
 * style text the grammar cannot express.
 */
internal fun toggleFormat(text: String, start: Int, end: Int, style: Int): FormatSelection? {
    val result = CaperMobile.composerToggle(text, start, end, style)
    if (result.isEmpty()) return null
    return FormatSelection(String(CharArray(result.size - 2) { result[it + 2].toChar() }), result[0], result[1])
}

/** The [FormatStyle] bits any visible character of the source selection has: the toolbar's pressed buttons (web). */
internal fun selectionStyles(text: String, start: Int, end: Int): Int = CaperMobile.selectionStyles(text, start, end)
