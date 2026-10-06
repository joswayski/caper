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
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonTransformingSerializer

@Serializable data class MediaStatus(val enabled: Boolean = false)
@Serializable data class Account(val id: String, val username: String? = null, val displayName: String? = null, val debugEnabled: Boolean = false, val avatarId: Int? = null)
@Serializable data class Challenge(val challengeId: String)
@Serializable data class VerifyResult(val account: Account, val token: String)
@Serializable data class Inviter(val username: String, val displayName: String)
@Serializable data class Space(val id: String, val name: String, val ownerId: String = "", val demo: Boolean = false, val inviter: Inviter? = null)
@Serializable data class SpaceList(val spaces: List<Space>, val invitations: List<Space> = emptyList(), val limits: SpaceLimits)
@Serializable data class SpaceLimits(val ownedSpaces: Int, val totalSpaces: Int, val channelsPerSpace: Int)
@Serializable data class Channel(
    val id: String,
    val spaceId: String,
    val name: String,
    val private: Boolean,
    val direct: Boolean = false,
    val joined: Boolean = true,
)
@Serializable data class DirectPeer(val id: String, val username: String, val displayName: String)
@Serializable data class DirectConversation(val id: String, val peer: DirectPeer, val lastSeq: String, val readSeq: String)
@Serializable data class DirectConversationList(val conversations: List<DirectConversation>)
@Serializable data class PushConfig(val platforms: List<String>)
@Serializable data class ChannelInvitation(val channel: Channel, val inviter: Inviter)
@Serializable data class Member(val id: String, val username: String, val displayName: String, val owner: Boolean, val avatarId: Int? = null)
@Serializable data class SpaceDetail(
    val space: Space,
    val channels: List<Channel>,
    val members: List<Member>,
    val channelInvitations: List<ChannelInvitation> = emptyList(),
)
@Serializable data class ChatAuthor(val id: String, val name: String, val isGuest: Boolean, val avatarId: Int? = null)
@Serializable data class ChatContent(
    val version: Int,
    val type: String,
    val text: String,
    /** Additive file list. Malformed entries are dropped, never the message. */
    @Serializable(with = TolerantAttachmentsSerializer::class) val attachments: List<ChatAttachment> = emptyList(),
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

/** `GET /api/assets/usage`. Unknown fields (such as retired client compression settings) are ignored. */
@Serializable data class AssetUsage(val used: Long, val limit: Long, val maxUploadBytes: Long? = null)
@Serializable data class PresignedUpload(val method: String = "PUT", val url: String, val headers: Map<String, String> = emptyMap())
@Serializable data class AssetReservation(val id: String, val kind: String = "file", val upload: PresignedUpload)
@Serializable data class MessageReaction(val emoji: String, val authorIds: List<String>)
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
)
@Serializable data class ChatHistory(
    val messages: List<ChatMessage>,
    val cursor: String,
    val hasMore: Boolean,
    val space: ChatRoom? = null,
    val channel: ChatRoom? = null,
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
)
@Serializable data class MediaSnapshot(
    val participants: List<Participant>,
    val revision: Long? = null,
    val sessionStartedAt: Long? = null,
)
@Serializable data class SpectatorParticipant(
    val id: String, val name: String,
    val muted: Boolean, val deafened: Boolean, val avatarId: Int? = null,
) {
    fun asParticipant() = Participant(id, name, muted, deafened, emptyList(), avatarId)
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
    data class Verify(val challengeId: String, val email: String, val attemptsRemaining: Int? = null) : SessionScreen
    data class Profile(val account: Account) : SessionScreen
    data class Spaces(val account: Account) : SessionScreen
}

/** Web keys availability by media root: General uses the demo root. */
fun voiceRootKey(demo: Boolean, channelId: String) = if (demo) "" else channelId

/** A valid persisted v1 atlas tile, or null for old/invalid API data. */
fun caperAvatarIndex(avatarId: Int?): Int? = avatarId?.takeIf { it in 0..799 }

/** Web's Join tooltip while availability is unknown or false; null once available. */
fun voiceJoinUnavailableLabel(available: Boolean?): String? = when (available) {
    true -> null
    false -> "Joining is not available at this time."
    null -> "Checking voice availability…"
}

data class AppUiState(
    val screen: SessionScreen = SessionScreen.Loading,
    val account: Account? = null,
    val spaces: List<Space> = emptyList(),
    val invitations: List<Space> = emptyList(),
    val limits: SpaceLimits? = null,
    val selectedSpace: SpaceDetail? = null,
    val selectedChannel: Channel? = null,
    val directConversations: List<DirectConversation> = emptyList(),
    val selectedDirectId: String? = null,
    val messages: List<ChatMessage> = emptyList(),
    val hasMoreMessages: Boolean = false,
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
    val channelGrants: List<Member> = emptyList(),
    val pendingChannelInvitations: List<Member> = emptyList(),
    val pendingSpaceInvitations: List<Member> = emptyList(),
    val pendingMessage: PendingMessageUi? = null,
    val gateway: GatewayStatus = GatewayStatus.DISCONNECTED,
    val reactionSaves: Map<String, ReactionSaveUi> = emptyMap(),
    val chatAuthorId: String? = null,
    /** `GET /api/assets/usage` succeeded: show the attach control. */
    val uploadsEnabled: Boolean = false,
    val drafts: List<DraftAttachmentUi> = emptyList(),
    val attachmentError: String? = null,
    /** Re-signed URLs for long-open conversations, keyed by attachment ID. */
    val freshAttachmentUrls: Map<String, AttachmentUrls> = emptyMap(),
    /** Latest server processing percent (`attachment.progress`), keyed by attachment ID. */
    val attachmentProgress: Map<String, Int> = emptyMap(),
    /** This device's picked content for files it sent, shown while the server processes them. */
    val localAttachmentPreviews: Map<String, String> = emptyMap(),
    val busy: Boolean = false,
    val error: String? = null,
) {
    /** Each channel's media root decides its own stable sidebar Join action. */
    fun voiceAvailable(channel: Channel): Boolean? =
        voiceAvailability[voiceRootKey(selectedSpace?.space?.demo == true, channel.id)]

    val voiceAvailable: Boolean? get() = selectedChannel?.let(::voiceAvailable)
}

data class PendingMessageUi(
    val clientMessageId: String,
    val text: String,
    val author: ChatAuthor?,
    val createdAt: String,
    val error: String? = null,
    val rejected: Boolean = false,
    /** Uploaded files, shown from the picked content until the server confirms. */
    val attachments: List<ChatAttachment> = emptyList(),
)

/** A file in the composer: uploading, ready to send, or failed. */
data class DraftAttachmentUi(
    val key: String,
    val name: String,
    val kind: String,
    val size: Long = 0,
    /** The picked `content://` image itself, shown as the chip thumbnail and by the pending message. */
    val thumbnail: String? = null,
    val progress: Float = 0f,
    val error: String? = null,
    val attachment: ChatAttachment? = null,
)

data class ReactionSaveUi(val emoji: String, val active: Boolean, val saving: Boolean = true, val error: String? = null)
