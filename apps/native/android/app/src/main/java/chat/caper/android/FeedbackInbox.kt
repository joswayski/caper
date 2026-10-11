package chat.caper.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.ADMIN_BADGE
import chat.caper.android.data.feedbackAge
import chat.caper.android.data.feedbackLocation
import chat.caper.android.data.feedbackPreview
import chat.caper.android.data.feedbackStatus
import chat.caper.android.model.*
import chat.caper.android.ui.*

/**
 * The feedback team's inbox, in place of a conversation: every `@feedback` report, newest
 * first. A row opens its conversation as a visit, at the report with its thread open.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun FeedbackInbox(state: AppUiState, viewModel: CaperViewModel, narrow: Boolean, openNavigation: () -> Unit, modifier: Modifier = Modifier) {
    val inbox = state.feedback
    // The first page loads when the inbox opens; this covers a restored or recreated screen.
    LaunchedEffect(Unit) { if (!inbox.loaded && !inbox.loading) viewModel.refreshFeedback() }
    Column(modifier.fillMaxSize().background(SurfaceConversation)) {
        Row(Modifier.fillMaxWidth().height(53.dp).padding(start = if (narrow) 4.dp else 18.dp, end = 18.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            if (narrow) IconButton(openNavigation, Modifier.size(48.dp)) {
                Icon(painterResource(R.drawable.lucide_arrow_right), "Back to Browse", Modifier.size(20.dp).graphicsLayer { rotationZ = 180f }, tint = TextMuted)
            }
            Text("Feedback", Modifier.weight(1f).semantics { heading() }, fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1)
        }
        HorizontalDivider(color = Border)
        PullToRefreshBox(isRefreshing = inbox.loading && inbox.loaded, onRefresh = viewModel::refreshFeedback, modifier = Modifier.weight(1f).fillMaxWidth()) {
            when {
                !inbox.loaded && inbox.error != null -> InboxNotice(inbox.error, "Try again", viewModel::refreshFeedback)
                !inbox.loaded -> InboxNotice("Loading feedback…")
                inbox.reports.isEmpty() -> InboxNotice("No feedback yet. Reports appear here when someone writes @feedback.")
                else -> LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(vertical = 8.dp)) {
                    inbox.error?.let { error -> item("error") {
                        Text(error, Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 6.dp), color = ErrorText, fontSize = 12.sp)
                    } }
                    items(inbox.reports, key = { it.id }) { report -> FeedbackRow(report) { viewModel.openFeedbackReport(report) } }
                    if (inbox.cursor != null) item("more") {
                        Box(Modifier.fillMaxWidth().padding(vertical = 8.dp), contentAlignment = Alignment.Center) {
                            OutlinedButton(viewModel::loadMoreFeedback, enabled = !inbox.loadingMore && !inbox.loading, shape = MaterialTheme.shapes.small,
                                border = BorderStroke(1.dp, Border), colors = ButtonDefaults.outlinedButtonColors(contentColor = Text)) {
                                Text(if (inbox.loadingMore) "Loading…" else "Load older reports", fontSize = 12.sp)
                            }
                        }
                    }
                }
            }
        }
    }
}

/** A centred line (and optional action), scrollable so pull to refresh still works. */
@Composable private fun InboxNotice(text: String, action: String? = null, onAction: () -> Unit = {}) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp, Alignment.CenterVertically)) {
        Text(text, color = TextMuted, fontSize = 13.sp, textAlign = TextAlign.Center)
        if (action != null) OutlinedButton(onAction, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border),
            colors = ButtonDefaults.outlinedButtonColors(containerColor = SurfaceRaised, contentColor = Text)) { Text(action, fontSize = 12.sp) }
    }
}

/** One report: who, where, what, when, and a quiet status ("Waiting" in terracotta). */
@Composable private fun FeedbackRow(report: FeedbackReport, open: () -> Unit) {
    val name = report.reporter.name
    val location = feedbackLocation(report)
    val preview = feedbackPreview(report)
    val age = feedbackAge(report.createdAt)
    val status = feedbackStatus(report)
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp)
        .clickable(role = Role.Button, onClickLabel = "Open conversation", onClick = open)
        .padding(horizontal = 18.dp, vertical = 10.dp), verticalAlignment = Alignment.Top) {
        Avatar(name, 34.dp, avatarId = report.reporter.avatarId)
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                    Text(name, Modifier.weight(1f, fill = false), fontSize = 13.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (age.isNotEmpty()) { Spacer(Modifier.width(7.dp)); Text(age, color = TextMuted, fontSize = 10.sp, maxLines = 1) }
                }
                Spacer(Modifier.width(8.dp))
                Text(status, color = if (report.answered) TextMuted else TerracottaBright, fontSize = 11.sp, fontWeight = FontWeight.Bold, maxLines = 1)
            }
            Text(location, color = TextMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (preview.isNotEmpty()) Text(preview, color = MessageText, fontSize = 13.sp, lineHeight = 19.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** Marks a Caper feedback team member's message, in the pin gold with a soft glow. The server stamps it, so a look-alike name can't. */
@Composable internal fun AdminBadge() {
    Surface(color = PinGold.copy(alpha = 0.10f), border = BorderStroke(1.dp, PinGold.copy(alpha = 0.55f)), shape = MaterialTheme.shapes.extraSmall) {
        Text(ADMIN_BADGE, Modifier.padding(horizontal = 5.dp, vertical = 2.dp), color = PinGold, fontSize = 8.sp, fontWeight = FontWeight.Bold,
            style = androidx.compose.ui.text.TextStyle(shadow = androidx.compose.ui.graphics.Shadow(PinGold.copy(alpha = 0.6f), blurRadius = 8f)))
    }
}
