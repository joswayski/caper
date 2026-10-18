package chat.caper.android.data

import chat.caper.android.model.InviteLink
import chat.caper.android.model.InviteLinkChannel
import chat.caper.android.model.InvitePerson
import java.time.Instant
import kotlinx.coroutines.test.runTest
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class InviteLinksTest {
    private val server = MockWebServer()

    @After fun close() { server.close() }

    @Test fun `codes come from links on any host, scheme-less links and bare codes`() {
        assertEquals("aB3dE5fG7h", inviteCode("https://caper.chat/invite/aB3dE5fG7h"))
        assertEquals("aB3dE5fG7h", inviteCode("  https://caper.chat/invite/aB3dE5fG7h/  "))
        assertEquals("aB3dE5fG7h", inviteCode("https://caper.chat/invite/aB3dE5fG7h?utm=chat"))
        assertEquals("aB3dE5fG7h", inviteCode("https://caper.chat/invite/aB3dE5fG7h/?ref=1#top"))
        assertEquals("aB3dE5fG7h", inviteCode("http://localhost:3000/invite/aB3dE5fG7h"))
        assertEquals("aB3dE5fG7h", inviteCode("https://staging.example.com/invite/aB3dE5fG7h"))
        assertEquals("aB3dE5fG7h", inviteCode("caper.chat/invite/aB3dE5fG7h"))
        assertEquals("aB3dE5fG7h", inviteCode("aB3dE5fG7h"))
        assertEquals("0123456789", inviteCode("\n0123456789\t"))
    }

    @Test fun `anything else is not an invite`() {
        listOf(
            "", "   ", "aB3dE5fG7", "aB3dE5fG7hX", "aB3dE5fG7-", "aB3 dE5fG7h",
            "https://caper.chat/invite/aB3dE5fG7", "https://caper.chat/invite/aB3dE5fG7hX",
            "https://caper.chat/invite/", "https://caper.chat/invites/aB3dE5fG7h", "https://caper.chat/aB3dE5fG7h",
            "https://caper.chat/invite/aB3dE5fG7h/extra", "/invite/aB3dE5fG7h", "invite/aB3dE5fG7h",
            "https://caper.chat/invite/aB3dE5fG7h trailing",
        ).forEach { assertNull(it, inviteCode(it)) }
    }

    @Test fun `codes come from the app's caper scheme`() {
        assertEquals("aB3dE5fG7h", inviteCode("caper://invite/aB3dE5fG7h"))
        assertEquals("aB3dE5fG7h", inviteCode("caper://invite/aB3dE5fG7h/"))
        assertEquals("aB3dE5fG7h", inviteCode("caper://invite/aB3dE5fG7h?from=web"))
        assertEquals("aB3dE5fG7h", inviteCode("caper://invite/aB3dE5fG7h/#top"))
        assertEquals("aB3dE5fG7h", inviteCode("CAPER://Invite/aB3dE5fG7h"))
        assertEquals("aB3dE5fG7h", inviteCode("  caper://invite/aB3dE5fG7h\n"))
    }

    @Test fun `other caper scheme links are not invites`() {
        listOf(
            "caper://invite/", "caper://invite/aB3dE5fG7", "caper://invite/aB3dE5fG7hX", "caper://invite/aB3dE5f-7h",
            "caper://invites/aB3dE5fG7h", "caper://join/aB3dE5fG7h", "caper:invite/aB3dE5fG7h", "caper:///invite/aB3dE5fG7h",
            "caper://invite/aB3dE5fG7h/extra", "caper://invite/aB3dE5fG7h trailing", "other://invite/aB3dE5fG7h",
        ).forEach { assertNull(it, inviteCode(it)) }
    }

    @Test fun `links use the API origin`() {
        assertEquals("https://caper.chat/invite/aB3dE5fG7h", inviteLinkUrl("https://caper.chat", "aB3dE5fG7h"))
        assertEquals("https://caper.chat/invite/aB3dE5fG7h", inviteLinkUrl("https://caper.chat/", "aB3dE5fG7h"))
        assertEquals("https://caper.chat/invite/aB3dE5fG7h", CaperApi(baseUrl = "https://caper.chat/api").inviteUrl("aB3dE5fG7h"))
    }

    @Test fun `meta lines read like web`() {
        val now = Instant.parse("2026-10-11T12:00:00Z")
        val link = InviteLink(
            code = "aB3dE5fG7h", channel = InviteLinkChannel("channel00001", "feedback"),
            createdBy = InvitePerson("mira", "Mira"), uses = 3, maxUses = 10,
            expiresAt = "2026-10-17T12:00:00.123456Z", createdAt = "2026-10-10T12:00:00Z",
        )
        assertEquals("#feedback · 7 of 10 joins remaining · Expires in 6 days", inviteLinkMeta(link, now))
        assertEquals("2 uses · Never expires", inviteLinkMeta(link.copy(channel = null, uses = 2, maxUses = null, expiresAt = null), now))
        assertEquals("1 use · Never expires", inviteLinkMeta(link.copy(channel = null, uses = 1, maxUses = null, expiresAt = null), now))
        assertEquals("Expires in 30 minutes", inviteExpiryLabel("2026-10-11T12:30:00Z", now))
        assertEquals("Expires in 1 minute", inviteExpiryLabel("2026-10-11T12:00:20Z", now))
        assertEquals("Expires in 1 hour", inviteExpiryLabel("2026-10-11T13:00:00Z", now))
        assertEquals("Expires in 12 hours", inviteExpiryLabel("2026-10-12T00:00:00Z", now))
        assertEquals("Expires in 1 day", inviteExpiryLabel("2026-10-12T11:59:00Z", now))
        assertEquals("Expires in 7 days", inviteExpiryLabel("2026-10-18T12:00:00Z", now))
        assertEquals("Expired", inviteExpiryLabel("2026-10-11T11:59:59Z", now))
        assertNull(inviteExpiryLabel("soon", now))
        assertEquals("1 member", inviteMemberCount(1))
        assertEquals("4 members", inviteMemberCount(4))
    }

    @Test fun `expiry is a number of minutes, hours or days, or never, with web's default`() {
        assertEquals("30", DEFAULT_INVITE_EXPIRY_NUMBER)
        assertEquals(InviteExpiryUnit.Days, DEFAULT_INVITE_EXPIRY_UNIT)
        assertEquals(InviteOption.Valid(100), inviteMaxUses(DEFAULT_INVITE_MAX_USES))
        assertEquals(listOf("minutes", "hours", "days", "Never"), InviteExpiryUnit.entries.map { it.label })
        assertEquals(InviteOption.Valid(2_592_000), inviteExpirySeconds(DEFAULT_INVITE_EXPIRY_NUMBER, DEFAULT_INVITE_EXPIRY_UNIT))
        assertEquals(InviteOption.Valid(60), inviteExpirySeconds("1", InviteExpiryUnit.Minutes))
        assertEquals(InviteOption.Valid(5_400), inviteExpirySeconds(" 90 ", InviteExpiryUnit.Minutes))
        assertEquals(InviteOption.Valid(3_600), inviteExpirySeconds("1", InviteExpiryUnit.Hours))
        assertEquals(InviteOption.Valid(31_536_000), inviteExpirySeconds("8760", InviteExpiryUnit.Hours))
        assertEquals(InviteOption.Valid(31_536_000), inviteExpirySeconds("365", InviteExpiryUnit.Days))
        assertEquals(InviteOption.Valid(31_536_000), inviteExpirySeconds("525600", InviteExpiryUnit.Minutes))
        assertEquals(InviteOption.Valid(604_800), inviteExpirySeconds("007", InviteExpiryUnit.Days))
        // Never ignores the number, even a bad one.
        listOf("7", "", "0", "abc").forEach { assertEquals(it, InviteOption.Valid(null), inviteExpirySeconds(it, InviteExpiryUnit.Never)) }
    }

    @Test fun `expiry outside 1 minute to 365 days or not a whole number is refused`() {
        val invalid = InviteOption.Invalid("Expiry must be between 1 minute and 365 days.")
        listOf(
            "0" to InviteExpiryUnit.Minutes, "0" to InviteExpiryUnit.Days, "366" to InviteExpiryUnit.Days,
            "8761" to InviteExpiryUnit.Hours, "525601" to InviteExpiryUnit.Minutes,
            "" to InviteExpiryUnit.Days, "  " to InviteExpiryUnit.Hours, "-1" to InviteExpiryUnit.Days, "+1" to InviteExpiryUnit.Days,
            "1.5" to InviteExpiryUnit.Days, "1,000" to InviteExpiryUnit.Minutes, "1e3" to InviteExpiryUnit.Minutes,
            "seven" to InviteExpiryUnit.Days, "7 days" to InviteExpiryUnit.Days, "\u0667" to InviteExpiryUnit.Days,
            "99999999999999999999" to InviteExpiryUnit.Minutes, "2147483648" to InviteExpiryUnit.Days,
        ).forEach { (number, unit) -> assertEquals("$number $unit", invalid, inviteExpirySeconds(number, unit)) }
    }

    @Test fun `max uses is empty for no limit or a whole number from 1 to 10,000`() {
        assertEquals(InviteOption.Valid(null), inviteMaxUses(""))
        assertEquals(InviteOption.Valid(null), inviteMaxUses("   "))
        assertEquals(InviteOption.Valid(1), inviteMaxUses("1"))
        assertEquals(InviteOption.Valid(25), inviteMaxUses(" 25\n"))
        assertEquals(InviteOption.Valid(10_000), inviteMaxUses("10000"))
        val invalid = InviteOption.Invalid("Max uses must be a whole number from 1 to 10,000.")
        listOf("0", "10001", "-5", "+5", "2.5", "10,000", "1e3", "ten", "5 uses", "\u0665", "99999999999999999999")
            .forEach { assertEquals(it, invalid, inviteMaxUses(it)) }
        assertEquals("8,766 of 10,000 joins remaining", inviteUsesLabel(1_234, 10_000))
        assertEquals("0 of 1 join remaining", inviteUsesLabel(1, 1))
        assertEquals("0 of 1 join remaining", inviteUsesLabel(2, 1))
    }

    @Test fun `invalid links read as web's sentence`() {
        assertEquals(INVALID_INVITE_LINK, friendlyError(ApiException(404, "invite link is invalid or has expired")))
        assertEquals(
            "You can't join this space with a link. Ask the owner to invite you.",
            friendlyError(ApiException(403, "you can't join this space with a link; ask the owner to invite you")),
        )
    }

    @Test fun `no public channels prepares a space-only link and preserves server values on reopening`() = runTest {
        val link = """{"code":"aB3dE5fG7h","channel":null,"createdBy":{"username":"mira","displayName":"Mira","avatarId":3},"uses":7,"maxUses":100,"expiresAt":"2026-10-27T12:00:00Z","createdAt":"2026-10-11T12:00:00Z"}"""
        repeat(2) { server.enqueue(MockResponse().setBody(link)) }
        val api = CaperApi(baseUrl = server.url("/").toString())
        val first = api.ensureDefaultInviteLink("account-secret", "space0000001")
        assertEquals(first, api.ensureDefaultInviteLink("account-secret", "space0000001"))
        assertNull(first.channel)
        assertEquals(100, first.maxUses)
        assertEquals(7, first.uses)
        assertEquals(16 * 86_400L, java.time.Duration.between(Instant.parse(first.createdAt), Instant.parse(first.expiresAt)).seconds)
        repeat(2) {
            val request = server.takeRequest()
            assertEquals("POST", request.method)
            assertEquals("/api/spaces/space0000001/invite-links/default", request.path)
            assertEquals("Bearer account-secret", request.headers["Authorization"])
            assertEquals("", request.body.readUtf8())
        }
    }

    @Test fun `selected channel preparation is bodyless, reuses server values and propagates failures`() = runTest {
        val link = """{"code":"aB3dE5fG7h","channel":{"id":"channel00001","name":"design"},"createdBy":{"username":"mira","displayName":"Mira","avatarId":3},"uses":7,"maxUses":100,"expiresAt":"2026-10-27T12:00:00Z","createdAt":"2026-10-11T12:00:00Z"}"""
        repeat(2) { server.enqueue(MockResponse().setBody(link)) }
        server.enqueue(MockResponse().setResponseCode(400).setBody("""{"error":"channel must be public"}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val first = api.ensureDefaultInviteLink("account-secret", "space0000001", "channel00001")
        assertEquals("channel00001", first.channel?.id)
        assertEquals("design", first.channel?.name)
        assertEquals(first, api.ensureDefaultInviteLink("account-secret", "space0000001", "channel00001"))
        val error = runCatching { api.ensureDefaultInviteLink("account-secret", "space0000001", "channel00002") }.exceptionOrNull() as ApiException
        assertEquals(400, error.status)
        repeat(3) { index ->
            val request = server.takeRequest()
            assertEquals("POST", request.method)
            assertEquals("/api/spaces/space0000001/invite-links/default?channelId=${if (index == 2) "channel00002" else "channel00001"}", request.path)
            assertEquals("Bearer account-secret", request.headers["Authorization"])
            assertEquals("", request.body.readUtf8())
        }
    }

    @Test fun `regeneration uses one bodyless authenticated POST and propagates stale failures`() = runTest {
        val link = """{"code":"newCode123","channel":{"id":"channel00001","name":"design"},"createdBy":{"username":"mira","displayName":"Mira","avatarId":3},"uses":0,"maxUses":100,"expiresAt":"2026-11-10T12:00:00Z","createdAt":"2026-10-11T12:00:00Z"}"""
        server.enqueue(MockResponse().setResponseCode(201).setBody(link))
        server.enqueue(MockResponse().setResponseCode(404).setBody("""{"error":"invite link is invalid or has expired"}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val replacement = api.regenerateInviteLink("account-secret", "space0000001", "aB3dE5fG7h")
        assertEquals("newCode123", replacement.code)
        assertEquals("channel00001", replacement.channel?.id)
        assertEquals(0, replacement.uses)
        assertEquals(100, replacement.maxUses)
        assertEquals(30 * 86_400L, java.time.Duration.between(Instant.parse(replacement.createdAt), Instant.parse(replacement.expiresAt)).seconds)
        val stale = runCatching { api.regenerateInviteLink("account-secret", "space0000001", "aB3dE5fG7h") }.exceptionOrNull() as ApiException
        assertEquals(404, stale.status)
        repeat(2) {
            val request = server.takeRequest()
            assertEquals("POST", request.method)
            assertEquals("/api/spaces/space0000001/invite-links/aB3dE5fG7h/regenerate", request.path)
            assertEquals("Bearer account-secret", request.headers["Authorization"])
            assertEquals("", request.body.readUtf8())
        }
        assertEquals(2, server.requestCount)
    }

    @Test fun `invite link requests use the spec's routes and bodies`() = runTest {
        val link = """{"code":"aB3dE5fG7h","channel":null,"createdBy":{"username":"mira","displayName":"Mira","avatarId":3},"uses":0,"maxUses":null,"expiresAt":null,"createdAt":"2026-10-11T12:00:00Z"}"""
        server.enqueue(MockResponse().setBody("""{"code":"aB3dE5fG7h","space":{"name":"Studio"},"channel":{"name":"feedback"},"inviter":{"username":"mira","displayName":"Mira","avatarId":3},"memberCount":4,"expiresAt":null,"alreadyMember":true}"""))
        server.enqueue(MockResponse().setBody("""{"space":{"id":"space0000001","name":"Studio","ownerId":"account00001"},"channelId":null,"joined":false}"""))
        server.enqueue(MockResponse().setBody("""{"links":[$link]}"""))
        server.enqueue(MockResponse().setResponseCode(201).setBody(link))
        server.enqueue(MockResponse().setResponseCode(204))
        server.enqueue(MockResponse().setResponseCode(404).setBody("""{"error":"invite link is invalid or has expired"}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())

        val preview = api.invitePreview("aB3dE5fG7h", "account-secret")
        assertEquals("Studio", preview.space.name)
        assertEquals("feedback", preview.channel?.name)
        assertEquals(4, preview.memberCount)
        assertTrue(preview.alreadyMember)
        val joined = api.joinInvite("account-secret", "aB3dE5fG7h")
        assertEquals("space0000001", joined.space.id)
        assertNull(joined.channelId)
        assertFalse(joined.joined)
        assertNull(api.inviteLinks("account-secret", "space0000001").links.single().maxUses)
        assertEquals("aB3dE5fG7h", api.createInviteLink("account-secret", "space0000001", null, 604_800, null).code)
        api.revokeInviteLink("account-secret", "space0000001", "aB3dE5fG7h")
        val missing = runCatching { api.invitePreview("zzzzzzzzzz") }.exceptionOrNull() as ApiException
        assertEquals(404, missing.status)

        val previewRequest = server.takeRequest()
        assertEquals("GET", previewRequest.method)
        assertEquals("/api/invites/aB3dE5fG7h", previewRequest.path)
        assertEquals("Bearer account-secret", previewRequest.headers["Authorization"])
        val join = server.takeRequest()
        assertEquals("POST", join.method)
        assertEquals("/api/invites/aB3dE5fG7h/join", join.path)
        assertEquals("Bearer account-secret", join.headers["Authorization"])
        assertEquals("/api/spaces/space0000001/invite-links", server.takeRequest().path)
        val create = server.takeRequest()
        assertEquals("POST", create.method)
        assertEquals("/api/spaces/space0000001/invite-links", create.path)
        assertEquals("""{"channelId":null,"maxAge":604800,"maxUses":null}""", create.body.readUtf8())
        val revoke = server.takeRequest()
        assertEquals("DELETE", revoke.method)
        assertEquals("/api/spaces/space0000001/invite-links/aB3dE5fG7h", revoke.path)
    }

    @Test(expected = IllegalArgumentException::class)
    fun `malformed codes never reach the network`() = runTest {
        CaperApi(baseUrl = server.url("/").toString()).invitePreview("../spaces")
    }
}
