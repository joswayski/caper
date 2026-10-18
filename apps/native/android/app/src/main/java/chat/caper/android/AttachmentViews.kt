package chat.caper.android

import android.content.Context
import android.content.Intent
import android.os.Build
import android.widget.Toast
import androidx.annotation.OptIn
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.net.toUri
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import chat.caper.android.data.AttachmentPolicy
import chat.caper.android.model.AttachmentState
import chat.caper.android.model.ChatAttachment
import chat.caper.android.model.DraftAttachmentUi
import chat.caper.android.ui.*
import coil3.ImageLoader
import coil3.compose.AsyncImage
import coil3.disk.DiskCache
import coil3.gif.AnimatedImageDecoder
import coil3.gif.GifDecoder
import coil3.memory.MemoryCache
import coil3.network.HttpException
import coil3.network.okhttp.OkHttpNetworkFetcherFactory
import coil3.request.ImageRequest
import coil3.serviceLoaderEnabled
import okhttp3.OkHttpClient
import okio.Path.Companion.toOkioPath

/**
 * One image loader for attachments. Caches are keyed by attachment ID, so re-signed URLs reuse
 * them. OkHttp adds `Accept-Encoding: gzip` itself, so a gzip-encoded response is decoded
 * transparently. Below API 31 the platform cannot decode AVIF, so a bundled decoder handles it.
 * Animation decoders are selected per request so posters and reduced-motion images stay still.
 */
internal object AttachmentImages {
    @Volatile private var loader: ImageLoader? = null

    fun loader(context: Context): ImageLoader = loader ?: synchronized(this) {
        loader ?: context.applicationContext.let { app ->
            ImageLoader.Builder(app)
                .serviceLoaderEnabled(false)
                .components {
                    // Delivery URLs are already signed; never follow redirects or attach credentials.
                    add(OkHttpNetworkFetcherFactory(callFactory = { OkHttpClient.Builder().followRedirects(false).followSslRedirects(false).build() }))
                    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) add(AvifCompatDecoder.Factory())
                }
                .memoryCache { MemoryCache.Builder().maxSizePercent(app, 0.15).build() }
                .diskCache { DiskCache.Builder().directory(app.cacheDir.resolve("attachment-images").toOkioPath()).maxSizeBytes(128L * 1024 * 1024).build() }
                .build()
                .also { loader = it }
        }
    }

    fun request(context: Context, data: Any?, key: String): ImageRequest =
        ImageRequest.Builder(context).data(data).memoryCacheKey(key).diskCacheKey(key).build()
}

internal fun Throwable.httpStatus(): Int? = (this as? HttpException)?.response?.code

/** Reserved display size before the image loads (web `frame`: at most 360 × 300). */
internal fun attachmentFrame(width: Int?, height: Int?, maxWidth: Int = 360, maxHeight: Int = 300): Pair<Int, Int>? {
    if (width == null || height == null || width <= 0 || height <= 0) return null
    val scale = minOf(1.0, maxWidth.toDouble() / width, maxHeight.toDouble() / height)
    return maxOf(1, Math.round(width * scale).toInt()) to maxOf(1, Math.round(height * scale).toInt())
}

internal fun durationLabel(durationMs: Long?): String? {
    val seconds = (durationMs ?: return null) / 1000
    return if (seconds >= 3600) "%d:%02d:%02d".format(java.util.Locale.US, seconds / 3600, seconds / 60 % 60, seconds % 60) else "%d:%02d".format(java.util.Locale.US, seconds / 60, seconds % 60)
}

/**
 * Files under a message, shown by processing status. Pending rows show local copies and are not
 * interactive. [progress] is the latest server percent per file; [localPreviews] are this
 * device's own picked images, shown while the server processes them. Tapping a ready image or
 * video opens the media viewer on this message's images and videos (the timeline's
 * [LocalMediaViewerHost] when there is one); [caption] (sender and time) shows under the file
 * name there.
 */
@Composable internal fun MessageAttachments(
    attachments: List<ChatAttachment>,
    pending: Boolean,
    onLoadFailed: (ChatAttachment, Int?) -> Unit,
    progress: Map<String, Int> = emptyMap(),
    localPreviews: Map<String, String> = emptyMap(),
    caption: String? = null,
) {
    if (attachments.isEmpty()) return
    var viewer by remember { mutableStateOf<ViewerPages?>(null) }
    var playing by remember { mutableStateOf<String?>(null) }
    var revealed by remember(attachments.map { it.id to it.spoiler }) { mutableStateOf(emptySet<String>()) }
    val uriHandler = LocalUriHandler.current
    val still = reducedMotion()
    // The system browser downloads or shows the file and handles `Content-Encoding: gzip` itself.
    val open: (String) -> Unit = { url -> runCatching { uriHandler.openUri(url) } }
    val host = LocalMediaViewerHost.current
    val view: (ChatAttachment) -> (() -> Unit)? = { attachment ->
        if (pending) null else ({
            val pages = viewerPages(attachments, attachment.id, revealed)
            if (host != null) host.open = pages?.let { OpenViewer(it, caption) } else viewer = pages
        })
    }
    @Composable fun RenderAttachment(attachment: ChatAttachment, modifier: Modifier, shape: Shape) {
        val url = attachment.url
        val visual = attachment.kind == "image" || attachment.kind == "video"
        val status = when {
            attachment.unavailable -> "File removed"
            attachment.state == AttachmentState.FAILED -> "Processing failed"
            attachment.state != AttachmentState.PROCESSING && url == null -> "File unavailable"
            else -> null
        }
        when {
            attachment.spoiler && attachment.id !in revealed -> Surface(
                modifier = modifier.then(if (modifier == Modifier) Modifier.fillMaxWidth().height(if (visual) 180.dp else 64.dp) else Modifier)
                    .clip(shape).clickable(role = Role.Button, onClickLabel = "Reveal spoiler attachment") {
                        revealed = revealed + attachment.id
                    },
                color = Surface, shape = shape, border = BorderStroke(1.dp, Border),
            ) {
                val source = if (visual && !attachment.unavailable && attachment.state != AttachmentState.FAILED) {
                    attachment.previewUrl ?: if (attachment.kind == "image") attachment.url else null
                } else null
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    SpoilerThumbnail(source, "${attachment.id}:spoiler") { onLoadFailed(attachment, it.httpStatus()) }
                    Text("SPOILER · Tap to reveal", Modifier.background(Blackout.copy(alpha = 0.8f), MaterialTheme.shapes.small).padding(12.dp), color = Text, fontSize = 12.sp)
                }
            }
            visual && status != null -> MediaFrame(attachment, null, description = "${attachment.name}, $status", modifier = modifier, shape = shape) {
                Column(Modifier.align(Alignment.Center).padding(8.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(attachment.name, color = TextMuted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(status, color = if (attachment.state == AttachmentState.FAILED) ErrorText else TextMuted, fontSize = 11.sp)
                }
            }
            attachment.unavailable -> FileCard(attachment, null)
            attachment.state == AttachmentState.FAILED -> FileCard(attachment, null, detail = "Couldn’t process this file", error = true)
            attachment.state == AttachmentState.PROCESSING -> ProcessingAttachment(
                attachment, attachment.previewUrl ?: localPreviews[attachment.id], progress[attachment.id], onLoadFailed, modifier, shape,
            )
            url == null -> FileCard(attachment, null)
            attachment.kind == "image" -> MediaFrame(attachment, view(attachment), modifier = modifier, shape = shape) {
                val gif = attachment.contentType == "image/gif"
                AttachmentImage(
                    attachment, if (gif) url else attachment.previewUrl ?: url,
                    if (gif) "gif" else "display", Modifier.fillMaxSize(), ContentScale.Crop, onLoadFailed,
                    animate = gif && !still,
                )
            }
            // Stored GIFs and animated images: muted, looping, inline, no controls.
            attachment.kind == "video" && attachment.animated -> MediaFrame(attachment, view(attachment), modifier = modifier, shape = shape) {
                attachment.previewUrl?.let { AttachmentImage(attachment, it, "display", Modifier.fillMaxSize(), ContentScale.Crop, onLoadFailed) }
                if (!pending) AnimatedVideo(attachment, url, onLoadFailed)
            }
            attachment.kind == "video" -> MediaFrame(attachment, view(attachment), modifier = modifier, shape = shape) {
                attachment.previewUrl?.let { AttachmentImage(attachment, it, "display", Modifier.fillMaxSize(), ContentScale.Crop, onLoadFailed) }
                Surface(Modifier.align(Alignment.Center).size(44.dp), shape = CircleShape, color = Blackout.copy(alpha = 0.72f)) {
                    Icon(painterResource(R.drawable.lucide_play), null, Modifier.padding(12.dp), tint = Text)
                }
                durationLabel(attachment.durationMs)?.let {
                    Text(it, Modifier.align(Alignment.BottomEnd).padding(6.dp).background(Blackout.copy(alpha = 0.72f), MaterialTheme.shapes.extraSmall).padding(horizontal = 5.dp, vertical = 2.dp), color = Text, fontSize = 10.sp, fontWeight = FontWeight.Bold)
                }
            }
            attachment.kind == "audio" -> FileCard(attachment, if (pending) null else ({ playing = attachment.id }), R.drawable.lucide_play)
            else -> FileCard(attachment, if (pending) null else ({ open(url) }))
        }
    }
    val (visuals, files) = attachments.partition { it.kind == "image" || it.kind == "video" }
    Column(Modifier.padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (visuals.size == 1) RenderAttachment(visuals.single(), Modifier, MaterialTheme.shapes.small)
        else if (visuals.isNotEmpty()) BoxWithConstraints(Modifier.widthIn(max = 480.dp).fillMaxWidth()) {
            val cells = attachmentGalleryCells(visuals.size, maxWidth.value)
            val height = cells.maxOf { it.y + it.height }
            Box(Modifier.fillMaxWidth().height(height.dp)) {
                visuals.forEachIndexed { index, attachment -> key(attachment.id) {
                    val cell = cells[index]
                    val shape = RoundedCornerShape(
                        topStart = if (cell.left && cell.top) 12.dp else 1.dp,
                        topEnd = if (cell.right && cell.top) 12.dp else 1.dp,
                        bottomEnd = if (cell.right && cell.bottom) 12.dp else 1.dp,
                        bottomStart = if (cell.left && cell.bottom) 12.dp else 1.dp,
                    )
                    RenderAttachment(attachment, Modifier.offset(cell.x.dp, cell.y.dp).size(cell.width.dp, cell.height.dp), shape)
                } }
            }
        }
        if (files.isNotEmpty()) BoxWithConstraints(Modifier.widthIn(max = 480.dp).fillMaxWidth()) {
            // Keep two readable cards only when each has at least 220 dp of room.
            val columns = if (maxWidth >= 446.dp) 2 else 1
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                files.chunked(columns).forEach { row ->
                    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        row.forEach { attachment -> key(attachment.id) {
                            Box(Modifier.width((maxWidth - 6.dp * (columns - 1)) / columns)) { RenderAttachment(attachment, Modifier, MaterialTheme.shapes.small) }
                        } }
                    }
                }
            }
        }
    }
    viewer?.let { MediaViewer(it, attachments, caption, close = { viewer = null }, open = open, onLoadFailed = onLoadFailed) }
    attachments.firstOrNull { it.id == playing && it.url != null && it.state == AttachmentState.READY }?.let { attachment ->
        MediaPlayerDialog(attachment, close = { playing = null }, open = open, onLoadFailed = onLoadFailed)
    }
}

/** Placeholder while the media worker compresses the file: preview (if any), spinner and percent. */
@Composable private fun ProcessingAttachment(attachment: ChatAttachment, preview: String?, percent: Int?, onLoadFailed: (ChatAttachment, Int?) -> Unit, modifier: Modifier = Modifier, shape: Shape = MaterialTheme.shapes.small) {
    val label = if (percent != null) "Processing… $percent%" else "Processing…"
    val visual = attachment.kind == "image" || attachment.kind == "video"
    if (!visual) {
        FileCard(attachment, null, detail = label, busy = percent ?: -1)
        return
    }
    MediaFrame(attachment, null, description = "${attachment.name}, $label", modifier = modifier, shape = shape) {
        if (preview != null) AttachmentImage(attachment, preview, "processing", Modifier.fillMaxSize(), ContentScale.Crop, onLoadFailed)
        Box(Modifier.fillMaxSize().background(Blackout.copy(alpha = if (preview != null) 0.45f else 0f)))
        Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Spinner(percent, Modifier.size(28.dp))
            Text(label, color = Text, fontSize = 11.sp, fontWeight = FontWeight.Bold)
        }
    }
}

@Composable internal fun Spinner(percent: Int?, modifier: Modifier) {
    if (percent != null && percent > 0) CircularProgressIndicator(
        progress = { percent / 100f }, modifier = modifier, color = Terracotta, trackColor = Border, strokeWidth = 3.dp,
    ) else CircularProgressIndicator(modifier = modifier, color = Terracotta, trackColor = Border, strokeWidth = 3.dp)
}

/** A stored GIF: plays muted and looping like the original, with no controls. */
@OptIn(UnstableApi::class)
@Composable private fun BoxScope.AnimatedVideo(attachment: ChatAttachment, url: String, onLoadFailed: (ChatAttachment, Int?) -> Unit) {
    val context = LocalContext.current
    val latest by rememberUpdatedState(attachment)
    val reportFailure by rememberUpdatedState(onLoadFailed)
    var failed by remember(url) { mutableStateOf(false) }
    val player = remember(url) {
        ExoPlayer.Builder(context).build().apply {
            volume = 0f
            repeatMode = Player.REPEAT_MODE_ONE
            setMediaItem(MediaItem.fromUri(url))
            addListener(object : Player.Listener {
                override fun onPlayerError(error: PlaybackException) {
                    failed = true
                    reportFailure(latest, (error.cause as? HttpDataSource.InvalidResponseCodeException)?.responseCode)
                }
            })
            prepare()
        }
    }
    DisposableEffect(player) { onDispose { player.release() } }
    // Like the viewer's video: plays only while the app is visible, and pauses when it is stopped.
    LifecycleStartEffect(player) {
        player.playWhenReady = true
        onStopOrDispose { player.pause() }
    }
    if (failed) return
    AndroidView(
        factory = {
            PlayerView(it).apply {
                useController = false
                setShutterBackgroundColor(android.graphics.Color.TRANSPARENT)
                resizeMode = androidx.media3.ui.AspectRatioFrameLayout.RESIZE_MODE_ZOOM
            }
        },
        update = { it.player = player },
        modifier = Modifier.matchParentSize(),
    )
}

@Composable private fun MediaFrame(
    attachment: ChatAttachment, onClick: (() -> Unit)?, description: String = attachment.name,
    modifier: Modifier = Modifier, shape: Shape = MaterialTheme.shapes.small, content: @Composable BoxScope.() -> Unit,
) {
    val frame = attachmentFrame(attachment.width, attachment.height)
    val width: Dp = (frame?.first ?: 240).dp
    val ratio = frame?.let { it.first.toFloat() / it.second } ?: (4f / 3f)
    Box(
        modifier.then(if (modifier == Modifier) Modifier.widthIn(max = width).fillMaxWidth().aspectRatio(ratio) else Modifier)
            .clip(shape).background(Surface).border(1.dp, Border, shape)
            .then(if (onClick != null) Modifier.clickable(role = Role.Button, onClickLabel = "Open ${attachment.name}", onClick = onClick) else Modifier)
            .semantics { contentDescription = description },
        content = content,
    )
}

@Composable internal fun AttachmentImage(
    attachment: ChatAttachment, data: String, variant: String, modifier: Modifier, scale: ContentScale,
    onLoadFailed: (ChatAttachment, Int?) -> Unit,
    animate: Boolean = false,
) {
    val context = LocalContext.current
    val request = remember(data, attachment.id, variant, animate) {
        val key = "${attachment.id}:$variant" + if (animate) ":animated" else ""
        AttachmentImages.request(context, data, key).newBuilder().apply {
            if (animate) decoderFactory(if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) AnimatedImageDecoder.Factory() else GifDecoder.Factory())
        }.build()
    }
    AsyncImage(
        model = request, contentDescription = attachment.name, imageLoader = AttachmentImages.loader(context),
        modifier = modifier, contentScale = scale,
        onError = { error -> onLoadFailed(attachment, error.result.throwable.httpStatus()) },
    )
}

/** [busy] shows a spinner: a percent, or -1 while the percent is unknown. */
@Composable private fun FileCard(
    attachment: ChatAttachment, onClick: (() -> Unit)?, icon: Int = R.drawable.lucide_file_text,
    detail: String? = null, error: Boolean = false, busy: Int? = null,
) {
    val muted = attachment.unavailable || error
    val text = detail ?: when {
        attachment.unavailable -> "File removed"
        attachment.kind == "audio" -> listOfNotNull(durationLabel(attachment.durationMs), AttachmentPolicy.formatBytes(attachment.size)).joinToString(" · ")
        else -> AttachmentPolicy.formatBytes(attachment.size)
    }
    Surface(
        // Clipped first, so the press ripple keeps the card's corners.
        Modifier.widthIn(max = 360.dp).fillMaxWidth().heightIn(min = 48.dp).clip(MaterialTheme.shapes.small)
            .then(if (onClick != null) Modifier.clickable(role = Role.Button, onClickLabel = "Open ${attachment.name}", onClick = onClick) else Modifier),
        shape = MaterialTheme.shapes.small, color = if (muted) Color.Transparent else Surface,
        border = BorderStroke(1.dp, if (error) ErrorText.copy(alpha = 0.6f) else Border),
    ) {
        Row(Modifier.padding(horizontal = 10.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            if (busy != null) Spinner(busy.takeIf { it >= 0 }, Modifier.size(18.dp))
            else Icon(painterResource(if (muted) R.drawable.lucide_file_text else icon), null, Modifier.size(18.dp), tint = if (muted) TextMuted else Text)
            Column(Modifier.weight(1f)) {
                Text(attachment.name, color = if (muted) TextMuted else Text, fontSize = 13.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(text, color = if (error) ErrorText else TextMuted, fontSize = 11.sp)
            }
        }
    }
}

@Composable private fun ViewerFrame(title: String, close: () -> Unit, actions: @Composable RowScope.() -> Unit, content: @Composable BoxScope.() -> Unit) {
    Dialog(onDismissRequest = close, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Column(Modifier.fillMaxSize().swipeCancelsPresses().background(Blackout)) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(title, Modifier.weight(1f), color = Text, fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                actions()
                IconButton(close, Modifier.semantics { contentDescription = "Close" }) { Icon(painterResource(R.drawable.lucide_x), null, tint = Text) }
            }
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center, content = content)
        }
    }
}

/** In-app audio playback; a failed load asks for fresh URLs once, then offers the browser. */
@OptIn(UnstableApi::class)
@Composable private fun MediaPlayerDialog(attachment: ChatAttachment, close: () -> Unit, open: (String) -> Unit, onLoadFailed: (ChatAttachment, Int?) -> Unit) {
    val url = attachment.url ?: return
    val playback = rememberAttachmentPlayback(attachment, url, silent = false, onLoadFailed) { playWhenReady = true }
    val player = playback.player
    ViewerFrame(attachment.name, close, actions = { TextButton({ open(url) }) { Text("Open", color = Text) } }) {
        AndroidView(
            factory = { PlayerView(it).apply { useController = true; setShowBuffering(PlayerView.SHOW_BUFFERING_WHEN_PLAYING) } },
            update = { it.player = player },
            modifier = Modifier.fillMaxSize(),
        )
        if (playback.failed) Text("This file could not be played.", Modifier.align(Alignment.TopCenter).padding(12.dp), color = ErrorText, fontSize = 12.sp)
    }
}

/** Compact composer previews shared by message and profile uploads; never scroll sideways. */
@kotlin.OptIn(ExperimentalLayoutApi::class)
@Composable internal fun DraftAttachmentStrip(
    drafts: List<DraftAttachmentUi>, remove: (String) -> Unit, retry: ((String) -> Unit)? = null,
    enabled: Boolean = true, uploadOnSend: Boolean = false, sending: Boolean = false,
    toggleSpoiler: ((String) -> Unit)? = null,
) {
    if (drafts.isEmpty()) return
    val context = LocalContext.current
    val still = reducedMotion()
    val open: (String) -> Unit = { url ->
        runCatching {
            val uri = url.toUri()
            val intent = if (uri.scheme == "content") {
                Intent(Intent.ACTION_VIEW).setDataAndType(uri, context.contentResolver.getType(uri) ?: "*/*")
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            } else Intent(Intent.ACTION_VIEW, uri)
            context.startActivity(intent)
        }.onFailure {
            Toast.makeText(context, "No app is available to preview this file.", Toast.LENGTH_SHORT).show()
        }
    }
    Column(Modifier.fillMaxWidth()) {
        val failed = drafts.count { it.error != null }
        val active = drafts.count { it.attachment == null && it.error == null && (!uploadOnSend || sending || it.compressing) }
        if (failed > 0) Text(
            if (failed == 1) "1 file failed to upload. Send will retry it, or you can remove it."
            else "$failed files failed to upload. Send will retry them, or you can remove them.",
            Modifier.fillMaxWidth().padding(bottom = 8.dp).semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp,
        )
        if (active > 0) Text(
            "Uploading $active file${if (active == 1) "" else "s"}… Your message won’t send until every file is ready.",
            Modifier.fillMaxWidth().padding(bottom = 8.dp).semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 12.sp,
        )
        BoxWithConstraints(Modifier.fillMaxWidth().padding(bottom = 8.dp).semantics { contentDescription = "Files to send" }) {
            val columns = ((maxWidth.value + 8f) / 98f).toInt().coerceIn(1, 4)
            val tileSize = minOf(104.dp, (maxWidth - 8.dp * (columns - 1)) / columns)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp), maxItemsInEachRow = columns) {
                // Keyed, so each tile arrives once and keeps its own progress animation.
                drafts.forEach { draft -> key(draft.key) {
                    var menuOpen by remember { mutableStateOf(false) }
                    LaunchedEffect(enabled) { if (!enabled) menuOpen = false }
                    Surface(Modifier.entrance(MOTION_BASE).size(tileSize), shape = MaterialTheme.shapes.small, color = Surface, border = BorderStroke(1.dp, if (draft.error != null) ErrorText.copy(alpha = 0.6f) else Border)) {
                        Box(Modifier.fillMaxSize()) {
                            val url = draft.pickedUri ?: draft.attachment?.url
                            val busy = draft.attachment == null && draft.error == null && (!uploadOnSend || sending || draft.compressing)
                            val status = when {
                                draft.error != null -> "Failed"
                                !busy -> null
                                draft.compressing -> if (draft.progress > 0f) "Prep ${(draft.progress * 100).toInt()}%" else "Preparing…"
                                else -> "Uploading ${(draft.progress * 100).toInt()}%"
                            }
                            val accessibleName = if (draft.spoiler) "spoiler attachment" else draft.name
                            Box(Modifier.fillMaxSize().combinedClickable(
                                enabled = url != null || enabled, role = Role.Button, onClickLabel = "Preview $accessibleName",
                                onClick = { url?.let(open) },
                                onLongClickLabel = "Attachment options",
                                onLongClick = if (enabled) ({ menuOpen = true }) else null,
                            ).semantics { contentDescription = listOfNotNull(accessibleName, if (draft.spoiler) status else draft.error ?: status).joinToString(", ") }, contentAlignment = Alignment.Center) {
                                val thumbnail = draft.thumbnail
                                if (draft.spoiler) {
                                    val visual = draft.kind == "image" || draft.kind == "video"
                                    val source = if (visual && draft.error == null && draft.attachment?.unavailable != true) {
                                        thumbnail ?: if (draft.kind == "image") draft.pickedImage ?: draft.localUrl ?: draft.pickedUri else null
                                    } else null
                                    SpoilerThumbnail(source, "draft:${draft.key}:$source:spoiler")
                                    Text("SPOILER", Modifier.background(Blackout.copy(alpha = 0.8f)).padding(4.dp), color = Text, fontSize = 11.sp)
                                }
                                else if (thumbnail != null) AsyncImage(
                                    model = remember(thumbnail) { AttachmentImages.request(context, thumbnail, "draft:${draft.key}") },
                                    contentDescription = null, imageLoader = AttachmentImages.loader(context), modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Crop,
                                ) else if (draft.kind != "video") Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Icon(painterResource(if (draft.kind == "audio") R.drawable.lucide_play else R.drawable.lucide_file_text), null, Modifier.size(18.dp), tint = TextMuted)
                                    Text(when (draft.kind) { "image" -> "Image"; "audio" -> "Audio"; else -> "File" }, color = TextMuted, fontSize = 11.sp)
                                }
                            }
                            Column(Modifier.fillMaxSize()) {
                                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                                    if (draft.kind == "video" && !draft.spoiler) {
                                        Box(Modifier.size(32.dp).background(Blackout.copy(alpha = 0.32f), CircleShape), contentAlignment = Alignment.Center) {
                                            Icon(painterResource(R.drawable.lucide_play), "Video", Modifier.size(16.dp), tint = Text.copy(alpha = 0.72f))
                                        }
                                    }
                                }
                                if (status != null) Column(Modifier.fillMaxWidth().background(Blackout.copy(alpha = 0.9f)).padding(horizontal = 4.dp, vertical = if (draft.error != null && retry != null) 0.dp else 3.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                                    if (draft.error != null && retry != null) {
                                        Button({ retry(draft.key) }, Modifier.heightIn(min = 44.dp).semantics { contentDescription = "Retry uploading $accessibleName" }, enabled = enabled,
                                            colors = ButtonDefaults.buttonColors(containerColor = Action, contentColor = OnAction),
                                            shape = MaterialTheme.shapes.small, contentPadding = PaddingValues(horizontal = 4.dp)) {
                                            Text("Retry upload", fontSize = 10.sp)
                                        }
                                    } else Text(status, color = Text, fontSize = 10.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    // Progress glides between updates instead of jumping.
                                    val progress by animateFloatAsState(draft.progress.coerceIn(0f, 1f),
                                        if (still) snap() else tween(MOTION_BASE, easing = LinearEasing), label = "upload")
                                    if (busy) LinearProgressIndicator(
                                        progress = { progress }, modifier = Modifier.fillMaxWidth().padding(top = 4.dp).height(3.dp),
                                        color = Terracotta, trackColor = Border,
                                    )
                                }
                            }
                            toggleSpoiler?.let { toggle ->
                                TextButton({ toggle(draft.key) }, Modifier.align(Alignment.TopStart).width(tileSize - 48.dp).background(Blackout.copy(alpha = 0.9f))
                                    .semantics {
                                        contentDescription = "${if (draft.spoiler) "Remove spoiler from" else "Mark as spoiler:"} $accessibleName"
                                        stateDescription = if (draft.spoiler) "Spoiler enabled" else "Spoiler disabled"
                                    }, enabled = enabled, contentPadding = PaddingValues(0.dp)) {
                                    Text(if (draft.spoiler) "Spoiler ✓" else "Spoiler", fontSize = 10.sp)
                                }
                            }
                            Box(Modifier.align(Alignment.TopEnd)) {
                                Box(Modifier.size(48.dp).clip(MaterialTheme.shapes.small).clickable(enabled = enabled, role = Role.Button, onClick = { if (enabled) remove(draft.key) }).semantics { contentDescription = "Remove $accessibleName" }) {
                                    Box(Modifier.align(Alignment.TopEnd).padding(4.dp).size(24.dp).background(Blackout.copy(alpha = 0.9f), CircleShape), contentAlignment = Alignment.Center) {
                                        Icon(painterResource(R.drawable.lucide_x), null, Modifier.size(14.dp), tint = Text)
                                    }
                                }
                                DropdownMenu(menuOpen && enabled, { menuOpen = false }, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                                    DropdownMenuItem(
                                        text = { Text("Remove attachment", fontSize = 13.sp) },
                                        onClick = { menuOpen = false; if (enabled) remove(draft.key) }, enabled = enabled,
                                    )
                                }
                            }
                        }
                    }
                } }
            }
        }
    }
}
