package chat.caper.android.data

import chat.caper.android.model.Account
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatHistory
import chat.caper.android.model.ChatMessage
import chat.caper.android.model.DirectConversationList
import chat.caper.android.model.FeedbackReports
import chat.caper.android.model.MessageMention
import chat.caper.android.model.ThreadHistory
import chat.caper.android.model.VisitUi
import java.time.Instant
import java.time.ZoneOffset
import java.util.Locale
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class FeedbackTest {
    private val json = Json { ignoreUnknownKeys = true }
    private val page = json.decodeFromString<FeedbackReports>("""{"reports":[
        {"id":"message00000002","messageId":"message00000002","threadRootId":"message00000001","createdAt":"2026-10-11T19:00:00.123456+00:00",
         "text":"@feedback uploads fail","attachments":0,"reporter":{"id":"alice0000001","username":"alice","displayName":"Alice","avatarId":12},
         "channel":{"id":"channel00001","name":"general","direct":false},"space":{"id":"space0000001","name":"Studio"},"replyCount":3,"answered":true},
        {"id":"message00000003","messageId":"message00000003","threadRootId":"message00000003","createdAt":"2026-10-11T18:00:00Z",
         "text":"","attachments":2,"reporter":{"id":"bob000000001","username":"bob","displayName":"Bob","avatarId":null},
         "channel":{"id":"direct000001","name":"Alice & Bob","direct":true},"space":null,"replyCount":0,"answered":false,"future":1}
        ],"cursor":"123"}""")

    @Test fun `the Caper DM's peer is marked team, and the empty note says where messages go`() {
        val list = json.decodeFromString<DirectConversationList>("""{"conversations":[
            {"id":"direct000001","peer":{"id":"caper0000001","username":"caper","displayName":"Caper","avatarId":1,"team":true},"lastSeq":"0","readSeq":"0","status":"accepted"},
            {"id":"direct000002","peer":{"id":"sam000000001","username":"sam","displayName":"Sam"},"lastSeq":"0","readSeq":"0"}]}""").conversations
        assertEquals(listOf(true, false), list.map { it.peer.team })
        assertEquals("Messages here go to the Caper team.", CAPER_DM_EMPTY_NOTE)
    }

    @Test fun `the account says whether it is on the feedback team`() {
        assertTrue(json.decodeFromString<Account>("""{"id":"account00001","username":"jose","displayName":"Jose","feedbackTeam":true}""").feedbackTeam)
        assertFalse(json.decodeFromString<Account>("""{"id":"account00001","username":"jose","displayName":"Jose"}""").feedbackTeam)
    }

    @Test fun `admin authors, visitor windows and feedback mentions decode`() {
        val message = { author: String, mentions: String -> json.decodeFromString<ChatMessage>(
            """{"id":"message00000001","channelId":"channel00001","seq":"1","author":$author,"content":{"version":1,"type":"text","text":"@caper hi","mentions":$mentions},"createdAt":"2026-10-11T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000001"}""",
        ) }
        val admin = message("""{"id":"account00001","name":"Jose","isGuest":false,"admin":true}""", """[{"type":"feedback"}]""")
        assertTrue(admin.author.admin)
        assertEquals(listOf(MessageMention("feedback")), admin.content.mentions)
        assertTrue(notifiesCaperTeam(admin.content))
        val plain = message("""{"id":"account00002","name":"Jose","isGuest":false}""", "[]")
        assertFalse(plain.author.admin)
        assertFalse(notifiesCaperTeam(plain.content))
        assertFalse(notifiesCaperTeam(ChatContent(1, "text", "@feedback", mentions = listOf(MessageMention("everyone")))))

        val history = json.decodeFromString<ChatHistory>("""{"messages":[],"pinnedMessages":[],"cursor":"4","hasMore":false,"hasNewer":false,
            "space":{"id":"","name":"Direct messages"},"channel":{"id":"direct000001","name":"Alice & Bob","direct":true},"visitor":true}""")
        assertTrue(history.visitor)
        assertTrue(history.channel!!.direct)
        assertFalse(json.decodeFromString<ChatHistory>("""{"messages":[],"cursor":"4","hasMore":false}""").visitor)
        val root = """{"id":"message00000001","channelId":"channel00001","seq":"1","author":{"id":"account00001","name":"A","isGuest":false},"content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-10-11T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000001"}"""
        assertTrue(json.decodeFromString<ThreadHistory>("""{"root":$root,"messages":[],"cursor":"4","hasMore":false,"visitor":true}""").visitor)
        assertFalse(json.decodeFromString<ThreadHistory>("""{"root":$root,"messages":[],"cursor":"4","hasMore":false}""").visitor)
    }

    @Test fun `reports decode and describe where, what and their status`() {
        val (channel, direct) = page.reports
        assertEquals("123", page.cursor)
        assertEquals("Alice", channel.reporter.name)
        assertEquals(12, channel.reporter.avatarId)
        assertEquals("#general · Studio", feedbackLocation(channel))
        assertEquals("DM · Alice & Bob", feedbackLocation(direct))
        assertEquals("@feedback uploads fail", feedbackPreview(channel))
        assertEquals("Sent 2 files", feedbackPreview(direct))
        assertEquals("Sent 1 file", feedbackPreview(direct.copy(attachments = 1)))
        assertEquals("", feedbackPreview(direct.copy(attachments = 0)))
        assertEquals("Answered", feedbackStatus(channel))
        assertEquals("Waiting", feedbackStatus(direct))
        assertNull(json.decodeFromString<FeedbackReports>("""{"reports":[],"cursor":null}""").cursor)
        assertEquals("Someone", direct.reporter.copy(username = null, displayName = null).name)
    }

    @Test fun `a report opens its conversation at the message, in its thread`() {
        val (channel, direct) = page.reports
        val reply = requireNotNull(feedbackTarget(channel))
        assertEquals(FeedbackTarget("channel00001", "space0000001", "message00000002", "message00000001", "general", "Studio", channel.reporter), reply)
        assertEquals("message00000001", reply.thread)
        assertEquals("message00000001", reply.replyRoot)
        val topLevel = requireNotNull(feedbackTarget(direct))
        assertNull(topLevel.spaceId)
        assertEquals("message00000003", topLevel.thread)
        // A top-level report is its own thread: its context is the conversation.
        assertNull(topLevel.replyRoot)
        assertNull(feedbackTarget(channel.copy(messageId = "../x")))
        assertNull(feedbackTarget(channel.copy(channel = channel.channel.copy(id = "bad"))))
    }

    @Test fun `only feedback notifications open a visit`() {
        assertEquals(FeedbackTarget("channel00001", "space0000001", "message00000002", "message00000001"),
            feedbackTargetFromNotification("feedback", null, "space0000001", "channel00001", "message00000002", "message00000001"))
        val dm = requireNotNull(feedbackTargetFromNotification("feedback", "direct000001", null, null, "message00000003", null))
        assertEquals("direct000001", dm.channelId)
        assertNull(dm.spaceId)
        assertEquals("message00000003", dm.thread)
        assertNull(feedbackTargetFromNotification("mention.user", "direct000001", null, null, "message00000003", null))
        assertNull(feedbackTargetFromNotification(null, "direct000001", null, null, "message00000003", null))
        assertNull(feedbackTargetFromNotification("feedback", "direct000001", "space0000001", null, "message00000003", null))
        assertNull(feedbackTargetFromNotification("feedback", null, "space0000001", null, "message00000003", null))
        assertNull(feedbackTargetFromNotification("feedback", "direct000001", null, null, null, null))
        assertNull(feedbackTargetFromNotification("feedback", "direct000001", null, null, "message00000003", "../root"))
    }

    @Test fun `visits are titled like the conversation`() {
        assertEquals("#plans · Studio", visitTitle(VisitUi("channel00001", "space0000001", "Studio", "plans", direct = false)))
        assertEquals("Alice & Bob", visitTitle(VisitUi("direct000001", null, null, "Alice & Bob", direct = true)))
        assertEquals("#plans", visitTitle(VisitUi("channel00001", "space0000001", null, "plans", direct = false)))
        assertEquals("Caper team has been notified 🫡", CAPER_TEAM_NOTIFIED)
        assertEquals("ADMIN", ADMIN_BADGE)
    }

    @Test fun `report times are compact and accept offsets`() {
        val now = Instant.parse("2026-10-11T20:00:00Z")
        fun age(value: String) = feedbackAge(value, now, ZoneOffset.UTC, Locale.US)
        assertEquals("now", age("2026-10-11T19:59:30Z"))
        assertEquals("now", age("2026-10-11T20:05:00Z"))
        assertEquals("5m", age("2026-10-11T19:55:00+00:00"))
        assertEquals("59m", age("2026-10-11T19:00:00.123456+00:00"))
        assertEquals("1h", age("2026-10-11T18:59:00.123456+00:00"))
        assertEquals("3d", age("2026-10-08T19:00:00Z"))
        assertEquals("Sep 30", age("2026-09-30T10:00:00Z"))
        assertEquals("Oct 4, 2025", age("2025-10-04T10:00:00Z"))
        assertEquals("", age("yesterday"))
    }

    @Test fun `archive moves whole threads and says whose feedback it is`() {
        val (channel, direct) = page.reports
        assertFalse(channel.archived)
        assertTrue(json.decodeFromString<FeedbackReports>("""{"reports":[{"id":"message00000004","messageId":"message00000004","threadRootId":"message00000004",
            "createdAt":"2026-10-11T18:00:00Z","reporter":{"id":"bob000000001"},"channel":{"id":"direct000001","name":"Bob"},"archived":true}]}""").reports.single().archived)
        // Another report in the same thread goes with it; the same root in another conversation stays.
        val sameThread = channel.copy(id = "message00000005", messageId = "message00000005")
        val elsewhere = channel.copy(id = "message00000006", channel = channel.channel.copy(id = "channel00002"))
        assertEquals(listOf(direct, elsewhere), withoutThread(listOf(channel, direct, sameThread, elsewhere), channel))
        assertEquals("Archive feedback from Alice", archiveActionLabel(channel, archived = false))
        assertEquals("Restore feedback from Alice to the inbox", archiveActionLabel(channel, archived = true))
    }

    @Test fun `pages append without duplicates and cursors are decimal`() {
        val loaded = page.reports.take(1)
        assertEquals(page.reports, mergeFeedbackPage(loaded, page, first = false))
        assertEquals(page.reports, mergeFeedbackPage(emptyList(), page, first = true))
        assertEquals(page.reports, mergeFeedbackPage(listOf(page.reports[1]), page, first = true))
        assertEquals("123", validFeedbackCursor("123"))
        assertNull(validFeedbackCursor(null))
        assertNull(validFeedbackCursor("0"))
        assertNull(validFeedbackCursor("12a"))
        assertNull(validFeedbackCursor("-1"))
    }
}
