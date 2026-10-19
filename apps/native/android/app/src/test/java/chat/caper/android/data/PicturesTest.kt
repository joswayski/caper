package chat.caper.android.data

import java.io.IOException
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.*
import org.junit.Test

class PicturesTest {
    private val server = MockWebServer()

    @After fun close() { server.close() }

    private fun gif(version: String, width: Int, height: Int): ByteArray =
        version.toByteArray(Charsets.US_ASCII) + byteArrayOf(
            (width and 0xFF).toByte(), (width shr 8).toByte(), (height and 0xFF).toByte(), (height shr 8).toByte(), 0, 0, 0,
        )

    @Test fun `gif size comes from the magic bytes and little-endian logical screen`() {
        assertEquals(480 to 270, gifSize(gif("GIF89a", 480, 270)))
        assertEquals(1024 to 1, gifSize(gif("GIF87a", 1024, 1)))
        assertEquals(0x1234 to 0xABCD, gifSize(gif("GIF89a", 0x1234, 0xABCD)))
        // Not a GIF, whatever it claims: PNG, a wrong version, a short header.
        assertNull(gifSize(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0)))
        assertNull(gifSize(gif("GIF88a", 10, 10)))
        assertNull(gifSize(gif("gif89a", 10, 10)))
        assertNull(gifSize("GIF89a".toByteArray(Charsets.US_ASCII) + byteArrayOf(1, 0, 1)))
    }

    @Test fun `gifs upload unchanged up to 5 MiB and 1024 on each side`() {
        assertTrue(gifUploadsUnchanged(PicturePolicy.GIF_MAX_BYTES, 1024, 1024))
        assertTrue(gifUploadsUnchanged(812_345, 480, 480))
        assertFalse(gifUploadsUnchanged(PicturePolicy.GIF_MAX_BYTES + 1, 480, 480))
        assertFalse(gifUploadsUnchanged(1_000, 1025, 480))
        assertFalse(gifUploadsUnchanged(1_000, 480, 1025))
        assertFalse(gifUploadsUnchanged(1_000, 0, 480))
    }

    @Test fun `stills crop to the centered square and scale to at most 512`() {
        assertEquals(SquareCrop(420, 0, 3024), squareCrop(3864, 3024))
        assertEquals(SquareCrop(0, 656, 3024), squareCrop(3024, 4336))
        assertEquals(SquareCrop(0, 0, 300), squareCrop(300, 300))
        // An odd difference leaves the extra pixel on the far side.
        assertEquals(SquareCrop(0, 0, 100), squareCrop(100, 101))
        assertEquals(SquareCrop(1, 0, 100), squareCrop(103, 100))
        assertEquals(512, pictureEdge(4000, 3000))
        assertEquals(300, pictureEdge(300, 900))
        assertEquals(1, pictureEdge(1, 1))
    }

    @Test fun `decoding subsamples by powers of two while keeping the target edge`() {
        assertEquals(1, pictureSampleSize(512, 512))
        assertEquals(1, pictureSampleSize(1023, 2000))
        assertEquals(2, pictureSampleSize(1024, 2000))
        assertEquals(4, pictureSampleSize(3024, 4032))
        assertEquals(8, pictureSampleSize(4096, 8000))
        // A small picture is never subsampled below its own short edge.
        assertEquals(1, pictureSampleSize(300, 5000))
        assertEquals(1, pictureSampleSize(0, 10))
        listOf(512 to 512, 3024 to 4032, 513 to 9000, 6000 to 1200).forEach { (width, height) ->
            assertTrue(minOf(width, height) / pictureSampleSize(width, height) >= pictureEdge(width, height))
        }
    }

    @Test fun `filenames follow the purpose and the uploaded type`() {
        assertEquals("avatar.webp", pictureFilename(PictureTarget.Avatar, "image/webp"))
        assertEquals("avatar.gif", pictureFilename(PictureTarget.Avatar, "image/gif"))
        assertEquals("space-icon.gif", pictureFilename(PictureTarget.SpaceIcon("space0000001"), "image/gif"))
        assertEquals("space-icon.png", pictureFilename(PictureTarget.SpaceIcon("space0000001"), "image/png"))
        assertEquals("avatar.jpg", pictureFilename(PictureTarget.Avatar, "image/jpeg"))
    }

    @Test fun `api errors read as friendly picture copy`() {
        val avatar = PictureTarget.Avatar
        val icon = PictureTarget.SpaceIcon("space0000001")
        assertEquals("Use a PNG, JPEG, WebP or GIF picture.", pictureError(ApiException(400, "unsupported", "picture_type"), avatar))
        assertEquals("Pictures can be up to 1024 × 1024.", pictureError(ApiException(400, "invalid dimensions"), avatar))
        assertEquals("That picture is too large. Try another one.", pictureError(ApiException(413, "too large", "picture_too_large"), avatar))
        assertEquals("That picture couldn’t be used. Try another one.", pictureError(ApiException(422, "bad", "picture_invalid"), icon))
        assertEquals("You’ve changed pictures too many times today. Try again later.", pictureError(ApiException(429, "slow down", "picture_rate_limited"), avatar))
        assertEquals("Picture uploads aren’t available right now.", pictureError(ApiException(503, "uploads unavailable"), avatar))
        assertEquals("Only the space owner can change its icon.", pictureError(ApiException(404, "resource not found"), icon))
        assertEquals("That’s no longer available.", pictureError(ApiException(404, "resource not found"), avatar))
        assertEquals("The upload didn’t finish. Try again.", pictureError(ApiException(409, "upload not finished"), avatar))
        assertEquals(PicturePolicy.GIF_UNREADABLE, pictureError(PictureException(PicturePolicy.GIF_UNREADABLE), avatar))
        assertEquals("Storage refused the upload (403).", pictureError(UploadException("Storage refused the upload (403)."), avatar))
        assertEquals("Couldn’t reach Caper. Check your connection.", pictureError(IOException("reset"), avatar))
    }

    private val id = "PiCtUrE123456789"

    private fun reservation() = """{"id":"$id","kind":"image","upload":{"method":"PUT","url":"${server.url("/r2/original/$id?X-Amz-Signature=abc")}",
        "headers":{"content-type":"image/webp","content-disposition":"inline; filename=\"avatar.webp\""}}}"""

    private val account = """{"id":"account00001","username":"jose","displayName":"Jose","avatarId":16,
        "avatarAsset":"$id","avatarUrl":"https://cdn.caper.chat/assets/$id?exp=1790000000&sig=s"}"""

    @Test fun `an avatar reserves with its purpose, puts exactly the presigned headers and applies on complete`() = runBlocking {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setResponseCode(409).setBody("""{"error":"upload not finished"}"""))
        server.enqueue(MockResponse().setBody("""{"id":"$id","kind":"image","contentType":"image/webp","name":"avatar.webp","size":3,"account":$account}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val bytes = byteArrayOf(1, 2, 3)

        val result = PictureUploader(api, completeRetryDelaysMs = listOf(1)).upload("account-secret", PictureTarget.Avatar, PreparedPicture(bytes, "image/webp", 512, 512))

        assertEquals(id, result.account!!.avatarAsset)
        assertEquals(16, result.account!!.avatarId)
        val create = server.takeRequest()
        assertEquals("/api/assets", create.path)
        assertEquals("Bearer account-secret", create.headers["Authorization"])
        val body = Json.parseToJsonElement(create.body.readUtf8()).jsonObject
        assertEquals(setOf("purpose", "filename", "contentType", "byteSize", "width", "height"), body.keys)
        assertEquals("avatar", body["purpose"]!!.jsonPrimitive.content)
        assertEquals("avatar.webp", body["filename"]!!.jsonPrimitive.content)
        assertEquals("image/webp", body["contentType"]!!.jsonPrimitive.content)
        assertEquals(3L, body["byteSize"]!!.jsonPrimitive.long)
        assertEquals(512, body["width"]!!.jsonPrimitive.int)
        val put = server.takeRequest()
        assertEquals("PUT", put.method)
        assertEquals("/r2/original/$id?X-Amz-Signature=abc", put.path)
        assertEquals("image/webp", put.headers["Content-Type"])
        assertEquals("inline; filename=\"avatar.webp\"", put.headers["Content-Disposition"])
        assertNull("no credentials go to storage", put.headers["Authorization"])
        assertArrayEquals(bytes, put.body.readByteArray())
        assertEquals("/api/assets/$id/complete", server.takeRequest().path)
        assertEquals("/api/assets/$id/complete", server.takeRequest().path)
        assertEquals(4, server.requestCount)
    }

    @Test fun `a space icon names its space, and an unusable picture cancels the reservation`() = runBlocking {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setBody("""{"id":"$id","kind":"image","space":{"id":"space0000001","name":"Studio","ownerId":"account00001",
            "iconAsset":"$id","iconUrl":"https://cdn.caper.chat/assets/$id?exp=1&sig=i"}}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val target = PictureTarget.SpaceIcon("space0000001")
        val gif = PreparedPicture(gif("GIF89a", 480, 480), "image/gif", 480, 480)

        val result = PictureUploader(api).upload("account-secret", target, gif)

        assertEquals(id, result.space!!.iconAsset)
        val body = Json.parseToJsonElement(server.takeRequest().body.readUtf8()).jsonObject
        assertEquals("spaceIcon", body["purpose"]!!.jsonPrimitive.content)
        assertEquals("space0000001", body["spaceId"]!!.jsonPrimitive.content)
        assertEquals("space-icon.gif", body["filename"]!!.jsonPrimitive.content)
        repeat(2) { server.takeRequest() }

        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setResponseCode(422).setBody("""{"error":"invalid picture","code":"picture_invalid"}"""))
        server.enqueue(MockResponse().setResponseCode(204))
        val error = runCatching { PictureUploader(api).upload("account-secret", target, gif) }.exceptionOrNull()!!
        assertEquals(PicturePolicy.UNUSABLE, pictureError(error, target))
        repeat(3) { server.takeRequest() }
        assertEquals("/api/assets/$id/cancel", server.takeRequest().path)
    }

    @Test fun `a refused reservation sends no bytes, and removal returns the account without a picture`() = runBlocking {
        server.enqueue(MockResponse().setResponseCode(429).setBody("""{"error":"too many pictures","code":"picture_rate_limited"}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val error = runCatching {
            PictureUploader(api).upload("account-secret", PictureTarget.Avatar, PreparedPicture(byteArrayOf(1), "image/webp", 1, 1))
        }.exceptionOrNull()!!
        assertEquals("picture_rate_limited", (error as ApiException).code)
        assertEquals(1, server.requestCount)
        server.takeRequest()

        server.enqueue(MockResponse().setBody("""{"id":"account00001","username":"jose","displayName":"Jose","avatarId":16}"""))
        server.enqueue(MockResponse().setBody("""{"id":"space0000001","name":"Studio","ownerId":"account00001"}"""))
        val removed = api.removeAvatar("account-secret")
        assertNull(removed.avatarAsset)
        assertFalse(removed.canRestoreAvatar)
        assertEquals(16, removed.avatarId)
        assertNull(api.removeSpaceIcon("account-secret", "space0000001").iconAsset)
        val avatar = server.takeRequest()
        assertEquals("DELETE", avatar.method)
        assertEquals("/api/account/avatar", avatar.path)
        val icon = server.takeRequest()
        assertEquals("DELETE", icon.method)
        assertEquals("/api/spaces/space0000001/icon", icon.path)
    }

    @Test fun `restore sends current asset including mascot null and returns capability`() = runBlocking {
        val api = CaperApi(baseUrl = server.url("/").toString())
        for (asset in listOf(id, null)) {
            server.enqueue(MockResponse().setBody("""{"id":"account00001","avatarAsset":"$id","canRestoreAvatar":true}"""))
            assertTrue(api.restoreAvatar("account-secret", asset).canRestoreAvatar)
            val request = server.takeRequest()
            assertEquals("POST", request.method)
            assertEquals("/api/account/avatar/restore", request.path)
            val body = Json.parseToJsonElement(request.body.readUtf8()).jsonObject
            assertEquals(if (asset == null) "null" else "\"$asset\"", body["avatarAsset"].toString())
        }
        server.enqueue(MockResponse().setResponseCode(409).setBody("""{"error":"stale avatar"}"""))
        val error = runCatching { api.restoreAvatar("account-secret", id) }.exceptionOrNull()
        assertTrue(error is ApiException && error.status == 409)
    }

    @Test fun `empty pictures are refused before reserving`() = runBlocking {
        val error = runCatching {
            PictureUploader(CaperApi(baseUrl = server.url("/").toString()))
                .upload("account-secret", PictureTarget.Avatar, PreparedPicture(ByteArray(0), "image/webp", 1, 1))
        }.exceptionOrNull()
        assertTrue(error is PictureException)
        assertEquals(0, server.requestCount)
    }
}
