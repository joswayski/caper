package chat.caper.android

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.ui.*

/**
 * The card a person's @mention pill opens: on phones, a bottom sheet in the
 * who-reacted pattern. Message opens (or creates) your DM with them; for you it
 * says "You" instead.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun MentionCardSheet(
    card: MentionCard,
    message: (done: () -> Unit, failed: (String) -> Unit) -> Unit,
    onDismiss: () -> Unit,
) {
    var opening by remember(card) { mutableStateOf(false) }
    var error by remember(card) { mutableStateOf<String?>(null) }
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = SurfaceRaised, contentColor = Text) {
        Column(
            Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 24.dp).semantics { paneTitle = card.title },
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Avatar(card.displayName ?: card.username ?: card.title, 48.dp, avatarId = card.avatarId)
            Column {
                Text(
                    card.title, Modifier.semantics { heading() }, color = Text, fontSize = 16.sp,
                    fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
                card.subtitle?.let { Text(it, color = TextMuted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
            if (card.self) Text("You", color = TextMuted, fontSize = 13.sp)
            else if (card.username != null) {
                Button(
                    {
                        opening = true
                        error = null
                        message({ opening = false; onDismiss() }, { opening = false; error = it })
                    },
                    Modifier.fillMaxWidth().heightIn(min = 48.dp), enabled = !opening, shape = MaterialTheme.shapes.small,
                ) { Text(if (opening) "Opening…" else "Message") }
                error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
            }
        }
    }
}
