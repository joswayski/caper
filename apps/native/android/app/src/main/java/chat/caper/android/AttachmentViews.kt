package chat.caper.android

import android.content.Context
import android.os.Build
import androidx.annotation.OptIn
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
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
import coil3.memory.MemoryCache
import coil3.network.HttpException
import coil3.network.okhttp.OkHttpNetworkFetcherFactory
import coil3.request.ImageRequest
import okhttp3.OkHttpClient
import okio.Path.Companion.toOkioPath

/**
 * One image loader for attachments. Caches are keyed by attachment ID, so re-signed URLs reuse
 * them. OkHttp adds `Accept-Encoding: gzip` itself, so a gzip-encoded response is decoded
 * transparently. Below API 31 the platform cannot decode AVIF, so a bundled decoder handles it.
 */
internal object AttachmentImages {
    @Volatile private var loader: ImageLoader? = null

    fun loader(context: Context): ImageLoader = loader ?: synchronized(this) {
        loader ?: context.applicationContext.let { app ->
            ImageLoader.Builder(app)
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

private fun Throwable.httpStatus(): Int? = (this as? HttpException)?.response?.code

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
 * device's own picked images, shown while the server processes them.
 */
@Composable internal fun MessageAttachments(
    attachments: List<ChatAttachment>,
    pending: Boolean,
    onLoadFailed: (ChatAttachment, Int?) -> Unit,
    progress: Map<String, Int> = emptyMap(),
    localPreviews: Map<String, String> = emptyMap(),
) {
    if (attachments.isEmpty()) return
    var viewing by remember { mutableStateOf<String?>(null) }
    var playing by remember { mutableStateOf<String?>(null) }
    val uriHandler = LocalUriHandler.current
    // The system browser downloads or shows the file and handles `Content-Encoding: gzip` itself.
    val open: (String) -> Unit = { url -> runCatching { uriHandler.openUri(url) } }
    Column(Modifier.padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        attachments.forEach { attachment ->
            val url = attachment.url
            when {
                attachment.unavailable -> FileCard(attachment, null)
                attachment.state == AttachmentState.FAILED -> FileCard(attachment, null, detail = "Couldn’t process this file", error = true)
                attachment.state == AttachmentState.PROCESSING -> ProcessingAttachment(
                    attachment, attachment.previewUrl ?: localPreviews[attachment.id], progress[attachment.id], onLoadFailed,
                )
                url == null -> FileCard(attachment, null)
                attachment.kind == "image" -> MediaFrame(attachment, if (pending) null else ({ viewing = attachment.id })) {
                    AttachmentImage(attachment, attachment.previewUrl ?: url, "display", Modifier.fillMaxSize(), ContentScale.Crop, onLoadFailed)
                }
                // Stored GIFs and animated images: muted, looping, inline, no controls.
                attachment.kind == "video" && attachment.animated -> MediaFrame(attachment, null) {
                    attachment.previewUrl?.let { AttachmentImage(attachment, it, "display", Modifier.fillMaxSize(), ContentScale.Crop, onLoadFailed) }
                    if (!pending) AnimatedVideo(attachment, url, onLoadFailed)
                }
                attachment.kind == "video" -> MediaFrame(attachment, if (pending) null else ({ playing = attachment.id })) {
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
    }
    attachments.firstOrNull { it.id == viewing && it.url != null && it.state == AttachmentState.READY }?.let { attachment ->
        ImageViewer(attachment, close = { viewing = null }, open = open, onLoadFailed = onLoadFailed)
    }
    attachments.firstOrNull { it.id == playing && it.url != null && it.state == AttachmentState.READY }?.let { attachment ->
        MediaPlayerDialog(attachment, close = { playing = null }, open = open, onLoadFailed = onLoadFailed)
    }
}

/** Placeholder while the media worker compresses the file: preview (if any), spinner and percent. */
@Composable private fun ProcessingAttachment(attachment: ChatAttachment, preview: String?, percent: Int?, onLoadFailed: (ChatAttachment, Int?) -> Unit) {
    val label = if (percent != null) "Processing… $percent%" else "Processing…"
    val visual = attachment.kind == "image" || attachment.kind == "video" || attachmentFrame(attachment.width, attachment.height) != null
    if (!visual) {
        FileCard(attachment, null, detail = label, busy = percent ?: -1)
        return
    }
    MediaFrame(attachment, null, description = "${attachment.name}, $label") {
        if (preview != null) AttachmentImage(attachment, preview, "processing", Modifier.fillMaxSize(), ContentScale.Crop, onLoadFailed)
        Box(Modifier.fillMaxSize().background(Blackout.copy(alpha = if (preview != null) 0.45f else 0f)))
        Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Spinner(percent, Modifier.size(28.dp))
            Text(label, color = Text, fontSize = 11.sp, fontWeight = FontWeight.Bold)
        }
    }
}

@Composable private fun Spinner(percent: Int?, modifier: Modifier) {
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
            playWhenReady = true
        }
    }
    DisposableEffect(player) { onDispose { player.release() } }
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
    attachment: ChatAttachment, onClick: (() -> Unit)?, description: String = attachment.name, content: @Composable BoxScope.() -> Unit,
) {
    val frame = attachmentFrame(attachment.width, attachment.height)
    val width: Dp = (frame?.first ?: 240).dp
    val ratio = frame?.let { it.first.toFloat() / it.second } ?: (4f / 3f)
    Box(
        Modifier.widthIn(max = width).fillMaxWidth().aspectRatio(ratio)
            .clip(MaterialTheme.shapes.small).background(Surface).border(1.dp, Border, MaterialTheme.shapes.small)
            .then(if (onClick != null) Modifier.clickable(role = Role.Button, onClickLabel = "Open ${attachment.name}", onClick = onClick) else Modifier)
            .semantics { contentDescription = description },
        content = content,
    )
}

@Composable private fun AttachmentImage(
    attachment: ChatAttachment, data: String, variant: String, modifier: Modifier, scale: ContentScale,
    onLoadFailed: (ChatAttachment, Int?) -> Unit,
) {
    val context = LocalContext.current
    val request = remember(data, attachment.id, variant) { AttachmentImages.request(context, data, "${attachment.id}:$variant") }
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
        Modifier.widthIn(max = 360.dp).fillMaxWidth().heightIn(min = 48.dp)
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
        Column(Modifier.fillMaxSize().background(Blackout)) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(title, Modifier.weight(1f), color = Text, fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                actions()
                IconButton(close, Modifier.semantics { contentDescription = "Close" }) { Icon(painterResource(R.drawable.lucide_x), null, tint = Text) }
            }
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center, content = content)
        }
    }
}

/** Full size. If the original cannot be decoded on this device, the WebP preview is shown instead. */
@Composable private fun ImageViewer(attachment: ChatAttachment, close: () -> Unit, open: (String) -> Unit, onLoadFailed: (ChatAttachment, Int?) -> Unit) {
    val url = attachment.url ?: return
    var usePreview by remember(url) { mutableStateOf(false) }
    val preview = attachment.previewUrl
    ViewerFrame(attachment.name, close, actions = { TextButton({ open(url) }) { Text("Open", color = Text) } }) {
        if (usePreview && preview != null) {
            AttachmentImage(attachment, preview, "display", Modifier.fillMaxSize(), ContentScale.Fit, onLoadFailed)
        } else {
            AttachmentImage(attachment, url, "original", Modifier.fillMaxSize(), ContentScale.Fit) { failed, status ->
                if (status == null && preview != null) usePreview = true
                onLoadFailed(failed, status)
            }
        }
    }
}

/** In-app video/audio playback; a failed load asks for fresh URLs once, then offers the browser. */
@OptIn(UnstableApi::class)
@Composable private fun MediaPlayerDialog(attachment: ChatAttachment, close: () -> Unit, open: (String) -> Unit, onLoadFailed: (ChatAttachment, Int?) -> Unit) {
    val url = attachment.url ?: return
    val context = LocalContext.current
    var failed by remember(url) { mutableStateOf(false) }
    val latest by rememberUpdatedState(attachment)
    val reportFailure by rememberUpdatedState(onLoadFailed)
    val player = remember(url) {
        ExoPlayer.Builder(context).build().apply {
            setMediaItem(MediaItem.fromUri(url))
            addListener(object : Player.Listener {
                override fun onPlayerError(error: PlaybackException) {
                    failed = true
                    reportFailure(latest, (error.cause as? HttpDataSource.InvalidResponseCodeException)?.responseCode)
                }
            })
            prepare()
            playWhenReady = true
        }
    }
    DisposableEffect(player) { onDispose { player.release() } }
    ViewerFrame(attachment.name, close, actions = { TextButton({ open(url) }) { Text("Open", color = Text) } }) {
        AndroidView(
            factory = { PlayerView(it).apply { useController = true; setShowBuffering(PlayerView.SHOW_BUFFERING_WHEN_PLAYING) } },
            update = { it.player = player },
            modifier = Modifier.fillMaxSize(),
        )
        if (failed) Text("This file could not be played.", Modifier.align(Alignment.TopCenter).padding(12.dp), color = ErrorText, fontSize = 12.sp)
    }
}

/** Composer chips: thumbnail, name, size or upload progress, and remove (web `DraftAttachments`). */
@Composable internal fun DraftAttachmentStrip(drafts: List<DraftAttachmentUi>, remove: (String) -> Unit) {
    if (drafts.isEmpty()) return
    val context = LocalContext.current
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(bottom = 8.dp).semantics { contentDescription = "Files to send" }, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        drafts.forEach { draft ->
            Surface(Modifier.width(232.dp), shape = MaterialTheme.shapes.small, color = Surface, border = BorderStroke(1.dp, if (draft.error != null) ErrorText.copy(alpha = 0.6f) else Border)) {
                Row(Modifier.padding(start = 8.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.size(40.dp).clip(MaterialTheme.shapes.extraSmall).background(SurfaceComposer), contentAlignment = Alignment.Center) {
                        val thumbnail = draft.thumbnail
                        if (thumbnail != null) AsyncImage(
                            model = remember(thumbnail) { AttachmentImages.request(context, thumbnail, "draft:${draft.key}") },
                            contentDescription = null, imageLoader = AttachmentImages.loader(context), modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Crop,
                        ) else Icon(painterResource(if (draft.kind == "video" || draft.kind == "audio") R.drawable.lucide_play else R.drawable.lucide_file_text), null, Modifier.size(18.dp), tint = TextMuted)
                    }
                    Column(Modifier.weight(1f)) {
                        Text(draft.name, color = Text, fontSize = 12.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        val status = draft.error ?: when {
                            draft.attachment != null -> AttachmentPolicy.sizeLabel(draft.sourceSize, draft.storedSize)
                            draft.compressing -> if (draft.progress > 0f) "Compressing… ${(draft.progress * 100).toInt()}%" else "Preparing…"
                            else -> "Uploading… ${(draft.progress * 100).toInt()}%"
                        }
                        Text(status, color = if (draft.error != null) ErrorText else TextMuted, fontSize = 10.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        if (draft.attachment == null && draft.error == null) LinearProgressIndicator(
                            progress = { draft.progress.coerceIn(0f, 1f) }, modifier = Modifier.fillMaxWidth().padding(top = 4.dp).height(3.dp),
                            color = Terracotta, trackColor = Border,
                        )
                    }
                    IconButton({ remove(draft.key) }, Modifier.size(40.dp).semantics { contentDescription = "Remove ${draft.name}" }) {
                        Icon(painterResource(R.drawable.lucide_x), null, Modifier.size(14.dp), tint = TextMuted)
                    }
                }
            }
        }
    }
}
