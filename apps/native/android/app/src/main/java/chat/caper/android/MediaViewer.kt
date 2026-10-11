package chat.caper.android

import android.app.DownloadManager
import android.content.Context
import android.os.Build
import android.os.Environment
import androidx.annotation.OptIn
import androidx.compose.animation.core.animate
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.net.toUri
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.media3.common.AudioAttributes
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import chat.caper.android.data.withFreshUrls
import chat.caper.android.model.AppUiState
import chat.caper.android.model.AttachmentState
import chat.caper.android.model.ChatAttachment
import chat.caper.android.ui.*
import coil3.compose.AsyncImage
import coil3.gif.AnimatedImageDecoder
import coil3.gif.GifDecoder
import coil3.size.Size
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Opens in the media viewer: a ready image or video with a URL. Other files keep their cards. */
internal fun ChatAttachment.viewable(): Boolean =
    (kind == "image" || kind == "video") && !unavailable && state == AttachmentState.READY && url != null

/** The files one viewer pages through (by attachment ID) and the page it opened on. */
internal data class ViewerPages(val ids: List<String>, val start: Int)

/**
 * What tapping [clicked] opens: that message's viewable images and videos in message order,
 * starting at [clicked]. Null when [clicked] is not viewable.
 */
internal fun viewerPages(attachments: List<ChatAttachment>, clicked: String): ViewerPages? {
    // Pager keys must be unique; a repeated ID (which the API never sends) keeps its first place.
    val ids = attachments.filter { it.viewable() }.map { it.id }.distinct()
    val start = ids.indexOf(clicked)
    return if (start < 0) null else ViewerPages(ids, start)
}

/** "2 / 5"; nothing for a single file. */
internal fun viewerCounter(page: Int, count: Int): String? = if (count > 1) "${page + 1} / $count" else null

/** [width] × [height] scaled to fit the viewport (`ContentScale.Fit`); the viewport when the size is unknown. */
internal fun fittedSize(width: Float, height: Float, viewportWidth: Float, viewportHeight: Float): Pair<Float, Float> {
    if (width <= 0f || height <= 0f) return viewportWidth to viewportHeight
    val scale = minOf(viewportWidth / width, viewportHeight / height)
    return width * scale to height * scale
}

/** How far a zoomed image may move from centre along one axis: until its edge meets the screen's. */
internal fun panLimit(content: Float, viewport: Float, scale: Float): Float = maxOf(0f, (content * scale - viewport) / 2f)

/** A name DownloadManager can write: no path separators, reserved or control characters, or leading dots. */
internal fun downloadName(name: String): String =
    name.replace(Regex("[\\\\/:*?\"<>|\\p{Cntrl}]"), "_").trim().trimStart('.').ifEmpty { "file" }

/** A timeline's open viewer: the files it pages through and the caption under their names. */
internal data class OpenViewer(val pages: ViewerPages, val caption: String?)

/**
 * Holds a timeline's media viewer above its LazyColumn. A viewer composed in its message row
 * would close, stopping its video, once the row scrolls out of composition, as it does when new
 * messages arrive and the timeline follows them.
 */
internal class MediaViewerHost { var open by mutableStateOf<OpenViewer?>(null) }

/** The timeline's viewer host; files outside a timeline (forwarded conversations) keep their own viewer. */
internal val LocalMediaViewerHost = staticCompositionLocalOf<MediaViewerHost?> { null }

/**
 * [ids]' files as the conversation has them now, in [ids] order and on their freshest URLs, from
 * any loaded or pinned message or the original it forwards. Files no longer loaded are left out.
 */
internal fun liveAttachments(state: AppUiState, ids: List<String>): List<ChatAttachment> {
    val wanted = ids.toSet()
    val found = HashMap<String, ChatAttachment>()
    (state.messages + state.pinnedMessages).forEach { message ->
        (message.content.attachments + message.forward?.message?.content?.attachments.orEmpty()).forEach {
            if (it.id in wanted) found.putIfAbsent(it.id, it)
        }
    }
    return ids.mapNotNull { id -> found[id]?.withFreshUrls(state.freshAttachmentUrls[id]) }
}

/** [host]'s viewer over its files as [state] has them now; it closes once none is loaded (another conversation opened). */
@Composable internal fun HostedMediaViewer(host: MediaViewerHost, state: AppUiState, onLoadFailed: (ChatAttachment, Int?) -> Unit) {
    val open = host.open ?: return
    val attachments = remember(state.messages, state.pinnedMessages, state.freshAttachmentUrls, open) { liveAttachments(state, open.pages.ids) }
    if (attachments.isEmpty()) return SideEffect { host.open = null }
    val uriHandler = LocalUriHandler.current
    key(open) {
        MediaViewer(open.pages, attachments, open.caption, close = { host.open = null }, open = { url -> runCatching { uriHandler.openUri(url) } }, onLoadFailed = onLoadFailed)
    }
}

private const val MAX_ZOOM = 4f
private const val DOUBLE_TAP_ZOOM = 2f

/**
 * Full-screen viewer for one message's images and videos: swipe, arrows or the keyboard move
 * between them; images zoom; a video plays with controls while its page is showing. Pages follow
 * [attachments] live, so a re-signed URL reloads and a file removed meanwhile says so.
 */
@Composable internal fun MediaViewer(
    pages: ViewerPages, attachments: List<ChatAttachment>, caption: String?, close: () -> Unit,
    open: (String) -> Unit, onLoadFailed: (ChatAttachment, Int?) -> Unit,
) {
    val ids = pages.ids
    val names = remember { attachments.associate { it.id to it.name } }
    val pager = rememberPagerState(pages.start) { ids.size }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val density = LocalDensity.current
    var notice by remember { mutableStateOf<String?>(null) }
    var dragged by remember { mutableFloatStateOf(0f) }
    val live = { page: Int -> attachments.firstOrNull { it.id == ids[page] }?.takeIf { it.viewable() } }
    val go: (Int) -> Unit = { page -> if (page in ids.indices) scope.launch { pager.animateScrollToPage(page) } }
    LaunchedEffect(notice) { if (notice != null) { delay(4000); notice = null } }
    Dialog(onDismissRequest = close, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        val focus = remember { FocusRequester() }
        LaunchedEffect(Unit) { focus.requestFocus() }
        // Swiping down past this (or flinging down) closes; the backdrop fades on the way.
        val dismissAt = with(density) { 120.dp.toPx() }
        val flingAt = with(density) { 1000.dp.toPx() }
        val page = pager.currentPage
        val current = live(page)
        val url = current?.url
        Column(
            Modifier.fillMaxSize().background(Blackout.copy(alpha = 0.92f * (1f - (dragged / dismissAt).coerceIn(0f, 1f) / 2f)))
                .semantics { paneTitle = "Image viewer" }
                // Hardware keyboards: Left/Right move between files.
                .onKeyEvent { event ->
                    if (event.type != KeyEventType.KeyDown) false else when (event.key) {
                        Key.DirectionLeft -> { go(page - 1); true }
                        Key.DirectionRight -> { go(page + 1); true }
                        else -> false
                    }
                }
                .focusRequester(focus).focusable(),
        ) {
            Row(
                Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Top + WindowInsetsSides.Horizontal))
                    .padding(start = 16.dp, end = 4.dp, top = 4.dp, bottom = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(names[ids[page]].orEmpty(), Modifier.semantics { heading() }, color = Text, fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    caption?.let { Text(it, color = TextMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                }
                viewerCounter(page, ids.size)?.let {
                    Text(it, Modifier.padding(horizontal = 8.dp).semantics { contentDescription = "File ${page + 1} of ${ids.size}"; liveRegion = LiveRegionMode.Polite },
                        color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                }
                val tint = if (url != null) Text else TextMuted.copy(alpha = 0.4f)
                IconButton({
                    if (current != null && url != null) notice = if (download(context, current, url)) "Downloading ${current.name}…" else "Couldn’t start the download."
                }, Modifier.semantics { contentDescription = "Download" }, enabled = url != null) {
                    Icon(painterResource(R.drawable.lucide_download), null, Modifier.size(20.dp), tint = tint)
                }
                IconButton({ url?.let(open) }, Modifier.semantics { contentDescription = "Open in browser" }, enabled = url != null) {
                    Icon(painterResource(R.drawable.lucide_external_link), null, Modifier.size(20.dp), tint = tint)
                }
                IconButton(close, Modifier.semantics { contentDescription = "Close viewer" }) { Icon(painterResource(R.drawable.lucide_x), null, tint = Text) }
            }
            Box(
                Modifier.weight(1f).fillMaxWidth().windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Bottom + WindowInsetsSides.Horizontal))
                    .draggable(
                        rememberDraggableState { dragged = (dragged + it).coerceAtLeast(0f) }, Orientation.Vertical,
                        onDragStopped = { velocity ->
                            if (dragged > dismissAt || velocity > flingAt) close() else animate(dragged, 0f) { value, _ -> dragged = value }
                        },
                    ),
            ) {
                HorizontalPager(pager, Modifier.fillMaxSize().graphicsLayer { translationY = dragged }, pageSpacing = 16.dp, key = { ids[it] }) { index ->
                    val attachment = live(index)
                    val active = index == pager.settledPage
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        when {
                            attachment == null -> Text("File removed", color = TextMuted, fontSize = 14.sp)
                            attachment.kind == "image" -> ViewerImage(attachment, active, onLoadFailed)
                            else -> ViewerVideo(attachment, active, onLoadFailed)
                        }
                    }
                }
                if (page > 0) PageButton(R.drawable.lucide_chevron_left, "Previous file", Modifier.align(Alignment.CenterStart)) { go(page - 1) }
                if (page < ids.lastIndex) PageButton(R.drawable.lucide_chevron_right, "Next file", Modifier.align(Alignment.CenterEnd)) { go(page + 1) }
                notice?.let {
                    Text(it, Modifier.align(Alignment.BottomCenter).padding(16.dp).background(SurfaceRaised, MaterialTheme.shapes.small)
                        .padding(horizontal = 12.dp, vertical = 8.dp).semantics { liveRegion = LiveRegionMode.Polite }, color = Text, fontSize = 13.sp)
                }
            }
        }
    }
}

@Composable private fun PageButton(icon: Int, label: String, modifier: Modifier, onClick: () -> Unit) {
    IconButton(onClick, modifier.padding(horizontal = 8.dp).semantics { contentDescription = label },
        colors = IconButtonDefaults.iconButtonColors(containerColor = Blackout.copy(alpha = 0.72f), contentColor = Text)) {
        Icon(painterResource(icon), null, Modifier.size(20.dp))
    }
}

/**
 * The original over its preview (or a spinner) while it loads, at full resolution for zooming
 * (Coil caps decoded bitmaps at 4096 px a side). GIFs and animated WebP play. An original this
 * device cannot decode stays on the preview.
 */
@Composable private fun ViewerImage(attachment: ChatAttachment, active: Boolean, onLoadFailed: (ChatAttachment, Int?) -> Unit) {
    val url = attachment.url ?: return
    val context = LocalContext.current
    val preview = attachment.previewUrl
    var loaded by remember(url) { mutableStateOf<IntSize?>(null) }
    var failed by remember(url) { mutableStateOf(false) }
    val request = remember(url, attachment.id) {
        AttachmentImages.request(context, url, "${attachment.id}:original").newBuilder()
            .size(Size.ORIGINAL)
            .decoderFactory(if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) AnimatedImageDecoder.Factory() else GifDecoder.Factory())
            .build()
    }
    val known = attachment.width?.let { width -> attachment.height?.let { IntSize(width, it) } }
    ZoomableImage(active, loaded ?: known, Modifier.clearAndSetSemantics { contentDescription = attachment.name }) {
        if (loaded == null) when {
            preview != null -> AttachmentImage(attachment, preview, "display", Modifier.fillMaxSize(), ContentScale.Fit, onLoadFailed)
            failed -> Text("This image could not be shown.", color = ErrorText, fontSize = 12.sp)
            else -> Spinner(null, Modifier.size(28.dp))
        }
        if (!failed || preview == null) AsyncImage(
            model = request, contentDescription = attachment.name, imageLoader = AttachmentImages.loader(context),
            modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Fit,
            onSuccess = { loaded = IntSize(it.result.image.width, it.result.image.height) },
            onError = { error ->
                failed = true
                onLoadFailed(attachment, error.result.throwable.httpStatus())
            },
        )
    }
}

/**
 * Fits [content] to the page and zooms it: pinch, or double-tap to toggle 2× on the tapped point;
 * one finger pans while zoomed. Unzoomed one-finger drags are left to the pager and swipe-to-close.
 */
@Composable private fun ZoomableImage(active: Boolean, contentSize: IntSize?, modifier: Modifier, content: @Composable BoxScope.() -> Unit) {
    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    var viewport by remember { mutableStateOf(IntSize.Zero) }
    val bounds by rememberUpdatedState(contentSize)
    // Moving to another file resets the zoom.
    LaunchedEffect(active) { if (!active) { scale = 1f; offset = Offset.Zero } }
    fun clamped(value: Offset, zoom: Float): Offset {
        val (width, height) = fittedSize(
            bounds?.width?.toFloat() ?: 0f, bounds?.height?.toFloat() ?: 0f, viewport.width.toFloat(), viewport.height.toFloat(),
        )
        val x = panLimit(width, viewport.width.toFloat(), zoom)
        val y = panLimit(height, viewport.height.toFloat(), zoom)
        return Offset(value.x.coerceIn(-x, x), value.y.coerceIn(-y, y))
    }
    Box(
        modifier.fillMaxSize().onSizeChanged { viewport = it }
            .pointerInput(Unit) {
                detectTapGestures(onDoubleTap = { tap ->
                    val center = Offset(size.width / 2f, size.height / 2f)
                    if (scale > 1f) { scale = 1f; offset = Offset.Zero }
                    else { scale = DOUBLE_TAP_ZOOM; offset = clamped((center - tap) * (DOUBLE_TAP_ZOOM - 1f), DOUBLE_TAP_ZOOM) }
                })
            }
            .pointerInput(Unit) {
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    do {
                        val event = awaitPointerEvent()
                        if (event.changes.count { it.pressed } > 1 || scale > 1f) {
                            val center = Offset(size.width / 2f, size.height / 2f)
                            val centroid = event.calculateCentroid()
                            val next = (scale * event.calculateZoom()).coerceIn(1f, MAX_ZOOM)
                            // Zoom about the fingers: the point under them stays under them.
                            val moved = if (centroid.isSpecified) (centroid - center) * (1f - next / scale) + offset * (next / scale) else offset
                            offset = if (next == 1f) Offset.Zero else clamped(moved + event.calculatePan(), next)
                            scale = next
                            event.changes.forEach { if (it.positionChanged()) it.consume() }
                        }
                    } while (event.changes.any { it.pressed })
                }
            }
            .graphicsLayer { scaleX = scale; scaleY = scale; translationX = offset.x; translationY = offset.y },
        contentAlignment = Alignment.Center,
        content = content,
    )
}

/** [rememberAttachmentPlayback]'s player and what it has shown so far. */
internal class AttachmentPlayback(val player: ExoPlayer) {
    var rendered by mutableStateOf(false)
    var failed by mutableStateOf(false)
}

/**
 * An ExoPlayer for [attachment], prepared and released with the composition. Sound takes audio
 * focus unless [silent] (a stored GIF, which leaves other apps' audio alone). A re-signed URL
 * doesn't restart playback: the player keeps reading its URL until that fails, then carries on
 * from the same spot on the newest one; a failure with nothing newer goes to [onLoadFailed].
 */
@OptIn(UnstableApi::class)
@Composable internal fun rememberAttachmentPlayback(
    attachment: ChatAttachment, url: String, silent: Boolean, onLoadFailed: (ChatAttachment, Int?) -> Unit, configure: ExoPlayer.() -> Unit = {},
): AttachmentPlayback {
    val context = LocalContext.current
    val latest by rememberUpdatedState(attachment)
    val reportFailure by rememberUpdatedState(onLoadFailed)
    // The URL the player is reading.
    var source by remember(attachment.id) { mutableStateOf(url) }
    val playback = remember(attachment.id) {
        AttachmentPlayback(ExoPlayer.Builder(context).build().apply {
            setAudioAttributes(AudioAttributes.DEFAULT, !silent)
            configure()
            setMediaItem(MediaItem.fromUri(url))
            prepare()
        })
    }
    val player = playback.player
    fun reload(next: String) {
        source = next
        playback.failed = false
        player.setMediaItem(MediaItem.fromUri(next), player.currentPosition)
        player.prepare()
    }
    DisposableEffect(player) {
        val listener = object : Player.Listener {
            override fun onRenderedFirstFrame() { playback.rendered = true }
            override fun onPlayerError(error: PlaybackException) {
                val fresh = latest.url
                if (fresh != null && fresh != source) reload(fresh)
                else {
                    playback.failed = true
                    reportFailure(latest, (error.cause as? HttpDataSource.InvalidResponseCodeException)?.responseCode)
                }
            }
        }
        player.addListener(listener)
        onDispose {
            player.removeListener(listener)
            player.release()
        }
    }
    // The fresh URL a failure asked for.
    LaunchedEffect(url) { if (playback.failed && url != source) reload(url) }
    return playback
}

/**
 * Plays while its page is the settled one and the app is visible, and pauses when paged away;
 * leaving the viewer releases it. A stored GIF loops silently without controls. The poster shows
 * until the first frame.
 */
@OptIn(UnstableApi::class)
@Composable private fun ViewerVideo(attachment: ChatAttachment, active: Boolean, onLoadFailed: (ChatAttachment, Int?) -> Unit) {
    val url = attachment.url ?: return
    val playback = rememberAttachmentPlayback(attachment, url, silent = attachment.animated, onLoadFailed) {
        if (attachment.animated) {
            volume = 0f
            repeatMode = Player.REPEAT_MODE_ONE
        }
    }
    val player = playback.player
    LifecycleStartEffect(player, active) {
        player.playWhenReady = active
        onStopOrDispose { player.pause() }
    }
    AndroidView(
        factory = {
            PlayerView(it).apply {
                useController = !attachment.animated
                setShowBuffering(PlayerView.SHOW_BUFFERING_WHEN_PLAYING)
                setShutterBackgroundColor(android.graphics.Color.TRANSPARENT)
            }
        },
        update = { it.player = player },
        modifier = Modifier.fillMaxSize(),
    )
    if (!playback.rendered) attachment.previewUrl?.let { AttachmentImage(attachment, it, "display", Modifier.fillMaxSize(), ContentScale.Fit, onLoadFailed) }
    if (playback.failed) Text("This file could not be played.", color = ErrorText, fontSize = 12.sp)
}

/**
 * Saves the file with the system download manager, which shows progress and opens it from its
 * notification. Android 10+ writes to the shared Downloads folder without a permission; older
 * versions would need storage access for that, so they keep it in the app's own Downloads folder.
 */
private fun download(context: Context, attachment: ChatAttachment, url: String): Boolean = runCatching {
    val name = downloadName(attachment.name)
    val request = DownloadManager.Request(url.toUri())
        .setTitle(attachment.name)
        .setMimeType(attachment.contentType)
        .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) request.setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, name)
    else request.setDestinationInExternalFilesDir(context, Environment.DIRECTORY_DOWNLOADS, name)
    context.getSystemService(DownloadManager::class.java).enqueue(request)
}.isSuccess
