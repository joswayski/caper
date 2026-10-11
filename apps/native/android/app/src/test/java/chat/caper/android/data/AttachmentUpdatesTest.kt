package chat.caper.android.data

import chat.caper.android.model.*
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.*
import org.junit.Test

class AttachmentUpdatesTest {
    private val id = "AbCdEfGh12345678"
    private fun file(status: String?, url: String? = null) = ChatAttachment(id, "image", "image/avif", "a.avif", 10, status = status, url = url)
    private val processing = file("processing")
    private val ready = file("ready", "https://cdn.example/original/a?exp=1&sig=s")
    private val message = ChatMessage(
        "message00000001", "channel00001", "3", ChatAuthor("author", "A", false),
        ChatContent(1, "text", "", listOf(processing)), "now", "client",
    )
    private fun update(seq: String, vararg files: ChatAttachment, messageId: String = message.id) =
        AttachmentsUpdate("message.attachments", 1, "channel00001", seq, messageId, files.toList())

    @Test fun `newer snapshots replace files and stale or duplicate ones do not`() {
        val done = mergeAttachments(message, update("8", ready))
        assertEquals(listOf(ready), done.content.attachments)
        assertEquals("8", done.attachmentsSeq)
        assertSame("a replayed older processing event cannot regress ready", done, mergeAttachments(done, update("6", processing)))
        assertSame(done, mergeAttachments(done, update("8", processing)))
        assertSame("other messages are untouched", message, mergeAttachments(message, update("9", ready, messageId = "message00000002")))
        assertSame("seq must exceed attachmentsSeq ?? 0", message, mergeAttachments(message, update("0", ready)))
        // Reactions stay as they were.
        val reacted = message.copy(reactions = listOf(MessageReaction("👍", listOf("a"))), reactionSeq = "4")
        assertEquals(reacted.reactions, mergeAttachments(reacted, update("5", ready)).reactions)
    }

    @Test fun `fetched messages take files only when their attachmentsSeq is newer`() {
        val applied = mergeAttachments(message, update("8", ready))
        val staleFetch = message.copy(content = message.content.copy(attachments = listOf(processing)))
        assertEquals(listOf(ready), mergeMessages(listOf(applied), listOf(staleFetch), mutableMapOf()).single().content.attachments)
        val olderSeq = staleFetch.copy(attachmentsSeq = "7")
        assertEquals(listOf(ready), mergeMessages(listOf(applied), listOf(olderSeq), mutableMapOf()).single().content.attachments)
        val failed = file("failed")
        val newerFetch = message.copy(content = message.content.copy(attachments = listOf(failed)), attachmentsSeq = "9")
        val merged = mergeMessages(listOf(applied), listOf(newerFetch), mutableMapOf()).single()
        assertEquals(listOf(failed), merged.content.attachments)
        assertEquals("9", merged.attachmentsSeq)
    }

    @Test fun `reactions and files merge independently`() {
        val reactedOld = message.copy(reactions = listOf(MessageReaction("👍", listOf("new"))), reactionSeq = "9")
        val filesNew = message.copy(content = message.content.copy(attachments = listOf(ready)), attachmentsSeq = "10", reactionSeq = "5")
        val merged = mergeMessages(listOf(reactedOld), listOf(filesNew), mutableMapOf()).single()
        assertEquals("9", merged.reactionSeq)
        assertEquals(listOf("new"), merged.reactions.single().authorIds)
        assertEquals(listOf(ready), merged.content.attachments)
        assertEquals("10", merged.attachmentsSeq)
    }

    @Test fun `edits, thread summaries and files merge independently`() {
        val applied = mergeAttachments(message, update("8", ready))
        // A later edit snapshot carries the older "processing" file: the text changes, the file stays ready.
        val edited = message.copy(content = message.content.copy(text = "caption"), revision = 2, editedAt = "2026-10-07T00:00:00Z", editSeq = "9")
        val merged = mergeMessages(listOf(applied), listOf(edited), mutableMapOf()).single()
        assertEquals("caption", merged.content.text)
        assertEquals(2, merged.revision)
        assertEquals(listOf(ready), merged.content.attachments)
        assertEquals("8", merged.attachmentsSeq)
        assertEquals(merged, mergeEdit(applied, edited))
        // And an older edit never overwrites a newer file snapshot carried by the edit side.
        val newerFiles = edited.copy(content = edited.content.copy(attachments = listOf(file("failed"))), attachmentsSeq = "10")
        assertEquals(listOf(file("failed")), mergeEdit(applied, newerFiles).content.attachments)
        // Thread summaries survive alongside the newer files.
        val summarized = message.copy(thread = ThreadSummary(1, emptyList(), "7"))
        val withThread = mergeMessages(listOf(summarized), listOf(applied), mutableMapOf()).single()
        assertEquals(listOf(ready), withThread.content.attachments)
        assertEquals("7", withThread.thread?.seq)
    }

    @Test fun `unloaded updates apply when their message arrives and the cache is bounded`() {
        val unseen = mutableMapOf<String, AttachmentsUpdate>()
        assertTrue(cacheUnseenAttachments(unseen, update("9", ready)))
        assertTrue(cacheUnseenAttachments(unseen, update("7", processing)))
        assertEquals("9", unseen.getValue(message.id).seq)
        val merged = mergeMessages(emptyList(), listOf(message), mutableMapOf(), unseen).single()
        assertEquals(listOf(ready), merged.content.attachments)
        assertTrue(unseen.isEmpty())
        val full = mutableMapOf("x" to update("1", ready, messageId = "x"))
        assertFalse(cacheUnseenAttachments(full, update("2", ready), limit = 1))
        assertTrue("existing entries still update", cacheUnseenAttachments(full, update("3", ready, messageId = "x"), limit = 1))
    }

    @Test fun `history recovery never regresses a newer applied attachments snapshot`() {
        val applied = mergeAttachments(message, update("8", ready))
        val refreshed = ChatHistory(listOf(message), cursor = "8", hasMore = false)
        assertEquals(listOf(ready), recoverHistory(listOf(applied), false, "8", refreshed).messages.single().content.attachments)
        val newer = message.copy(content = message.content.copy(attachments = listOf(file("failed"))), attachmentsSeq = "9")
        assertEquals("failed", recoverHistory(listOf(applied), false, "9", ChatHistory(listOf(newer), "9", false)).messages.single().content.attachments.single().status)
    }

    @Test fun `event validation and tolerant parsing`() {
        fun event(seq: String = "2", channel: String = "channel", type: String = "message.attachments", schema: Int = 1) = Json.parseToJsonElement(
            """{"type":"$type","schemaVersion":$schema,"channelId":"$channel","seq":"$seq","messageId":"message","attachments":[
                {"id":"$id","kind":"image","contentType":"image/avif","name":"a.avif","size":1,"status":"ready","animated":false,"newField":{"x":1},"url":"https://cdn.example/a?exp=1&sig=s"},
                {"id":"bad","kind":"hologram","contentType":"x","name":"x","size":1},
                "junk"],"futureTopLevel":true}""",
        ).jsonObject
        assertEquals("2", attachmentsSequence(event(), "channel"))
        assertThrows(IllegalArgumentException::class.java) { attachmentsSequence(event(channel = "other"), "channel") }
        assertThrows(IllegalArgumentException::class.java) { attachmentsSequence(event("02"), "channel") }
        assertThrows(IllegalArgumentException::class.java) { attachmentsSequence(event(schema = 2), "channel") }
        val parsed = Json { ignoreUnknownKeys = true }.decodeFromJsonElement(AttachmentsUpdate.serializer(), event())
        assertEquals("malformed entries are dropped, not the event", listOf(id), parsed.attachments.map { it.id })
    }

    @Test fun `progress events are tolerant and clamped`() {
        fun progress(body: String) = attachmentProgress(Json.parseToJsonElement(body).jsonObject, "channel")
        assertEquals(AttachmentProgress("m", id, 42), progress("""{"type":"attachment.progress","channelId":"channel","messageId":"m","attachmentId":"$id","percent":42}"""))
        assertEquals(100, progress("""{"channelId":"channel","messageId":"m","attachmentId":"$id","percent":140.5}""")!!.percent)
        assertEquals(0, progress("""{"channelId":"channel","messageId":"m","attachmentId":"$id","percent":-3}""")!!.percent)
        assertNull(progress("""{"channelId":"other","messageId":"m","attachmentId":"$id","percent":4}"""))
        assertNull(progress("""{"channelId":"channel","messageId":"m","percent":4}"""))
        assertNull(progress("""{"channelId":"channel","messageId":"m","attachmentId":"$id","percent":{"x":1}}"""))
        assertNull(progress("""{"channelId":"channel","messageId":"m","attachmentId":"$id","percent":"soon"}"""))
    }

    @Test fun `gateway sequences attachments like reactions and passes progress without advancing`() {
        val server = MockWebServer()
        val incoming = ArrayBlockingQueue<String>(4)
        val opened = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) { opened.add(webSocket); webSocket.send("""{"type":"hello","serverTime":1}""") }
            override fun onMessage(webSocket: WebSocket, text: String) { incoming.add(text) }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, reason) }
        }))
        val deliveries = ArrayBlockingQueue<String>(8)
        val gateway = GatewayClient(server.url("/").toString().trimEnd('/'), null, "channel", "0",
            onMessage = { deliveries.add("message:${it.seq}") },
            onAttachments = { deliveries.add("attachments:${it.seq}:${it.attachments.single().status}") },
            onAttachmentProgress = { deliveries.add("progress:${it.attachmentId}:${it.percent}") },
            onAccessDenied = {}, onResync = {})
        var socket: WebSocket? = null
        try {
            gateway.start()
            socket = opened.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("socket did not open")
            val sub = Json.parseToJsonElement(incoming.poll(2, TimeUnit.SECONDS)!!).jsonObject.getValue("id").jsonPrimitive.content
            socket.send("""{"type":"subscribed","id":"$sub"}""")
            socket.send("""{"type":"event","id":"$sub","event":{"type":"message.created","seq":"1","message":{"id":"message00000001","channelId":"channel","seq":"1","author":{"id":"author","name":"A","isGuest":false},"content":{"version":1,"type":"text","text":"","attachments":[{"id":"$id","kind":"image","contentType":"image/heic","name":"a.heic","size":1,"status":"processing"}]},"createdAt":"2026-01-01T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000001"}}}""")
            socket.send("""{"type":"event","id":"$sub","event":{"type":"attachment.progress","channelId":"channel","messageId":"message00000001","attachmentId":"$id","percent":30}}""")
            socket.send("""{"type":"event","id":"$sub","event":{"type":"attachment.progress","channelId":"channel","messageId":"message00000001","attachmentId":"$id","percent":"garbage"}}""")
            socket.send("""{"type":"event","id":"$sub","event":{"type":"message.attachments","schemaVersion":1,"channelId":"channel","seq":"2","messageId":"message00000001","attachments":[{"id":"$id","kind":"image","contentType":"image/avif","name":"a.avif","size":1,"status":"ready","url":"https://cdn.example/a?exp=1&sig=s"}]}}""")
            socket.send("""{"type":"event","id":"$sub","event":{"type":"message.created","seq":"3","message":{"id":"message00000003","channelId":"channel","seq":"3","author":{"id":"author","name":"A","isGuest":false},"content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-01-01T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000003"}}}""")
            assertEquals("message:1", deliveries.poll(2, TimeUnit.SECONDS))
            assertEquals("progress:$id:30", deliveries.poll(2, TimeUnit.SECONDS))
            assertEquals("attachments:2:ready", deliveries.poll(2, TimeUnit.SECONDS))
            assertEquals("message:3", deliveries.poll(2, TimeUnit.SECONDS))
            assertNull(deliveries.poll(250, TimeUnit.MILLISECONDS))
            assertEquals(GatewayStatus.LIVE, gateway.status.value)
        } finally {
            socket?.close(1000, "done")
            gateway.close()
            server.close()
        }
    }
}
