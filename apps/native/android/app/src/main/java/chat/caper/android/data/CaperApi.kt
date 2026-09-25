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
import kotlinx.serialization.json.put
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
    suspend fun general(): ChatHistory = validatedHistory(get("/api/chat/general"))
    suspend fun history(token: String?, channel: String, before: String? = null): ChatHistory {
        val history: ChatHistory = get(
            "/api/chat/channels/${channel.pathId()}/messages" + (before?.let { "?before=$it" } ?: ""), token,
        )
        return validatedHistory(history, channel)
    }
    private fun validatedHistory(history: ChatHistory, expectedChannel: String? = history.channel?.id): ChatHistory {
        require(Regex("^(0|[1-9][0-9]*)$").matches(history.cursor) && history.cursor.toLongOrNull() != null) { "Invalid history cursor." }
        val channel = requireNotNull(expectedChannel) { "History channel is missing." }
        history.messages.forEach { it.validated(channel) }
        return history
    }
    suspend fun chatSession(token: String?, name: String): ChatSession = post(
        "/api/chat/session", buildJsonObject { put("name", name) }, token,
    )
    suspend fun sendMessage(
        token: String?,
        chatToken: String,
        channel: String,
        author: ChatAuthor,
        clientMessageId: UUID,
        text: String,
    ): ChatMessage {
        val message: ChatMessage = post(
            "/api/chat/channels/${channel.pathId()}/messages",
            buildJsonObject { put("clientMessageId", clientMessageId.toString()); put("text", text) },
            token, mapOf("x-caper-chat-token" to chatToken),
        )
        return message.validated(channel, author, clientMessageId, text)
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
    suspend fun spaceMembers(token: String, space: String): MemberList = get("/api/spaces/${space.pathId()}/members", token)
    suspend fun addSpaceMember(token: String, space: String, username: String): Member = post(
        "/api/spaces/${space.pathId()}/members", buildJsonObject { put("username", username.trim()) }, token,
    )
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
