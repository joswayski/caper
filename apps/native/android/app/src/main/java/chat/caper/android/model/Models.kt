package chat.caper.android.model

import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonTransformingSerializer

@Serializable data class MediaStatus(val enabled: Boolean = false)
/**
 * [feedbackTeam]: a Caper feedback team member, who gets the Feedback inbox; absent (false) for everyone else.
 * [avatarAsset]/[avatarUrl]: a custom picture in place of the saved mascot (see [pictureOf]); every
 * person object below that carries `avatarId` may carry them too.
 */
@Serializable data class Account(
    val id: String, val username: String? = null, val displayName: String? = null, val debugEnabled: Boolean = false, val avatarId: Int? = null,
    val feedbackTeam: Boolean = false,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
)
@Serializable data class Challenge(val challengeId: String)
@Serializable data class VerifyResult(val account: Account, val token: String)
@Serializable data class Inviter(val username: String, val displayName: String)
/** [iconAsset]/[iconUrl]: the owner's icon in place of the initial (see [pictureOf]). */
@Serializable data class Space(
    val id: String, val name: String, val ownerId: String = "", val demo: Boolean = false, val inviter: Inviter? = null,
    val iconAsset: String? = null, val iconUrl: String? = null,
)
@Serializable data class SpaceList(val spaces: List<Space>, val invitations: List<Space> = emptyList(), val limits: SpaceLimits)
@Serializable data class SpaceLimits(val ownedSpaces: Int, val totalSpaces: Int, val channelsPerSpace: Int)
@Serializable data class UnreadChannel(val id: String, val lastSeq: String, val readSeq: String? = null)
@Serializable data class UnreadSpace(val id: String, val channels: List<UnreadChannel>)
@Serializable data class UnreadSpaces(val spaces: List<UnreadSpace>, val feedbackWaiting: Int = 0)
@Serializable data class Channel(
    val id: String,
    val spaceId: String,
    val name: String,
    val private: Boolean,
    val direct: Boolean = false,
    val joined: Boolean = true,
)
/** [team]: the server's "Caper" account; every top-level message to it reaches the Caper team. */
@Serializable data class DirectPeer(
    val id: String, val username: String, val displayName: String, val avatarId: Int? = null, val team: Boolean = false,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
    val admin: Boolean = false,
) { val canBlock: Boolean get() = !admin && !team }
/**
 * A DM. [status] is `accepted`, `outgoing` (you asked, they haven't accepted) or
 * `incoming` (a message request to you); old servers omit it, which means
 * accepted. [blocked] is true when you blocked the peer.
 */
@Serializable data class DirectConversation(
    val id: String,
    val peer: DirectPeer,
    val lastSeq: String,
    val readSeq: String,
    val status: String? = null,
    val blocked: Boolean = false,
) {
    val incoming: Boolean get() = status == "incoming"
    val outgoing: Boolean get() = status == "outgoing"
}
@Serializable data class BlockedAccount(
    val id: String, val username: String, val displayName: String, val avatarId: Int? = null,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
    val admin: Boolean = false, val team: Boolean = false,
) { val canBlock: Boolean get() = !admin && !team }
@Serializable data class BlockList(val blocks: List<BlockedAccount>)
/** `GET/PUT /api/account/privacy`: `anyone`, `spaces` or `nobody`. */
@Serializable data class DirectPrivacy(val directMessages: String)
@Serializable data class DirectConversationList(val conversations: List<DirectConversation>)
/** `GET /api/people`: accounts sharing a space or a DM with you (never you), by username. */
@Serializable data class Person(
    val id: String, val username: String, val displayName: String, val avatarId: Int? = null,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
    val admin: Boolean = false,
)
@Serializable data class PeopleList(val people: List<Person>)
@Serializable data class PushConfig(val platforms: List<String>)
/**
 * `GET /api/notifications/settings`: the account [level] (`all`, `mentions` or `nothing`),
 * [mobile] (`whenInactive` or `always`) and the overrides that still set something.
 */
@Serializable data class NotificationSettings(
    val level: String = "all",
    val mobile: String = "whenInactive",
    val overrides: List<NotificationOverride> = emptyList(),
)
/**
 * One space, channel (with its space) or DM override. [level] is null to inherit; a DM only
 * uses `nothing`. [mutedUntil] is an RFC 3339 UTC time, `forever`, or null.
 */
@Serializable data class NotificationOverride(
    val spaceId: String? = null,
    val channelId: String? = null,
    val conversationId: String? = null,
    val level: String? = null,
    val mutedUntil: String? = null,
)
@Serializable data class ChannelInvitation(val channel: Channel, val inviter: Inviter)
/** An invite link's inviter or creator: public identity only. */
@Serializable data class InvitePerson(
    val username: String, val displayName: String, val avatarId: Int? = null,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
)
@Serializable data class InviteNamed(val name: String)
/** The previewed space: its name and, when the owner set one, its icon. */
@Serializable data class InviteSpace(val name: String, val iconAsset: String? = null, val iconUrl: String? = null)
/** `GET /api/invites/{code}`: what a live link shares before joining. [channel] is its public landing channel. */
@Serializable data class InvitePreview(
    val code: String,
    val space: InviteSpace,
    val channel: InviteNamed? = null,
    val inviter: InvitePerson,
    val memberCount: Int = 0,
    val expiresAt: String? = null,
    val alreadyMember: Boolean = false,
)
/** `POST /api/invites/{code}/join`: [joined] is false when you already belonged. */
@Serializable data class InviteJoin(val space: Space, val channelId: String? = null, val joined: Boolean = true)
@Serializable data class InviteLinkChannel(val id: String, val name: String)
/** An owner's live invite link. Null [maxUses] has no limit; null [expiresAt] never expires. */
@Serializable data class InviteLink(
    val code: String,
    val channel: InviteLinkChannel? = null,
    val createdBy: InvitePerson,
    val uses: Int = 0,
    val maxUses: Int? = null,
    val expiresAt: String? = null,
    val createdAt: String,
)
@Serializable data class InviteLinkList(val links: List<InviteLink>)
@Serializable data class Member(
    val id: String, val username: String, val displayName: String, val owner: Boolean, val avatarId: Int? = null,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
    val admin: Boolean = false,
)
@Serializable data class SpaceDetail(
    val space: Space,
    val channels: List<Channel>,
    val members: List<Member>,
    val channelInvitations: List<ChannelInvitation> = emptyList(),
    /** Private channels you can read: who can read each (owner and grants). */
    val channelMembers: Map<String, List<String>> = emptyMap(),
) {
    /** A private channel's readers, then the rest of the space, each in the space's order; null for public channels. */
    fun memberGroups(channelId: String?): Pair<List<Member>, List<Member>>? {
        val ids = channelMembers[channelId ?: return null]?.toSet() ?: return null
        return members.partition { it.id in ids }
    }

    /** The members in the order the member list shows them for [channelId]. */
    fun memberOrder(channelId: String?): List<Member> = memberGroups(channelId)?.let { it.first + it.second } ?: members
}
/** [admin]: sent by a Caper feedback team member (ADMIN badge), stamped by the server at send time. */
@Serializable data class ChatAuthor(
    val id: String, val name: String, val isGuest: Boolean, val avatarId: Int? = null, val admin: Boolean = false,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
)
/**
 * One `content.mentions` entry: `user` (with `id` and `username`), `everyone`, `here` or
 * `feedback` (the message asked the Caper team to look: `@feedback`/`@caper`).
 * Other types decode too and are ignored where mentions are used.
 */
@Serializable data class MessageMention(val type: String, val id: String? = null, val username: String? = null)
@Serializable data class ChatContent(
    val version: Int,
    val type: String,
    val text: String,
    /** Additive file list. Malformed entries are dropped, never the message. */
    @Serializable(with = TolerantAttachmentsSerializer::class) val attachments: List<ChatAttachment> = emptyList(),
    /** Absent on older messages and servers. */
    @Serializable(with = MentionListSerializer::class) val mentions: List<MessageMention> = emptyList(),
)

/** A file on a message. URLs are signed per response and expire in 24–48 hours. */
@Serializable data class ChatAttachment(
    val id: String,
    val kind: String,
    val contentType: String,
    val name: String,
    val size: Long,
    val width: Int? = null,
    val height: Int? = null,
    val durationMs: Long? = null,
    /** Present (as `{}`) when a preview image exists. */
    val preview: JsonObject? = null,
    /** `processing`, `ready` or `failed`; absent in older payloads, which are ready. */
    val status: String? = null,
    /** A GIF or animated image stored as a silent looping MP4: play it like a GIF. */
    val animated: Boolean = false,
    /** Only ready files have a `url`; `previewUrl` can appear while processing. */
    val url: String? = null,
    val previewUrl: String? = null,
    /** The file was deleted: show a placeholder. */
    val unavailable: Boolean = false,
) {
    /** How to show the file. Unknown future states fall back on whether a URL was delivered. */
    val state: AttachmentState get() = when (status) {
        null, "ready" -> AttachmentState.READY
        "processing", "uploading" -> AttachmentState.PROCESSING
        "failed" -> AttachmentState.FAILED
        else -> if (url != null) AttachmentState.READY else AttachmentState.PROCESSING
    }

    internal fun isValid(): Boolean {
        val web = { value: String? -> value == null || value.startsWith("https://") || value.startsWith("http://") }
        return id.isNotEmpty() && name.isNotEmpty() && contentType.isNotEmpty() && size >= 0 &&
            kind in ATTACHMENT_KINDS && (width ?: 0) >= 0 && (height ?: 0) >= 0 && (durationMs ?: 0) >= 0 &&
            web(url) && web(previewUrl)
    }
}

enum class AttachmentState { PROCESSING, READY, FAILED }

val ATTACHMENT_KINDS = setOf("image", "video", "audio", "file")

/** Mirrors the web's `attachmentsOf`: one bad file never breaks a history page or frame. */
object TolerantAttachmentsSerializer : KSerializer<List<ChatAttachment>> {
    private val delegate = ListSerializer(ChatAttachment.serializer())
    private val lenient = Json { ignoreUnknownKeys = true }
    override val descriptor: SerialDescriptor = delegate.descriptor
    override fun serialize(encoder: Encoder, value: List<ChatAttachment>) = delegate.serialize(encoder, value)
    override fun deserialize(decoder: Decoder): List<ChatAttachment> {
        val input = decoder as? JsonDecoder ?: return delegate.deserialize(decoder)
        val array = input.decodeJsonElement() as? JsonArray ?: return emptyList()
        return array.mapNotNull { element ->
            runCatching { lenient.decodeFromJsonElement(ChatAttachment.serializer(), element) }.getOrNull()?.takeIf { it.isValid() }
        }
    }
}

/** `POST /api/assets/urls`: only ready files get a `url`; a processing file may get just a preview. */
@Serializable data class AttachmentUrls(val url: String? = null, val previewUrl: String? = null)
@Serializable data class AttachmentUrlsResponse(val urls: Map<String, AttachmentUrls> = emptyMap())

/** Server-tunable client compression (`GET /api/assets/usage`, docs/media.md "Client compression and previews"). */
@Serializable data class CompressionSettings(
    /** WebP/JPEG photo quality, also the fallback when AVIF cannot be encoded; 100 disables lossy photo encoding. */
    val imageQuality: Int = 92,
    val imageMaxEdge: Int = 4096,
    val paletteColors: Int = 256,
    val previewEdge: Int = 640,
    val videoMaxHeight: Int = 1440,
    /** Video target at 1080p30, scaled by pixel count and frame rate. */
    val videoBitrateKbps: Int = 8000,
    val audioBitrateKbps: Int = 128,
    /** Photo format: only exactly `avif` selects AVIF; older servers omit it and keep WebP. */
    val imageFormat: String = "webp",
    /** AVIF photo quality on libavif's `quality` scale. */
    val avifQuality: Int = 85,
    /** A re-encode never exceeds this percentage of a non-H.264 source's own bitrate (H.264: 100%); 0 removes the cap. */
    val videoSourceBitratePercent: Int = 200,
    /** Above 30 fps the target grows with the frame rate, up to this percentage; 100 turns it off. */
    val videoHighFpsBoostPercent: Int = 150,
)
/** `GET /api/assets/usage`: stored bytes, the allowance and the compression settings. */
@Serializable data class AssetUsage(val used: Long, val limit: Long, val compression: CompressionSettings = CompressionSettings())
@Serializable data class PresignedUpload(val method: String = "PUT", val url: String, val headers: Map<String, String> = emptyMap())
@Serializable data class AssetReservation(val id: String, val kind: String = "file", val upload: PresignedUpload, val previewUpload: PresignedUpload? = null)
/** A completed picture upload, already applied: the [account] (profile picture) or the [space] (icon) with it. */
@Serializable data class PictureCompletion(val id: String, val account: Account? = null, val space: Space? = null)

/** Keeps a malformed or future-shaped mention entry from failing the whole message. */
object MentionListSerializer : JsonTransformingSerializer<List<MessageMention>>(ListSerializer(MessageMention.serializer())) {
    override fun transformDeserialize(element: JsonElement): JsonElement =
        JsonArray((element as? JsonArray).orEmpty().mapNotNull { entry ->
            val fields = entry as? JsonObject ?: return@mapNotNull null
            val type = fields.string("type") ?: return@mapNotNull null
            JsonObject(buildMap {
                put("type", JsonPrimitive(type))
                fields.string("id")?.let { put("id", JsonPrimitive(it)) }
                fields.string("username")?.let { put("username", JsonPrimitive(it)) }
            })
        })

    private fun JsonObject.string(key: String): String? = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
}
/** `GET /api/feedback` (feedback team only): who wrote `@feedback`. */
@Serializable data class FeedbackReporter(
    val id: String, val username: String? = null, val displayName: String? = null, val avatarId: Int? = null,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
) {
    val name: String get() = displayName ?: username ?: "Someone"
}
/** Where a report was written: a space channel, or a DM named after its people ([direct]). */
@Serializable data class FeedbackChannel(val id: String, val name: String, val direct: Boolean = false)
@Serializable data class FeedbackSpace(val id: String, val name: String)
/**
 * One `@feedback` report. [threadRootId] is where replies go: the message itself when it
 * was top-level, else its thread root. [answered]: the newest message in that thread is the
 * team's. [archived]: the team archived the thread (shared by the whole team).
 */
@Serializable data class FeedbackReport(
    val id: String,
    val messageId: String,
    val threadRootId: String,
    val createdAt: String,
    val text: String = "",
    val attachments: Int = 0,
    val reporter: FeedbackReporter,
    val channel: FeedbackChannel,
    val space: FeedbackSpace? = null,
    val replyCount: Int = 0,
    val answered: Boolean = false,
    val archived: Boolean = false,
)
/** A page of reports, newest first; pass [cursor] as `?before=` for the next one (null at the end). */
@Serializable data class FeedbackReports(val reports: List<FeedbackReport>, val cursor: String? = null)
@Serializable data class MessageVersion(val revision: Int, val content: ChatContent, val createdAt: String)
@Serializable data class MessageVersions(val messageId: String, val versions: List<MessageVersion>, val hasMore: Boolean)
@Serializable data class MessageReaction(val emoji: String, val authorIds: List<String>)
/** One person who reacted; `id` matches the snapshot's `authorIds`. */
@Serializable data class Reactor(
    val id: String, val username: String? = null, val displayName: String? = null, val avatarId: Int? = null,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
)
@Serializable data class ReactorGroup(val emoji: String, val authors: List<Reactor>)
/** Who reacted to a message, per emoji in snapshot order and people in reaction order. */
@Serializable data class ReactorList(val messageId: String, val reactionSeq: String, val reactions: List<ReactorGroup>)
@Serializable data class MessagePin(val author: ChatAuthor, val createdAt: String)
@Serializable data class MessageForward(val message: ChatMessage?, val seq: String)
@Serializable data class ForwardDestination(val id: String, val name: String, val spaceName: String, val direct: Boolean)
@Serializable data class ForwardDestinations(val destinations: List<ForwardDestination>)
@Serializable data class ForwardSource(val spaceId: String? = null, val message: ChatMessage)
@Serializable data class ForwardConversation(val root: ChatMessage?, val messages: List<ChatMessage>, val cursor: String, val hasMore: Boolean, val source: ForwardSource? = null)
@Serializable data class ForwardUpdate(val type: String, val schemaVersion: Int, val channelId: String, val seq: String, val message: ChatMessage)
@Serializable data class EditUpdate(
    val type: String,
    val schemaVersion: Int,
    val channelId: String,
    val seq: String,
    val message: ChatMessage,
)
@Serializable data class PinUpdate(
    val type: String,
    val schemaVersion: Int,
    val channelId: String,
    val seq: String,
    val message: ChatMessage,
)
@Serializable data class ReactionUpdate(
    val type: String,
    val schemaVersion: Int,
    val channelId: String,
    val seq: String,
    val messageId: String,
    val reactions: List<MessageReaction>,
)
/** `message.attachments`: the message's files after the media worker changed them. */
@Serializable data class AttachmentsUpdate(
    val type: String,
    val schemaVersion: Int,
    val channelId: String,
    val seq: String,
    val messageId: String,
    @Serializable(with = TolerantAttachmentsSerializer::class) val attachments: List<ChatAttachment> = emptyList(),
)
@Serializable data class ChatMessage(
    val id: String,
    val channelId: String,
    val seq: String,
    val author: ChatAuthor,
    val content: ChatContent,
    val createdAt: String,
    val clientMessageId: String,
    val reactions: List<MessageReaction> = emptyList(),
    val reactionSeq: String? = null,
    /** Sequence of the last `message.attachments` applied to this message. */
    val attachmentsSeq: String? = null,
    val pin: MessagePin? = null,
    val pinSeq: String? = null,
    val threadRootId: String? = null,
    val broadcast: Boolean = false,
    val thread: ThreadSummary? = null,
    val forward: MessageForward? = null,
    val forwardSeq: String? = null,
    val revision: Int = 1,
    val editedAt: String? = null,
    val editSeq: String? = null,
    val replyTo: MessageReply? = null,
)
@Serializable data class MessageReply(val id: String, val author: ChatAuthor, val text: String, val hasAttachments: Boolean, val threadRootId: String? = null, val fullText: String? = null)
@Serializable data class ThreadSummary(val replyCount: Int, val participants: List<ChatAuthor>, val seq: String)
@Serializable data class UnreadThread(val rootId: String, val seq: String)
@Serializable data class UnreadThreads(val threads: List<UnreadThread>)
/** [visitor]: read by a feedback team member who is not in the conversation (see [ChatHistory.visitor]). */
@Serializable data class ThreadHistory(val root: ChatMessage, val messages: List<ChatMessage>, val cursor: String, val hasMore: Boolean, val hasNewer: Boolean = false, val visitor: Boolean = false)
/** [failedOlder]/[failedNewer]: which page [error] is about, so Retry repeats that page (web). */
data class ThreadUi(val rootId: String, val loading: Boolean = true, val hasMore: Boolean = false, val before: String? = null, val error: String? = null,
    val hasNewer: Boolean = false, val after: String? = null, val windowStart: String? = null, val windowEnd: String? = null,
    val failedOlder: Boolean = false, val failedNewer: Boolean = false)
@Serializable data class ChatHistory(
    val messages: List<ChatMessage>,
    val cursor: String,
    val hasMore: Boolean,
    val pinnedMessages: List<ChatMessage> = emptyList(),
    val space: ChatRoom? = null,
    val channel: ChatRoom? = null,
    val hasNewer: Boolean = false,
    /** A feedback team member reading a reported conversation they are not in; absent for members. */
    val visitor: Boolean = false,
)
@Serializable data class ChatRoom(val id: String, val name: String, val direct: Boolean = false)
@Serializable data class ChatSession(val token: String, val author: ChatAuthor)
@Serializable data class ErrorBody(val error: String? = null, val code: String? = null, val attemptsRemaining: Int? = null)
@Serializable data class MemberList(val members: List<Member>, val invitations: List<Member> = emptyList())
@Serializable data class PresenceMember(val userId: String, val status: String)
@Serializable data class PresenceSnapshot(val members: List<PresenceMember>)
data class TypingAuthor(val author: ChatAuthor, val revision: Long, val expiresAt: Long, val typing: Boolean)

object IceUrlsSerializer : JsonTransformingSerializer<List<String>>(ListSerializer(String.serializer())) {
    override fun transformDeserialize(element: kotlinx.serialization.json.JsonElement) =
        if (element is JsonPrimitive && element.isString) JsonArray(listOf(element)) else element
}
@Serializable data class IceServer(
    @Serializable(with = IceUrlsSerializer::class) val urls: List<String>,
    val username: String? = null,
    val credential: String? = null,
) {
    constructor(url: String, username: String? = null, credential: String? = null) : this(listOf(url), username, credential)
}
@Serializable data class TurnGeneration(val generation: String, val refreshAfterMs: Long, val expiresInMs: Long)
@Serializable data class TurnResponse(val iceServers: List<IceServer>, val turn: TurnGeneration)
@Serializable data class JoinResponse(val token: String, val id: String, val iceServers: List<IceServer>, val turn: TurnGeneration? = null)
@Serializable data class MediaTrack(val id: String, val kind: String)
@Serializable data class Participant(
    val id: String,
    val name: String,
    val muted: Boolean,
    val deafened: Boolean,
    val tracks: List<MediaTrack>,
    val avatarId: Int? = null,
    val avatarAsset: String? = null,
    val avatarUrl: String? = null,
)
@Serializable data class MediaSnapshot(
    val participants: List<Participant>,
    val revision: Long? = null,
    val sessionStartedAt: Long? = null,
)
@Serializable data class SpectatorParticipant(
    val id: String, val name: String,
    val muted: Boolean, val deafened: Boolean, val avatarId: Int? = null,
    val avatarAsset: String? = null, val avatarUrl: String? = null,
) {
    fun asParticipant() = Participant(id, name, muted, deafened, emptyList(), avatarId, avatarAsset, avatarUrl)
}
@Serializable data class SpectatorSnapshot(
    val participants: List<SpectatorParticipant>,
    val revision: Long,
    val sessionStartedAt: Long? = null,
)
@Serializable data class SessionDescription(val type: String, val sdp: String)
@Serializable data class SignalResponse(
    val sessionDescription: SessionDescription? = null,
    val tracks: List<SignalTrack> = emptyList(),
    val requiresImmediateRenegotiation: Boolean = false,
)
@Serializable data class SignalTrack(val mid: String)
data class AudioRoute(val id: Int, val name: String)

enum class GatewayStatus { DISCONNECTED, CONNECTING, LIVE, ERROR }

sealed interface SessionScreen {
    data object Loading : SessionScreen
    data object Home : SessionScreen
    data object SignedOut : SessionScreen
    /**
     * The code step. [resends] counts new codes for this email entry (see `data/CodeResend.kt`);
     * [sentAt] is when the current code was sent, on the `SystemClock.elapsedRealtime` clock.
     */
    data class Verify(
        val challengeId: String, val email: String, val attemptsRemaining: Int? = null,
        val resends: Int = 0, val sentAt: Long = 0,
    ) : SessionScreen
    data class Profile(val account: Account) : SessionScreen
    data class Spaces(val account: Account) : SessionScreen
}

/** Web keys availability by media root: General uses the demo root. */
fun voiceRootKey(demo: Boolean, channelId: String) = if (demo) "" else channelId

/** A valid persisted v1 atlas tile, or null for old/invalid API data. */
fun caperAvatarIndex(avatarId: Int?): Int? = avatarId?.takeIf { it in 0..799 }

/**
 * A custom profile picture or space icon. [asset] is stable until the picture changes and keys
 * every cache; the signed [url] re-signs daily, so it never does.
 */
data class Picture(val asset: String, val url: String)

/**
 * The picture to show, or null to keep the saved mascot / space initial: old servers send
 * neither field, and a server without CDN signing sends the asset without a URL.
 */
fun pictureOf(asset: String?, url: String?): Picture? {
    if (asset.isNullOrBlank() || url.isNullOrBlank() || asset.length > 64 || asset.any { it.isWhitespace() || it.isISOControl() }) return null
    if (!url.startsWith("https://") && !url.startsWith("http://")) return null
    return Picture(asset, url)
}

/**
 * Your current picture for a retained snapshot of you ([asset], [url]): the account's, keeping
 * the snapshot's URL when it names the same asset (it may be signed more recently), and none
 * once you removed it, which brings back the same saved mascot.
 */
internal fun ownPicture(own: Account, asset: String?, url: String?): Pair<String?, String?> = when {
    own.avatarAsset == null -> null to null
    asset == own.avatarAsset && url != null -> asset to url
    else -> own.avatarAsset to own.avatarUrl
}

/** Web's Join tooltip while availability is unknown or false; null once available. */
fun voiceJoinUnavailableLabel(available: Boolean?): String? = when (available) {
    true -> null
    false -> "Joining is not available at this time."
    null -> "Checking voice availability…"
}

data class AppUiState(
    val screen: SessionScreen = SessionScreen.Loading,
    /**
     * Home is open but its first space and conversation are still loading. The brand loading
     * screen stays up meanwhile, so launch lands on the restored conversation instead of
     * flashing Browse and an empty stage. Capped by `RESTORE_LIMIT_MS`.
     */
    val restoring: Boolean = false,
    val account: Account? = null,
    val spaces: List<Space> = emptyList(),
    val unreadSpaces: List<UnreadSpace> = emptyList(),
    val feedbackWaiting: Int = 0,
    val unreadThreads: Map<String, String> = emptyMap(),
    val invitations: List<Space> = emptyList(),
    val notificationInvitation: Space? = null,
    val notificationBrowseSpace: String? = null,
    val limits: SpaceLimits? = null,
    val selectedSpace: SpaceDetail? = null,
    val selectedChannel: Channel? = null,
    val directConversations: List<DirectConversation> = emptyList(),
    /** DM mention and space-invite/new-DM username candidates from `GET /api/people`; null until the first load succeeds. */
    val people: List<Person>? = null,
    /** Accounts you blocked, newest first; their messages collapse everywhere. */
    val blocks: List<BlockedAccount> = emptyList(),
    val blocksError: String? = null,
    /** The blocked-accounts list loaded at least once, so an empty list means none. */
    val blocksLoaded: Boolean = false,
    /**
     * The sidebar's "Message requests" list is expanded. Null follows the view (open
     * while a request is shown); a tap stores the explicit choice, which wins.
     */
    val requestsOpen: Boolean? = null,
    /** Notification settings with unsaved changes applied; null until they load. */
    val notificationSettings: NotificationSettings? = null,
    /** Why notification settings could not load, while none are shown. */
    val notificationSettingsError: String? = null,
    /** Failed notification saves by setting key (see `data/Notifications.kt`), shown beside that control. */
    val notificationErrors: Map<String, String> = emptyMap(),
    val selectedDirectId: String? = null,
    /** Immutable unread boundary captured before this channel visit acknowledges history. */
    val channelVisitUnread: UnreadChannel? = null,
    val messages: List<ChatMessage> = emptyList(),
    val thread: ThreadUi? = null,
    val threadOnlyRows: Set<String> = emptySet(),
    val hasMoreMessages: Boolean = false,
    val hasNewerMessages: Boolean = false,
    val loadingNewer: Boolean = false,
    val contextStart: String? = null,
    val contextEnd: String? = null,
    val focusedMessageId: String? = null,
    val focusRevision: Long = 0,
    val loadingMessageContext: Boolean = false,
    val messageContextError: String? = null,
    val loadingOlder: Boolean = false,
    val olderError: String? = null,
    /** Web's chat phases: first history page loading, or failed with no messages to show. */
    val messagesLoading: Boolean = false,
    val messagesError: String? = null,
    /** Web's sidebar error for a space that failed to open, with Retry opening. */
    val openError: String? = null,
    /** Web's chat session error above the composer, with Retry session. */
    val sessionError: String? = null,
    /** A resync that failed while earlier messages stay visible. */
    val refreshError: String? = null,
    val typingAuthors: List<ChatAuthor> = emptyList(),
    val presence: Map<String, String> = emptyMap(),
    val voiceRosters: Map<String, List<Participant>> = emptyMap(),
    val voiceSessionStartedAt: Map<String, Long> = emptyMap(),
    val deniedVoiceChannels: Set<String> = emptySet(),
    /** Voice availability by media root ("" is General's demo root); absent while checking. */
    val voiceAvailability: Map<String, Boolean> = emptyMap(),
    val presencePage: Int = 0,
    /** The private channel [channelGrants] and [pendingChannelInvitations] belong to; read them with [channelGrantsFor]. */
    val channelGrantsChannelId: String? = null,
    val channelGrants: List<Member> = emptyList(),
    val pendingChannelInvitations: List<Member> = emptyList(),
    val pendingSpaceInvitations: List<Member> = emptyList(),
    /** The open space's live invite links (owner only), newest first. */
    val inviteLinks: List<InviteLink> = emptyList(),
    val pendingMessage: PendingMessageUi? = null,
    val replyTargets: Map<String, MessageReply> = emptyMap(),
    val gateway: GatewayStatus = GatewayStatus.DISCONNECTED,
    val reactionSaves: Map<String, ReactionSaveUi> = emptyMap(),
    val pinnedMessages: List<ChatMessage> = emptyList(),
    val pinSaves: Map<String, PinSaveUi> = emptyMap(),
    val pinIntents: Map<String, PinIntentUi> = emptyMap(),
    val editIntents: Map<String, EditIntentUi> = emptyMap(),
    val chatAuthorId: String? = null,
    /** `GET /api/assets/usage` succeeded: show the attach control. */
    val uploadsEnabled: Boolean = false,
    /** A usage check has answered, so a disabled attach control is unavailable rather than still checking. */
    val uploadsChecked: Boolean = false,
    /** Files stay with their channel/DM and, for replies, their thread, even while closed. */
    val attachmentDrafts: Map<AttachmentDraftTarget, List<DraftAttachmentUi>> = emptyMap(),
    val attachmentErrors: Map<AttachmentDraftTarget, String> = emptyMap(),
    /** Re-signed URLs for long-open conversations, keyed by attachment ID. */
    val freshAttachmentUrls: Map<String, AttachmentUrls> = emptyMap(),
    /** Latest server processing percent (`attachment.progress`), keyed by attachment ID. */
    val attachmentProgress: Map<String, Int> = emptyMap(),
    /** This device's picked content for files it sent, shown while the server processes them. */
    val localAttachmentPreviews: Map<String, String> = emptyMap(),
    /** Feedback team: the inbox's reports (kept while you visit one). */
    val feedback: FeedbackInboxUi = FeedbackInboxUi(),
    /** Feedback team: the inbox is the open destination, in place of a conversation. */
    val feedbackShown: Boolean = false,
    /** Feedback team: the open conversation is visited, not joined (see [VisitUi]). */
    val visit: VisitUi? = null,
    /** Ask Android 13+ for notification permission now; set once per account by app open. */
    val pushPrompt: Boolean = false,
    /** Your picture ([PROFILE_PICTURE_KEY]) or a space's icon (by space ID) being changed, or why that failed. */
    val pictureSaves: Map<String, PictureSaveUi> = emptyMap(),
    /** Counts live `profile.updated` events, so views holding their own loaded copies (who reacted) refresh them. */
    val profileRevision: Long = 0,
    val busy: Boolean = false,
    val error: String? = null,
) {
    val drafts: List<DraftAttachmentUi> get() = selectedChannel?.id?.let { attachmentDrafts[AttachmentDraftTarget(it)] }.orEmpty()
    val attachmentError: String? get() = selectedChannel?.id?.let { attachmentErrors[AttachmentDraftTarget(it)] }

    /** [channelId]'s members and pending invitations; another channel's (or none loaded yet) read as empty. */
    fun channelGrantsFor(channelId: String): Pair<List<Member>, List<Member>> =
        if (channelGrantsChannelId == channelId) channelGrants to pendingChannelInvitations else emptyList<Member>() to emptyList()

    /** Applies [change] to [channelId]'s grants only while they are the ones loaded, so a late reply can't land on another channel. */
    fun withChannelGrants(channelId: String, change: (members: List<Member>, invitations: List<Member>) -> Pair<List<Member>, List<Member>>): AppUiState {
        if (channelGrantsChannelId != channelId) return this
        val (members, invitations) = change(channelGrants, pendingChannelInvitations)
        return copy(channelGrants = members, pendingChannelInvitations = invitations)
    }

    /**
     * Resolve retained identity snapshots from the account without touching conversation state:
     * your saved name and your current picture wherever you appear ([withPerson] for your own
     * ID, keeping a snapshot's URL when it already names your picture).
     */
    fun withAccountProfile(): AppUiState {
        val own = account ?: return this
        if (own.username == null || own.displayName == null) return this
        return withPerson(own.profile(), keepSignedUrls = true)
    }

    /** Local presentation never enters history, replay cursors or revision caches. */
    private fun project(message: ChatMessage): ChatMessage {
        var result = message
        pinIntents[message.id]?.let { result = result.copy(pin = it.pin) }
        editIntents[message.id]?.takeIf { message.revision <= it.expectedRevision }?.let {
            result = result.copy(content = message.content.copy(text = it.text, mentions = emptyList()))
        }
        return result
    }
    val displayedMessages: List<ChatMessage> get() = messages.map(::project)
    val displayedPins: List<ChatMessage> get() {
        val rows = pinnedMessages.associateBy { it.id }.toMutableMap()
        pinIntents.forEach { (id, intent) ->
            if (intent.pin == null) rows.remove(id)
            else rows[id] = messages.firstOrNull { it.id == id } ?: rows[id] ?: intent.message
        }
        return rows.values.map(::project).sortedByDescending { it.pinSeq?.toBigIntegerOrNull() }
    }

    /** Each channel's media root decides its own stable sidebar Join action. */
    fun voiceAvailable(channel: Channel): Boolean? =
        voiceAvailability[voiceRootKey(selectedSpace?.space?.demo == true && visit?.channelId != channel.id, channel.id)]

    val voiceAvailable: Boolean? get() = selectedChannel?.let(::voiceAvailable)

    val blockedIds: Set<String> get() = blocks.mapTo(HashSet()) { it.id }

    val channelMessages: List<ChatMessage> get() = messages.filter {
        (it.threadRootId == null || it.broadcast) && it.id !in threadOnlyRows &&
            (contextStart == null || it.seq.toBigInteger() >= contextStart.toBigInteger()) &&
            (contextEnd == null || it.seq.toBigInteger() <= contextEnd.toBigInteger())
    }
    val displayedChannelMessages: List<ChatMessage> get() = channelMessages.map(::project)

    /** You can write, react, pin and edit here: a joined channel, not a message request still waiting
     * for your answer, and not a DM with someone you blocked (the server refuses those too). */
    val canParticipate: Boolean get() = selectedChannel?.joined == true && selectedDirect?.let {
        it.incoming || it.blocked || it.peer.id in blockedIds
    } != true

    /** The open DM, when one is selected. */
    val selectedDirect: DirectConversation? get() = selectedDirectId?.let { id -> directConversations.firstOrNull { it.id == id } }
}

/** The Feedback inbox (feedback team only): loaded reports, newest first, and the next page's cursor. */
data class FeedbackInboxUi(
    val reports: List<FeedbackReport> = emptyList(),
    val cursor: String? = null,
    val loaded: Boolean = false,
    val loading: Boolean = false,
    val loadingMore: Boolean = false,
    val error: String? = null,
    /** Shows archived threads (`?archived=true`) instead of the inbox. */
    val archived: Boolean = false,
)

/**
 * The open conversation is a feedback visit: a team member reading a reported channel or DM
 * that is not in their lists. [spaceId] and [spaceName] are null for a DM, whose
 * [channelName] names both people ("Alice & Bob"). [visitor] is the server's flag (false
 * when the team member turns out to be in the conversation after all). [reporter] is known
 * when it was opened from the inbox, for mention suggestions.
 */
data class VisitUi(
    val channelId: String,
    val spaceId: String?,
    val spaceName: String?,
    val channelName: String,
    val direct: Boolean,
    val visitor: Boolean = true,
    val reporter: FeedbackReporter? = null,
)

data class PendingMessageUi(
    val clientMessageId: String,
    val text: String,
    val author: ChatAuthor?,
    val createdAt: String,
    val error: String? = null,
    val rejected: Boolean = false,
    val uploadFailed: Boolean = false,
    val threadRootId: String? = null,
    val broadcast: Boolean = false,
    /** Uploaded files with local `file://` copies until the server confirms. */
    val attachments: List<ChatAttachment> = emptyList(),
    val replyTo: MessageReply? = null,
)

/** A regular composer and each of its threads own independent files and upload errors. */
data class AttachmentDraftTarget(val channelId: String, val threadRootId: String? = null)

/** A file in the composer: preparing (compressing), uploading, ready to send, or failed. */
data class DraftAttachmentUi(
    val key: String,
    val name: String,
    val kind: String,
    val sourceSize: Long = 0,
    /** Bytes actually uploaded, once compression finished. */
    val storedSize: Long? = null,
    /** Local `file://` thumbnail (the stored image or its preview, a poster for video). */
    val thumbnail: String? = null,
    /** Local `file://` copy of the bytes being uploaded, shown by the pending message. */
    val localUrl: String? = null,
    /** The selected file URI, granted to the system opener for draft previews. */
    val pickedUri: String? = null,
    /** The picked `content://` image, kept for the (dormant) server-processing placeholder. */
    val pickedImage: String? = null,
    val compressing: Boolean = true,
    val progress: Float = 0f,
    val error: String? = null,
    val attachment: ChatAttachment? = null,
    val uploadedAt: Long? = null,
)

/** The [AppUiState.pictureSaves] key for your own profile picture (space icons use the space ID). */
const val PROFILE_PICTURE_KEY = "avatar"

/** A picture change in flight, [uploading] a new one or [removing] it, or the inline [error] it ended with. */
data class PictureSaveUi(val uploading: Boolean = false, val removing: Boolean = false, val error: String? = null) {
    val busy: Boolean get() = uploading || removing
}

data class ReactionSaveUi(val emoji: String, val active: Boolean, val saving: Boolean = true, val error: String? = null)
data class PinSaveUi(val active: Boolean, val saving: Boolean = true, val error: String? = null)
data class PinIntentUi(val message: ChatMessage, val pin: MessagePin?)
data class EditIntentUi(val text: String, val expectedRevision: Int)
