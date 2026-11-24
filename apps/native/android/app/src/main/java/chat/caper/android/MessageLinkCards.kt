package chat.caper.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.messageLink
import chat.caper.android.data.messageLinkRanges
import chat.caper.android.model.ForwardSource
import chat.caper.android.ui.Border
import chat.caper.android.ui.TextMuted
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay

/** How rows resolve and open message links; CaperApp provides the signed-in account's. */
internal class MessageLinks(
    val resolve: suspend (channel: String, message: String) -> ForwardSource?,
    val open: (url: String) -> Unit,
)
internal val LocalMessageLinks = staticCompositionLocalOf<MessageLinks?> { null }

/** Cards refresh this often while shown, so edits, reactions and deletions follow the original. */
private const val REFRESH_MS = 30_000L

/** Message links in [text], up to five distinct, like link previews. */
internal fun messageLinksIn(text: String): List<Pair<String, String>> =
    messageLinkRanges(text).mapNotNull { messageLink(it.href) }.distinctBy { it.second }.take(5)

/**
 * A live card of each message linked in [text] for readers the API lets see it (author, text,
 * files, embeds, Jump to message), and a neutral "Message you can’t view" for anyone else.
 */
@Composable internal fun MessageLinkCards(text: String) {
    val links = LocalMessageLinks.current ?: return
    remember(text) { messageLinksIn(text) }.forEach { (channel, message) ->
        // Loading and failures keep just the link in the text; nothing claims anything yet.
        var resolved by remember(channel, message) { mutableStateOf<Result<ForwardSource?>?>(null) }
        LaunchedEffect(channel, message) {
            while (true) {
                try { resolved = Result.success(links.resolve(channel, message)) }
                catch (error: CancellationException) { throw error }
                catch (error: Throwable) { if (resolved == null) resolved = Result.failure(error) }
                delay(REFRESH_MS)
            }
        }
        val value = resolved?.getOrNull()
        val url = chat.caper.android.data.messageLinkUrl(channel, message)
        when {
            resolved == null || resolved?.isFailure == true -> {}
            value == null -> TextButton({ links.open(url) }, Modifier.padding(start = 62.dp), shape = MaterialTheme.shapes.small) {
                Text("Message you can’t view", color = TextMuted, fontSize = 12.sp)
            }
            else -> Surface(Modifier.padding(start = 62.dp, end = 18.dp, bottom = 8.dp).fillMaxWidth(), shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Linked message", color = TextMuted, fontSize = 11.sp)
                    if (value.message.deletedAt != null) Text("This message was deleted.", color = TextMuted)
                    else {
                        SharedOriginal(value.message)
                        MessageEmbeds(value.message.content.embeds)
                    }
                    TextButton({ links.open(url) }, shape = MaterialTheme.shapes.small) { Text("Jump to message") }
                }
            }
        }
    }
}
