package chat.caper.android.data

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.*
import org.junit.Test

class ProfileSaveTest {
    private val server = MockWebServer()
    private val account = """{"id":"account00001","username":"jose","displayName":"Jose","avatarId":17}"""
    @After fun close() { server.close() }

    @Test fun `names alone omit avatar while changed pictures include explicit nullable baseline`() = runBlocking {
        val api = CaperApi(baseUrl = server.url("/").toString())
        server.enqueue(MockResponse().setBody(account))
        api.profile("secret", " jose ", " Jose ")
        val names = server.takeRequest()
        assertEquals("/api/account/profile", names.path)
        assertEquals("POST", names.method)
        assertEquals("""{"username":"jose","displayName":"Jose"}""", names.body.readUtf8())
        for (avatar in listOf(
            ProfileAvatar(799, null, null, null),
            ProfileAvatar(17, "previous", 17, "old"),
            ProfileAvatar(17, null, 17, "old"),
        )) {
            server.enqueue(MockResponse().setBody(account))
            api.profile("secret", "jose", "Joseph", avatar)
            val request = server.takeRequest()
            assertEquals("/api/account/profile", request.path)
            val body = Json.parseToJsonElement(request.body.readUtf8()).jsonObject
            val picture = body.getValue("avatar").jsonObject
            assertEquals(setOf("avatarId", "avatarAsset", "expectedAvatarId", "expectedAvatarAsset"), picture.keys)
            assertEquals(avatar.avatarId.toString(), picture["avatarId"].toString())
            assertEquals(avatar.avatarAsset?.let { "\"$it\"" } ?: "null", picture["avatarAsset"].toString())
            assertEquals(avatar.expectedAvatarId?.toString() ?: "null", picture["expectedAvatarId"].toString())
            assertEquals(avatar.expectedAvatarAsset?.let { "\"$it\"" } ?: "null", picture["expectedAvatarAsset"].toString())
        }
    }

    @Test fun `draft completion returns reserved id with unchanged account and does not apply it`() = runBlocking {
        val id = "PiCtUrE123456789"
        val api = CaperApi(baseUrl = server.url("/").toString())
        server.enqueue(MockResponse().setResponseCode(201).setBody("""{"id":"$id","kind":"image","upload":{"method":"PUT","url":"${server.url("/storage")}","headers":{"content-type":"image/webp"}}}"""))
        server.enqueue(MockResponse())
        server.enqueue(MockResponse().setResponseCode(409).setBody("""{"error":"not ready"}"""))
        server.enqueue(MockResponse().setBody("""{"id":"$id","account":$account}"""))
        val result = PictureUploader(api, listOf(1)).upload("secret", PictureTarget.Avatar,
            PreparedPicture(byteArrayOf(1, 2, 3), "image/webp", 1, 1), draft = true)
        assertEquals(id, result.id)
        assertNull(result.account!!.avatarAsset)
        assertEquals(17, result.account!!.avatarId)
        assertEquals("/api/assets", server.takeRequest().path)
        val storage = server.takeRequest()
        assertNull(storage.getHeader("Authorization"))
        assertArrayEquals(byteArrayOf(1, 2, 3), storage.body.readByteArray())
        repeat(2) { assertEquals("/api/assets/$id/complete?draft=true", server.takeRequest().path) }
        assertEquals(4, server.requestCount)

        // The only commit is the following profile request, with the reserved ID, not account.avatarAsset.
        server.enqueue(MockResponse().setResponseCode(409).setBody("""{"error":"stale","code":"profile_changed"}"""))
        val failure = runCatching { api.profile("secret", "jose", "Joseph", ProfileAvatar(17, result.id, 17, null)) }.exceptionOrNull()
        assertEquals("profile_changed", (failure as ApiException).code)
        assertEquals(409, failure.status)
        val save = server.takeRequest()
        assertEquals("/api/account/profile", save.path)
        assertEquals("\"$id\"", Json.parseToJsonElement(save.body.readUtf8()).jsonObject.getValue("avatar").jsonObject["avatarAsset"].toString())
    }
}
