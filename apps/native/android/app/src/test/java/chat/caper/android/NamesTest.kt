package chat.caper.android

import chat.caper.android.model.hasHiddenNameCharacters
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The same cases as apps/api/src/names.rs. */
class NamesTest {
    @Test fun `names keep visible text emoji and joined scripts`() {
        listOf(
            "Café Society", "José 🌱", "مرحبا بالعالم", "👨‍👩‍👧‍👦 family", "👩🏽‍💻", "❤️‍🔥", "🏳️‍🌈", "🏳️‍⚧️", "1️⃣", "☺︎ text style",
            "🏴󠁧󠁢󠁥󠁮󠁧󠁿 England",
            "می‌خواهم", "क्‍ष", "葛󠄀城", "Screenshot at 10.00.00 AM",
        ).forEach { assertFalse(it, hasHiddenNameCharacters(it)) }
    }

    @Test fun `names reject invisible and text-direction characters`() {
        listOf(
            "a\u0000b", "admin‮nimda", "‪x‬", "⁦x⁩", "a​b", "‎left", "‏right",
            "؜x", "a⁠b", "﻿bom", "soft­hyphen", "a͏b", "ᅟ", "ㅤ", "ﾠ", "⠀",
            "a᠎b", "hidden󠁁󠁂", "️", "a ️", "a️️", "‍a", "a‍",
            "a‍‍b", "a ‍b", "a‍ b", "🏴󠁧󠁢", "🏴󠁿", "x󠁧󠁿",
        ).forEach { assertTrue(it, hasHiddenNameCharacters(it)) }
    }
}
