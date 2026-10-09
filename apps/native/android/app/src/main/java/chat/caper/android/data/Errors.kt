package chat.caper.android.data

import java.io.IOException
import java.io.InterruptedIOException
import kotlinx.coroutines.TimeoutCancellationException

/** Web's `spaces/errors.ts` sentences for the API's lowercase error text. Keys are the server's exact strings. */
private val serverErrors = mapOf(
    "user not found" to "User not found. Check the username and try again.",
    "account not found" to "User not found. Check the username and try again.",
    "enter an exact username" to "Enter an exact username.",
    "invalid username" to "Use 3–32 lowercase letters, numbers, or underscores.",
    "user already in space" to "This person is already in the space.",
    "user already in channel" to "This person already has access to this channel.",
    "user already invited" to "This person already has a pending invitation.",
    "user must join the space first" to "This person needs to join the space before you can add them to a channel.",
    "invitation cooldown; try again after 24 hours" to
        "This person recently responded to an invitation. You can invite them again after 24 hours.",
    "too many invitation attempts; try again in 10 minutes" to "Too many invitations. Try again in 10 minutes.",
    "pending invitation limit reached" to "Too many invitations are waiting for a response. Try again later.",
    "membership limit reached" to "You’ve reached the limit of spaces you can join. Leave one to join this space.",
    "space limit reached" to "You’ve reached your space limit.",
    "channel limit reached" to "This space has reached its channel limit.",
    "channel name already exists" to "A channel with that name already exists.",
    "invalid channel name" to "Use lowercase letters separated by single dashes.",
    "invalid space name" to "Enter a space name up to 80 characters.",
    "owner cannot be removed" to "The space owner can’t be removed.",
    "public channels are self-joined" to "Anyone in the space can join a public channel without an invitation.",
    "resource not found" to "That’s no longer available.",
    "channel not found" to "This channel is no longer available.",
    "conversation not found" to "This conversation is no longer available.",
    "request not found" to "This message request is no longer available.",
    "you can't block yourself" to "You can’t block yourself.",
    "too many blocked accounts" to "You’ve blocked the maximum number of accounts.",
    "complete profile required" to "Finish your profile first.",
    "unauthorized" to "You’re signed out. Sign in again to continue.",
    "spaces unavailable" to "Caper is having trouble right now. Try again in a moment.",
    "messages unavailable" to "Messages are unavailable right now. Try again in a moment.",
)

private val clauseBreak = Regex(";\\s*")

/** Capitalizes lowercase server text and ends it as a sentence: "a; b" reads "A. B." */
internal fun errorSentence(message: String): String {
    val text = message.trim().split(clauseBreak).filter { it.isNotEmpty() }
        .joinToString(". ") { part -> part.replaceFirstChar { it.uppercase() } }
    return if (text.lastOrNull()?.let { it in ".!?…" } == true) text else "$text."
}

/**
 * Readable text for anything a request can throw, for display only: logic keeps
 * comparing the exception's own status, code and message. A blank message reads [fallback].
 */
internal fun friendlyError(error: Throwable?, fallback: String = "That didn’t work. Try again."): String {
    // OkHttp's call timeout and socket timeouts are InterruptedIOExceptions.
    if (error is InterruptedIOException || error is TimeoutCancellationException) return "That took too long. Try again."
    // ApiException is an IOException too, but it carries the server's answer.
    if (error is IOException && error !is ApiException) return "Couldn’t reach Caper. Check your connection."
    val message = error?.message
    if (message.isNullOrBlank()) return fallback
    return serverErrors[message] ?: errorSentence(message)
}
