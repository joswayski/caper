package chat.caper.android

import chat.caper.android.data.ShareTarget
import chat.caper.android.data.Sharing
import chat.caper.android.model.ForwardDestination
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SharingTest {
    @Test fun shortcutIdsRoundTripAndRejectAnythingElse() {
        val channel = ShareTarget.of(ForwardDestination("chan00000001", "general", "Team", false, spaceId = "space0000001"))!!
        val direct = ShareTarget.of(ForwardDestination("dm0000000001", "Ana", "Direct messages", true))!!
        assertEquals("channel:space0000001:chan00000001", channel.shortcutId)
        assertEquals(channel, ShareTarget.fromShortcutId(channel.shortcutId))
        assertEquals(direct, ShareTarget.fromShortcutId("dm:dm0000000001"))
        // A channel needs its space to open; IDs must be the API's 12-character form.
        assertNull(ShareTarget.of(ForwardDestination("chan00000001", "general", "Team", false)))
        listOf(null, "", "dm:", "dm:short", "channel:chan00000001", "channel:space0000001:../../x", "other:dm0000000001", "dm:dm0000000001:x")
            .forEach { assertNull(it, ShareTarget.fromShortcutId(it)) }
    }

    @Test fun onlyOtherAppsContentUrisAreAttached() {
        val own = "chat.caper.android"
        assertTrue(Sharing.acceptsStream("content", "com.google.android.apps.photos.contentprovider", own))
        assertFalse(Sharing.acceptsStream("file", null, own))
        assertFalse(Sharing.acceptsStream("content", own, own))
        assertFalse(Sharing.acceptsStream("content", "$own.notification-media", own))
        // A different app whose package merely starts with ours is still another app.
        assertTrue(Sharing.acceptsStream("content", "${own}x.files", own))
    }

    @Test fun sharedTextFollowsTheDraftOnItsOwnLine() {
        assertEquals("https://example.com", Sharing.appendText("", "https://example.com"))
        assertEquals("look\nhttps://example.com", Sharing.appendText("look  ", "https://example.com"))
        assertEquals("look", Sharing.appendText("look", " "))
    }

    @Test fun searchMatchesEveryWordInOrder() {
        val recent = listOf(
            ForwardDestination("a00000000001", "design", "Studio", false, "s00000000001"),
            ForwardDestination("a00000000002", "Ana Lopez", "Direct messages", true),
            ForwardDestination("a00000000003", "general", "Studio", false, "s00000000001"),
        )
        assertEquals(recent, Sharing.matching(recent, "  "))
        assertEquals(listOf(recent[0], recent[2]), Sharing.matching(recent, "studio"))
        assertEquals(listOf(recent[2]), Sharing.matching(recent, "#gen studio"))
        assertEquals(listOf(recent[1]), Sharing.matching(recent, "lopez"))
    }
}
