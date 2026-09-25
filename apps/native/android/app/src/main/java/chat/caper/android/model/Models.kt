package chat.caper.android.model

import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonTransformingSerializer

@Serializable data class Account(val id: String, val username: String? = null, val displayName: String? = null, val debugEnabled: Boolean = false)
@Serializable data class Challenge(val challengeId: String)
@Serializable data class VerifyResult(val account: Account, val token: String)
@Serializable data class Space(val id: String, val name: String, val ownerId: String = "", val demo: Boolean = false)
@Serializable data class SpaceList(val spaces: List<Space>, val limits: SpaceLimits)
@Serializable data class SpaceLimits(val ownedSpaces: Int, val totalSpaces: Int, val channelsPerSpace: Int)
@Serializable data class Channel(val id: String, val spaceId: String, val name: String, val private: Boolean)
@Serializable data class Member(val id: String, val username: String, val displayName: String, val owner: Boolean)
@Serializable data class SpaceDetail(val space: Space, val channels: List<Channel>, val members: List<Member>)
@Serializable data class ChatAuthor(val id: String, val name: String, val isGuest: Boolean)
@Serializable data class ChatContent(val version: Int, val type: String, val text: String)
@Serializable data class ChatMessage(
    val id: String,
    val channelId: String,
    val seq: String,
    val author: ChatAuthor,
    val content: ChatContent,
    val createdAt: String,
    val clientMessageId: String,
)
@Serializable data class ChatHistory(
    val messages: List<ChatMessage>,
    val cursor: String,
    val hasMore: Boolean,
    val space: ChatRoom? = null,
    val channel: ChatRoom? = null,
)
@Serializable data class ChatRoom(val id: String, val name: String)
@Serializable data class ChatSession(val token: String, val author: ChatAuthor)
@Serializable data class ErrorBody(val error: String? = null, val code: String? = null, val attemptsRemaining: Int? = null)
@Serializable data class MemberList(val members: List<Member>)
@Serializable data class PresenceMember(val userId: String, val status: String)
@Serializable data class PresenceSnapshot(val members: List<PresenceMember>)
data class TypingAuthor(val author: ChatAuthor, val revision: Long, val expiresAt: Long)

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
    val countryCode: String? = null,
    val muted: Boolean,
    val deafened: Boolean,
    val tracks: List<MediaTrack>,
)
@Serializable data class MediaSnapshot(val participants: List<Participant>, val revision: Long? = null)
@Serializable data class SpectatorParticipant(
    val id: String, val name: String, val countryCode: String? = null,
    val muted: Boolean, val deafened: Boolean,
) {
    fun asParticipant() = Participant(id, name, countryCode, muted, deafened, emptyList())
}
@Serializable data class SpectatorSnapshot(val participants: List<SpectatorParticipant>, val revision: Long)
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

data class AppUiState(
    val screen: SessionScreen = SessionScreen.Loading,
    val account: Account? = null,
    val spaces: List<Space> = emptyList(),
    val limits: SpaceLimits? = null,
    val selectedSpace: SpaceDetail? = null,
    val selectedChannel: Channel? = null,
    val messages: List<ChatMessage> = emptyList(),
    val hasMoreMessages: Boolean = false,
    val loadingOlder: Boolean = false,
    val olderError: String? = null,
    val typingAuthors: List<ChatAuthor> = emptyList(),
    val presence: Map<String, String> = emptyMap(),
    val voiceRosters: Map<String, List<Participant>> = emptyMap(),
    val deniedVoiceChannels: Set<String> = emptySet(),
    val presencePage: Int = 0,
    val channelGrants: List<Member> = emptyList(),
    val pendingMessage: PendingMessageUi? = null,
    val gateway: GatewayStatus = GatewayStatus.DISCONNECTED,
    val busy: Boolean = false,
    val error: String? = null,
)

data class PendingMessageUi(
    val clientMessageId: String,
    val text: String,
    val author: ChatAuthor?,
    val createdAt: String,
    val error: String? = null,
    val rejected: Boolean = false,
)
