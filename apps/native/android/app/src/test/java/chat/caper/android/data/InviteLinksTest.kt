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
        assertEquals("#feedback · 3 of 10 uses · Expires in 6 days", inviteLinkMeta(link, now))
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

    @Test fun `pickers offer the API's choices with web's defaults`() {
        assertEquals(listOf(3_600, 86_400, 604_800, 2_592_000, null), INVITE_MAX_AGES.map { it.first })
        assertEquals(604_800, DEFAULT_INVITE_MAX_AGE)
        assertEquals(listOf(null, 1, 5, 10, 25, 50, 100, 500, 1_000, 5_000, 10_000), INVITE_MAX_USES.map { it.first })
        assertEquals("No limit", INVITE_MAX_USES.first().second)
        assertEquals("1 use", INVITE_MAX_USES[1].second)
        assertEquals("10,000 uses", INVITE_MAX_USES.last().second)
        assertEquals("1,234 of 10,000 uses", inviteUsesLabel(1_234, 10_000))
    }

    @Test fun `invalid links read as web's sentence`() {
        assertEquals(INVALID_INVITE_LINK, friendlyError(ApiException(404, "invite link is invalid or has expired")))
        assertEquals(
            "You can't join this space with a link. Ask the owner to invite you.",
            friendlyError(ApiException(403, "you can't join this space with a link; ask the owner to invite you")),
        )
    }

    @Test fun `invite link requests use the spec's routes and bodies`() = runTest {
        val link = """{"code":"aB3dE5fG7h","channel":null,"createdBy":{"username":"mira","displayName":"Mira","avatarId":3},"uses":0,"maxUses":null,"expiresAt":null,"createdAt":"2026-10-11T12:00:00Z"}"""
        server.enqueue(MockResponse().setBody("""{"code":"aB3dE5fG7h","space":{"name":"Studio"},"channel":{"name":"feedback"},"inviter":{"username":"mira","displayName":"Mira","avatarId":3},"memberCount":4,"expiresAt":null}"""))
        server.enqueue(MockResponse().setBody("""{"space":{"id":"space0000001","name":"Studio","ownerId":"account00001"},"channelId":null,"joined":false}"""))
        server.enqueue(MockResponse().setBody("""{"links":[$link]}"""))
        server.enqueue(MockResponse().setResponseCode(201).setBody(link))
        server.enqueue(MockResponse().setResponseCode(204))
        server.enqueue(MockResponse().setResponseCode(404).setBody("""{"error":"invite link is invalid or has expired"}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())

        val preview = api.invitePreview("aB3dE5fG7h")
        assertEquals("Studio", preview.space.name)
        assertEquals("feedback", preview.channel?.name)
        assertEquals(4, preview.memberCount)
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
        assertNull(previewRequest.headers["Authorization"])
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
