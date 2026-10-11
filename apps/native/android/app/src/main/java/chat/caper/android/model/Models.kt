package chat.caper.android.model

import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
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
@Serializable data class DirectPeer(val id: String, val username: String, val displayName: String, val avatarId: Int? = null)
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
@Serializable data class BlockedAccount(val id: String, val username: String, val displayName: String, val avatarId: Int? = null)
@Serializable data class BlockList(val blocks: List<BlockedAccount>)
/** `GET/PUT /api/account/privacy`: `anyone`, `spaces` or `nobody`. */
@Serializable data class DirectPrivacy(val directMessages: String)
@Serializable data class DirectConversationList(val conversations: List<DirectConversation>)
/** `GET /api/people`: accounts sharing a space or a DM with you (never you), by username. */
@Serializable data class Person(val id: String, val username: String, val displayName: String, val avatarId: Int? = null)
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
@Serializable data class Member(val id: String, val username: String, val displayName: String, val owner: Boolean, val avatarId: Int? = null)
@Serializable data class SpaceDetail(
    val space: Space,
    val channels: List<Channel>,
    val members: List<Member>,
    val channelInvitations: List<ChannelInvitation> = emptyList(),
)
@Serializable data class ChatAuthor(val id: String, val name: String, val isGuest: Boolean, val avatarId: Int? = null)
/**
 * One `content.mentions` entry: `user` (with `id` and `username`), `everyone` or `here`.
 * Other types decode too and are ignored where mentions are used.
 */
@Serializable data class MessageMention(val type: String, val id: String? = null, val username: String? = null)
@Serializable data class ChatContent(
    val version: Int,
    val type: String,
    val text: String,
    /** Absent on older messages and servers. */
    @Serializable(with = MentionListSerializer::class) val mentions: List<MessageMention> = emptyList(),
)

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
@Serializable data class MessageVersion(val revision: Int, val content: ChatContent, val createdAt: String)
@Serializable data class MessageVersions(val messageId: String, val versions: List<MessageVersion>, val hasMore: Boolean)
@Serializable data class MessageReaction(val emoji: String, val authorIds: List<String>)
/** One person who reacted; `id` matches the snapshot's `authorIds`. */
@Serializable data class Reactor(val id: String, val username: String? = null, val displayName: String? = null, val avatarId: Int? = null)
@Serializable data class ReactorGroup(val emoji: String, val authors: List<Reactor>)
/** Who reacted to a message, per emoji in snapshot order and people in reaction order. */
@Serializable data class ReactorList(val messageId: String, val reactionSeq: String, val reactions: List<ReactorGroup>)
@Serializable data class MessagePin(val author: ChatAuthor, val createdAt: String)
@Serializable data class MessageForward(val message: ChatMessage?, val seq: String)
@Serializable data class ForwardDestination(val id: String, val name: String, val spaceName: String, val direct: Boolean)
@Serializable data class ForwardDestinations(val destinations: List<ForwardDestination>)
@Serializable data class ForwardConversation(val root: ChatMessage?, val messages: List<ChatMessage>, val cursor: String, val hasMore: Boolean)
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
)
@Serializable data class ThreadSummary(val replyCount: Int, val participants: List<ChatAuthor>, val seq: String)
@Serializable data class ThreadHistory(val root: ChatMessage, val messages: List<ChatMessage>, val cursor: String, val hasMore: Boolean, val hasNewer: Boolean = false)
data class ThreadUi(val rootId: String, val loading: Boolean = true, val hasMore: Boolean = false, val before: String? = null, val error: String? = null,
    val hasNewer: Boolean = false, val after: String? = null, val windowStart: String? = null, val windowEnd: String? = null)
@Serializable data class ChatHistory(
    val messages: List<ChatMessage>,
    val cursor: String,
    val hasMore: Boolean,
    val pinnedMessages: List<ChatMessage> = emptyList(),
    val space: ChatRoom? = null,
    val channel: ChatRoom? = null,
    val hasNewer: Boolean = false,
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
    val invitations: List<Space> = emptyList(),
    val limits: SpaceLimits? = null,
    val selectedSpace: SpaceDetail? = null,
    val selectedChannel: Channel? = null,
    val directConversations: List<DirectConversation> = emptyList(),
    /** DM mention candidates from `GET /api/people`; null until the first load succeeds. */
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
    val channelGrants: List<Member> = emptyList(),
    val pendingChannelInvitations: List<Member> = emptyList(),
    val pendingSpaceInvitations: List<Member> = emptyList(),
    val pendingMessage: PendingMessageUi? = null,
    val gateway: GatewayStatus = GatewayStatus.DISCONNECTED,
    val reactionSaves: Map<String, ReactionSaveUi> = emptyMap(),
    val pinnedMessages: List<ChatMessage> = emptyList(),
    val pinSaves: Map<String, PinSaveUi> = emptyMap(),
    val pinIntents: Map<String, PinIntentUi> = emptyMap(),
    val editIntents: Map<String, EditIntentUi> = emptyMap(),
    val chatAuthorId: String? = null,
    /** Ask Android 13+ for notification permission now; set once per account by app open. */
    val pushPrompt: Boolean = false,
    val busy: Boolean = false,
    val error: String? = null,
) {
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
        voiceAvailability[voiceRootKey(selectedSpace?.space?.demo == true, channel.id)]

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

data class PendingMessageUi(
    val clientMessageId: String,
    val text: String,
    val author: ChatAuthor?,
    val createdAt: String,
    val error: String? = null,
    val rejected: Boolean = false,
    val threadRootId: String? = null,
    val broadcast: Boolean = false,
)

data class ReactionSaveUi(val emoji: String, val active: Boolean, val saving: Boolean = true, val error: String? = null)
data class PinSaveUi(val active: Boolean, val saving: Boolean = true, val error: String? = null)
data class PinIntentUi(val message: ChatMessage, val pin: MessagePin?)
data class EditIntentUi(val text: String, val expectedRevision: Int)
