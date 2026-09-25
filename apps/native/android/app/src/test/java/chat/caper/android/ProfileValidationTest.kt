package chat.caper.android

import org.junit.Assert.*
import org.junit.Test

class ProfileValidationTest {
    @Test fun `profile accepts Unicode scalars but rejects controls and invalid usernames`() {
        assertTrue(profileValid("ab_", "🪴".repeat(64)))
        assertFalse(profileValid("ab_", "🪴".repeat(65)))
        assertFalse(profileValid("ab", "Jose"))
        assertFalse(profileValid("a".repeat(33), "Jose"))
        assertFalse(profileValid("a-b", "Jose"))
        assertFalse(profileValid("ab_", " \t"))
        assertFalse(profileValid("ab_", "Jo\u0000se"))
        assertFalse(profileValid("ab_", "Jo\u0085se"))
    }
}
