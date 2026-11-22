package chat.caper.android.data

import java.io.IOException

/** Every client's automatic resend schedule for an unconfirmed send: 3, 10 and 30 seconds, then Retry send. */
internal val SEND_RETRY_DELAYS_MS = listOf(3_000L, 10_000L, 30_000L)

/** The wait before automatic retry number [attempt] (from 0), or null once they are used up. */
internal fun sendRetryDelayMs(attempt: Int): Long? = SEND_RETRY_DELAYS_MS.getOrNull(attempt)

/**
 * Whether a failed send or its upload may succeed unchanged later: a network error, a timeout, or the server or
 * storage failing on its side (5xx, 429). A refusal (other 4xx) and a file problem never retry by themselves.
 */
internal fun transientSendFailure(error: Throwable): Boolean = when (error) {
    is ApiException -> classifySendFailure(error.status) == SendFailure.UNKNOWN
    is UploadException -> error.transient
    is IOException -> true
    else -> false
}

/**
 * Where a channel's queued messages go when the conversation closes before they are sent: back into its composer
 * after the text already there, one per paragraph, so nothing typed is lost.
 */
internal fun restoredDraft(existing: String, queued: List<String>): String =
    (listOf(existing) + queued).filter { it.isNotBlank() }.joinToString("\n\n")
