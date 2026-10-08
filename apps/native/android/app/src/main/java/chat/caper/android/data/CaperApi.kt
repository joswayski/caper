package chat.caper.android.data

import chat.caper.android.BuildConfig
import chat.caper.android.model.*
import java.io.IOException
import java.util.UUID
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.add
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject
import okhttp3.Call
import okhttp3.Callback
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response

class ApiException(val status: Int, override val message: String, val code: String? = null, val attemptsRemaining: Int? = null) : IOException(message)

class CaperApi(
    val client: OkHttpClient = OkHttpClient.Builder()
        .callTimeout(java.time.Duration.ofSeconds(30))
        .followRedirects(false)
        .followSslRedirects(false)
        .build(),
    baseUrl: String = BuildConfig.API_BASE_URL,
    @PublishedApi internal val json: Json = Json { ignoreUnknownKeys = true },
) {
    val baseUrl = canonicalApiOrigin(baseUrl)
    suspend fun requestCode(email: String): Challenge = post("/api/auth/email/request", buildJsonObject { put("email", email.trim()) })
    suspend fun verifyCode(challenge: String, code: String): VerifyResult = post(
        "/api/auth/email/verify",
        buildJsonObject { put("challengeId", challenge); put("code", code.trim()); put("tokenTransport", "bearer") },
    )
    suspend fun me(token: String): Account = get("/api/account/me", token)
    suspend fun profile(token: String, username: String, displayName: String): Account = post(
        "/api/account/profile",
        buildJsonObject { put("username", username.trim()); put("displayName", displayName.trim()) }, token,
    )
    suspend fun logout(token: String) { request<Unit>("/api/auth/logout", "POST", token = token) }
    suspend fun spaces(token: String): SpaceList = get("/api/spaces", token)
    suspend fun space(token: String, id: String): SpaceDetail = get("/api/spaces/${id.pathId()}", token)
    suspend fun directConversations(token: String): DirectConversationList = get("/api/dms", token)
    suspend fun people(token: String): PeopleList = get("/api/people", token)
    suspend fun startDirectConversation(token: String, username: String): DirectConversation = post(
        "/api/dms", buildJsonObject { put("username", username.trim()) }, token,
    )
    suspend fun acceptDirectRequest(token: String, id: String): DirectConversation = post("/api/dms/${id.pathId()}/accept", token = token)
    suspend fun declineDirectRequest(token: String, id: String) { request<Unit>("/api/dms/${id.pathId()}/decline", "POST", token) }
    suspend fun blocks(token: String): BlockList = get("/api/blocks", token)
    suspend fun block(token: String, account: String) { request<Unit>("/api/blocks/${account.pathId()}", "PUT", token) }
    suspend fun unblock(token: String, account: String) { request<Unit>("/api/blocks/${account.pathId()}", "DELETE", token) }
    suspend fun directPrivacy(token: String): DirectPrivacy = get("/api/account/privacy", token)
    suspend fun setDirectPrivacy(token: String, value: String): DirectPrivacy = request(
        "/api/account/privacy", "PUT", token, buildJsonObject { put("directMessages", value) }.toString(),
    )
    suspend fun markDirectConversationRead(token: String, id: String, seq: String) {
        require(Regex("^(0|[1-9][0-9]*)$").matches(seq)) { "Invalid read sequence." }
        request<Unit>("/api/dms/${id.pathId()}/read", "POST", token, buildJsonObject { put("seq", seq) }.toString())
    }
    suspend fun pushConfig(token: String): PushConfig = get("/api/push/config", token)
    suspend fun registerPush(token: String, deviceToken: String) { request<Unit>("/api/push/devices", "POST", token, buildJsonObject { put("platform", "fcm"); put("token", deviceToken) }.toString()) }
    suspend fun unregisterPush(token: String, deviceToken: String) { request<Unit>("/api/push/devices", "DELETE", token, buildJsonObject { put("platform", "fcm"); put("token", deviceToken) }.toString()) }
    suspend fun general(): ChatHistory = validatedHistory(get("/api/chat/general"))
    suspend fun history(token: String?, channel: String, before: String? = null): ChatHistory {
        val history: ChatHistory = get(
            "/api/chat/channels/${channel.pathId()}/messages" + (before?.let { "?before=$it" } ?: ""), token,
        )
        return validatedHistory(history, channel)
    }
    suspend fun thread(token: String?, channel: String, root: String, before: String? = null): ThreadHistory {
        require(messageId.matches(root)) { "Invalid message ID." }
        val page: ThreadHistory = get("/api/chat/channels/${channel.pathId()}/messages/$root/thread" + (before?.let { "?before=$it" } ?: ""), token)
        page.root.validated(channel)
        require(page.root.id == root && page.root.threadRootId == null) { "Invalid thread parent." }
        page.messages.forEach { it.validated(channel); require(it.threadRootId == root) { "Invalid thread reply." } }
        return page
    }
    private fun validatedHistory(history: ChatHistory, expectedChannel: String? = history.channel?.id): ChatHistory {
        require(Regex("^(0|[1-9][0-9]*)$").matches(history.cursor) && history.cursor.toLongOrNull() != null) { "Invalid history cursor." }
        val channel = requireNotNull(expectedChannel) { "History channel is missing." }
        history.messages.forEach { it.validated(channel) }
        history.pinnedMessages.forEach { it.validated(channel) }
        return history
    }
    suspend fun chatSession(token: String?, name: String): ChatSession = post(
        "/api/chat/session", buildJsonObject { put("name", name) }, token,
    )
    suspend fun forwardDestinations(token: String): ForwardDestinations = get("/api/chat/forward-destinations", token)

    suspend fun forward(token: String, chatToken: String, source: ChatMessage, destination: String, key: UUID, text: String): ChatMessage {
        val message: ChatMessage = post(
            "/api/chat/channels/${destination.pathId()}/forwards",
            buildJsonObject { put("sourceChannelId", source.channelId); put("sourceMessageId", source.id); put("clientMessageId", key.toString()); put("text", text) },
            token, mapOf("x-caper-chat-token" to chatToken),
        )
        require(message.forward != null) { "Invalid forward." }
        return message.validated(destination, expectedClientMessageId = key, expectedText = text)
    }

    suspend fun forwardedConversation(token: String, source: ChatMessage, before: String? = null): ForwardConversation {
        require(messageId.matches(source.id)) { "Invalid message ID." }
        before?.let { require(Regex("^(0|[1-9][0-9]*)$").matches(it)) { "Invalid cursor." } }
        val conversation: ForwardConversation = get("/api/chat/channels/${source.channelId.pathId()}/forwards/${source.id}/thread" + (before?.let { "?before=$it" } ?: ""), token)
        require(Regex("^(0|[1-9][0-9]*)$").matches(conversation.cursor)) { "Invalid source cursor." }
        conversation.root?.let { it.validated(it.channelId) }
        conversation.messages.forEach { it.validated(it.channelId) }
        return conversation
    }

    suspend fun sendMessage(
        token: String?,
        chatToken: String,
        channel: String,
        author: ChatAuthor,
        clientMessageId: UUID,
        text: String,
        attachmentIds: List<String> = emptyList(),
        threadRootId: String? = null,
        broadcast: Boolean = false,
    ): ChatMessage {
        require(attachmentIds.size <= AttachmentPolicy.MAX_ATTACHMENTS) { "Attach up to 10 files." }
        val message: ChatMessage = post(
            "/api/chat/channels/${channel.pathId()}/messages",
            buildJsonObject {
                put("clientMessageId", clientMessageId.toString()); put("text", text)
                // Only when non-empty: text-only requests keep their original idempotency hash.
                if (attachmentIds.isNotEmpty()) putJsonArray("attachmentIds") { attachmentIds.forEach { add(it.assetPathId()) } }
                if (threadRootId != null) { put("threadRootId", threadRootId); put("broadcast", broadcast) }
            },
            token, mapOf("x-caper-chat-token" to chatToken),
        )
        require(message.threadRootId == threadRootId && message.broadcast == broadcast) { "Reply destination mismatch." }
        return message.validated(channel, author, clientMessageId, text)
    }

    /** Succeeds only when uploads are configured; anything else hides the attach control. */
    suspend fun assetUsage(token: String): AssetUsage = get("/api/assets/usage", token)

    /**
     * Reserves one upload of the bytes this device will store (after compression): their exact
     * size, the picked file's size, measured dimensions/duration and an optional preview.
     */
    suspend fun createAsset(
        token: String, channel: String, filename: String, contentType: String, byteSize: Long,
        sourceByteSize: Long? = null, width: Int? = null, height: Int? = null, durationMs: Long? = null,
        previewContentType: String? = null, previewByteSize: Long? = null,
    ): AssetReservation {
        val reservation: AssetReservation = post("/api/assets", buildJsonObject {
            put("channelId", channel.pathId()); put("filename", filename); put("contentType", contentType); put("byteSize", byteSize)
            sourceByteSize?.let { put("sourceByteSize", it) }
            width?.let { put("width", it) }; height?.let { put("height", it) }; durationMs?.let { put("durationMs", it) }
            if (previewContentType != null && previewByteSize != null) putJsonObject("preview") {
                put("contentType", previewContentType); put("byteSize", previewByteSize)
            }
        }, token)
        reservation.id.assetPathId()
        require(reservation.upload.method == "PUT" && reservation.upload.url.isStorageUrl()) { "The upload service returned an invalid response." }
        reservation.previewUpload?.let { require(it.method == "PUT" && it.url.isStorageUrl()) { "The upload service returned an invalid response." } }
        return reservation
    }

    /** `409` means storage has not seen the upload yet; [AttachmentUploader] retries a few times. */
    suspend fun completeAsset(token: String, id: String): ChatAttachment =
        post("/api/assets/${id.assetPathId()}/complete", token = token)

    /** Re-signed delivery URLs for attachments this account can still see (at most 100 per call). */
    suspend fun attachmentUrls(token: String, ids: List<String>): Map<String, AttachmentUrls> {
        require(ids.size in 1..100) { "Request 1 to 100 files." }
        val response: AttachmentUrlsResponse = post(
            "/api/assets/urls", buildJsonObject { putJsonArray("ids") { ids.forEach { add(it.assetPathId()) } } }, token,
        )
        return response.urls.filter { (id, urls) ->
            id in ids && (urls.url != null || urls.previewUrl != null) &&
                urls.url?.isStorageUrl() != false && urls.previewUrl?.isStorageUrl() != false
        }
    }

    /**
     * PUT bytes straight to storage with exactly the presigned headers and a fixed
     * Content-Length of [size]. No account or chat credential is attached, and redirects are
     * never followed. The body streams from [open] without holding the file in memory.
     */
    suspend fun putUpload(upload: PresignedUpload, size: Long, open: () -> java.io.InputStream, progress: (Long) -> Unit = {}) {
        val request = Request.Builder().url(upload.url).apply {
            upload.headers.forEach { (name, value) -> header(name, value) }
            put(StreamingUploadBody(size, open, progress))
        }.build()
        try {
            storageClient.newCall(request).awaitDecoded { response ->
                if (!response.isSuccessful) throw UploadException("Storage refused the upload (${response.code}).")
            }
        } catch (error: UploadException) { throw error } catch (error: IOException) {
            throw UploadException("The upload was interrupted.")
        }
    }

    private val storageClient: OkHttpClient by lazy {
        client.newBuilder().callTimeout(java.time.Duration.ZERO).readTimeout(java.time.Duration.ofSeconds(60))
            .writeTimeout(java.time.Duration.ofSeconds(60)).build()
    }

    suspend fun editMessage(token: String?, chatToken: String, channel: String, message: String, text: String, expectedRevision: Int): ChatMessage {
        require(messageId.matches(message) && expectedRevision > 0 && validEditText(text)) { "Invalid message edit." }
        val result: ChatMessage = request(
            "/api/chat/channels/${channel.pathId()}/messages/$message", "PUT", token,
            buildJsonObject { put("text", text); put("expectedRevision", expectedRevision) }.toString(),
            mapOf("x-caper-chat-token" to chatToken),
        )
        require(result.id == message) { "Message identity mismatch." }
        return result.validated(channel)
    }

    suspend fun loadMessage(token: String?, channel: String, message: String): ChatMessage {
        require(messageId.matches(message)) { "Invalid message ID." }
        val result: ChatMessage = get("/api/chat/channels/${channel.pathId()}/messages/$message", token)
        require(result.id == message) { "Message identity mismatch." }
        return result.validated(channel)
    }

    suspend fun messageVersions(token: String?, channel: String, message: String, before: Int? = null): MessageVersions {
        require(messageId.matches(message) && (before == null || before > 0)) { "Invalid message history request." }
        val page: MessageVersions = get("/api/chat/channels/${channel.pathId()}/messages/$message/versions" + (before?.let { "?before=$it" } ?: ""), token)
        return page.validated(message, before)
    }

    suspend fun setReaction(token: String?, chatToken: String, channel: String, message: String, emoji: String, active: Boolean): ReactionUpdate {
        require(messageId.matches(message)) { "Invalid message ID." }
        val update: ReactionUpdate = request(
            "/api/chat/channels/${channel.pathId()}/messages/$message/reactions", "PUT", token,
            buildJsonObject { put("emoji", emoji); put("active", active) }.toString(),
            mapOf("x-caper-chat-token" to chatToken),
        )
        return update.validated(channel, message)
    }

    /** Who reacted, with the same read access and credentials as [history]. */
    suspend fun reactors(token: String?, channel: String, message: String): ReactorList {
        require(messageId.matches(message)) { "Invalid message ID." }
        val list: ReactorList = get("/api/chat/channels/${channel.pathId()}/messages/$message/reactions", token)
        return list.validated(message)
    }

    suspend fun setPin(token: String?, chatToken: String, channel: String, message: String, active: Boolean): PinUpdate {
        require(messageId.matches(message)) { "Invalid message ID." }
        val update: PinUpdate = request(
            "/api/chat/channels/${channel.pathId()}/messages/$message/pin", "PUT", token,
            buildJsonObject { put("active", active) }.toString(), mapOf("x-caper-chat-token" to chatToken),
        )
        return update.validated(channel, message)
    }

    suspend fun createSpace(token: String, name: String): Space = post(
        "/api/spaces", buildJsonObject { put("name", name.trim()) }, token,
    )
    suspend fun updateSpace(token: String, id: String, name: String): Space = request(
        "/api/spaces/${id.pathId()}", "PATCH", token, buildJsonObject { put("name", name.trim()) }.toString(),
    )
    suspend fun deleteSpace(token: String, id: String) { request<Unit>("/api/spaces/${id.pathId()}", "DELETE", token) }
    suspend fun createChannel(token: String, space: String, name: String, privateChannel: Boolean): Channel = post(
        "/api/spaces/${space.pathId()}/channels",
        buildJsonObject { put("name", name); put("private", privateChannel) }, token,
    )
    suspend fun updateChannel(token: String, space: String, channel: String, name: String, privateChannel: Boolean): Channel = request(
        "/api/spaces/${space.pathId()}/channels/${channel.pathId()}", "PATCH", token,
        buildJsonObject { put("name", name); put("private", privateChannel) }.toString(),
    )
    suspend fun deleteChannel(token: String, space: String, channel: String) {
        request<Unit>("/api/spaces/${space.pathId()}/channels/${channel.pathId()}", "DELETE", token)
    }
    suspend fun joinChannel(token: String, space: String, channel: String): Channel =
        post("/api/spaces/${space.pathId()}/channels/${channel.pathId()}/membership", token = token)
    suspend fun leaveChannel(token: String, space: String, channel: String) {
        request<Unit>("/api/spaces/${space.pathId()}/channels/${channel.pathId()}/membership", "DELETE", token)
    }
    suspend fun acceptChannelInvitation(token: String, space: String, channel: String): Channel =
        post("/api/spaces/${space.pathId()}/channels/${channel.pathId()}/invitation", token = token)
    suspend fun declineChannelInvitation(token: String, space: String, channel: String) {
        request<Unit>("/api/spaces/${space.pathId()}/channels/${channel.pathId()}/invitation", "DELETE", token)
    }
    suspend fun spaceMembers(token: String, space: String): MemberList = get("/api/spaces/${space.pathId()}/members", token)
    suspend fun addSpaceMember(token: String, space: String, username: String): Member = post(
        "/api/spaces/${space.pathId()}/members", buildJsonObject { put("username", username.trim()) }, token,
    )
    suspend fun spaceInvitations(token: String, space: String): MemberList =
        get("/api/spaces/${space.pathId()}/invitations", token)
    suspend fun cancelSpaceInvitation(token: String, space: String, user: String) {
        request<Unit>("/api/spaces/${space.pathId()}/invitations/${user.pathId()}", "DELETE", token)
    }
    suspend fun acceptSpaceInvitation(token: String, space: String): Space =
        post("/api/spaces/${space.pathId()}/invitation", token = token)
    suspend fun declineSpaceInvitation(token: String, space: String) {
        request<Unit>("/api/spaces/${space.pathId()}/invitation", "DELETE", token)
    }
    suspend fun removeSpaceMember(token: String, space: String, member: String) {
        request<Unit>("/api/spaces/${space.pathId()}/members/${member.pathId()}", "DELETE", token)
    }
    suspend fun channelMembers(token: String, space: String, channel: String): MemberList =
        get("/api/spaces/${space.pathId()}/channels/${channel.pathId()}/members", token)
    suspend fun addChannelMember(token: String, space: String, channel: String, username: String): Member = post(
        "/api/spaces/${space.pathId()}/channels/${channel.pathId()}/members",
        buildJsonObject { put("username", username.trim()) }, token,
    )
    suspend fun removeChannelMember(token: String, space: String, channel: String, member: String) {
        request<Unit>("/api/spaces/${space.pathId()}/channels/${channel.pathId()}/members/${member.pathId()}", "DELETE", token)
    }

    /** Web: GET `${mediaRoot}/status` — the channel's media root, or the demo root for General. */
    suspend fun mediaStatus(accountToken: String?, channel: String, demo: Boolean): MediaStatus =
        get(if (demo) "/api/media/status" else "/api/channels/${channel.pathId()}/media/status", accountToken.takeUnless { demo })

    suspend inline fun <reified T> media(
        accountToken: String?,
        channel: String,
        operation: String,
        body: kotlinx.serialization.json.JsonObject = buildJsonObject {},
        mediaToken: String? = null,
        demo: Boolean = false,
    ): T = post(
        if (demo) "/api/media/$operation" else "/api/channels/${channel.pathId()}/media/$operation", body, accountToken,
        mediaToken?.let { mapOf("x-caper-media-token" to it) } ?: emptyMap(),
    )

    suspend inline fun <reified T> get(path: String, token: String? = null): T = request(path, "GET", token)
    suspend inline fun <reified T> post(
        path: String,
        body: kotlinx.serialization.json.JsonObject? = null,
        token: String? = null,
        headers: Map<String, String> = emptyMap(),
    ): T = request(path, "POST", token, body?.toString(), headers)

    suspend inline fun <reified T> request(
        path: String,
        method: String,
        token: String? = null,
        body: String? = null,
        headers: Map<String, String> = emptyMap(),
    ): T {
        val request = Request.Builder().url(baseUrl + path).apply {
            token?.let { header("Authorization", "Bearer $it") }
            headers.forEach { (name, value) -> header(name, value) }
            val requestBody = body?.toRequestBody("application/json".toMediaType())
                ?: if (method in setOf("POST", "PUT", "PATCH")) "{}".toRequestBody("application/json".toMediaType()) else null
            method(method, requestBody)
        }.build()
        return client.newCall(request).awaitDecoded { response ->
            val text = response.body.string()
            if (!response.isSuccessful) {
                val detail = runCatching { json.decodeFromString<ErrorBody>(text) }.getOrNull()
                throw ApiException(response.code, detail?.error ?: "Request failed (${response.code}).", detail?.code, detail?.attemptsRemaining)
            }
            if (T::class == Unit::class || response.code == 204 || text.isBlank()) Unit as T
            else json.decodeFromString(text)
        }
    }

    fun json() = json
}

private val externalId = Regex("^[A-Za-z0-9]{12}$")
private val assetId = Regex("^[A-Za-z0-9]{16}$")
fun String.assetPathId(): String = also { require(assetId.matches(it)) { "Invalid file ID." } }
private fun String.isStorageUrl() = startsWith("https://") || startsWith("http://")

/**
 * Streams exactly [size] bytes with upload progress. No content type: the presigned header is
 * set verbatim. A source that turns out shorter or longer than declared fails the upload
 * instead of sending bytes that do not match the signed length.
 */
private class StreamingUploadBody(
    private val size: Long, private val open: () -> java.io.InputStream, private val progress: (Long) -> Unit,
) : okhttp3.RequestBody() {
    override fun contentType(): okhttp3.MediaType? = null
    override fun contentLength(): Long = size
    override fun writeTo(sink: okio.BufferedSink) {
        open().use { input ->
            val buffer = ByteArray(64 * 1024)
            var sent = 0L
            while (sent < size) {
                val read = input.read(buffer, 0, minOf(buffer.size.toLong(), size - sent).toInt())
                if (read < 0) throw UploadException("The file changed while uploading.")
                sink.write(buffer, 0, read)
                sent += read
                progress(sent)
            }
            if (input.read() >= 0) throw UploadException("The file changed while uploading.")
        }
    }
}
private val messageId = Regex("^[A-Za-z0-9]{15}$")
fun String.pathId(): String = also { require(externalId.matches(it)) { "Invalid resource ID." } }

// OkHttp invokes onResponse on its IO dispatcher. Keep body reads and JSON
// decoding there: a slow body must not block the service's Main owner context.
// Cancellation remains registered until the entire body is consumed, so it
// cancels the socket even after headers have been delivered.
@PublishedApi internal suspend fun <T> Call.awaitDecoded(decode: (Response) -> T): T = suspendCancellableCoroutine { continuation ->
    continuation.invokeOnCancellation { cancel() }
    enqueue(object : Callback {
        override fun onFailure(call: Call, error: IOException) {
            if (continuation.isActive) continuation.resumeWithException(error)
        }
        override fun onResponse(call: Call, response: Response) {
            response.use {
                try {
                    val value = decode(it)
                    if (continuation.isActive) continuation.resume(value)
                } catch (error: Throwable) {
                    if (continuation.isActive) continuation.resumeWithException(error)
                }
            }
        }
    })
}
