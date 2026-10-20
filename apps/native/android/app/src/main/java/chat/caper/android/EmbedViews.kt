package chat.caper.android

import android.annotation.SuppressLint
import android.view.View
import android.webkit.CookieManager
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebStorage
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.annotation.OptIn
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.media3.common.AudioAttributes
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import chat.caper.android.data.embedFrame
import chat.caper.android.data.embedPlayUrl
import chat.caper.android.data.embedPlayerNavigation
import chat.caper.android.data.linkRanges
import chat.caper.android.data.previewLink
import chat.caper.android.model.ChatEmbed
import chat.caper.android.model.EmbedBlock
import chat.caper.android.model.isWebLink
import chat.caper.android.ui.*
import coil3.compose.AsyncImage
import java.time.Instant
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.delay

/** The composer's card for the draft's first link; [removed] tells whether a text is sent without cards. */
internal class LinkPreview(val embed: ChatEmbed?, val remove: () -> Unit, val removed: (String) -> Boolean)

/**
 * Web's `useLinkPreview`: 400 ms after the draft's first link ([previewLink]) settles, [fetch]
 * asks for its card. Removing it (×) sends the message without cards; that lasts while the
 * same link stays first in the draft, and a sent or cleared draft starts over. [scope] is the
 * conversation, so another one starts fresh.
 */
@Composable internal fun rememberLinkPreview(draft: String, scope: Any?, fetch: suspend (String) -> ChatEmbed?): LinkPreview {
    val link = remember(draft) { previewLink(draft) }
    var shown by remember(scope) { mutableStateOf<Pair<String, ChatEmbed?>?>(null) }
    var removed by remember(scope) { mutableStateOf<String?>(null) }
    val latestFetch by rememberUpdatedState(fetch)
    val empty = draft.isEmpty()
    LaunchedEffect(empty) { if (empty) removed = null }
    LaunchedEffect(link, removed, scope) {
        if (link == null || link == removed) return@LaunchedEffect
        delay(400)
        shown = link to latestFetch(link)
    }
    val embed = shown?.takeIf { link != null && link != removed && it.first == link }?.second
    return LinkPreview(embed, remove = { removed = link }, removed = { text -> removed != null && previewLink(text) == removed })
}

/** Inner media corners, inside the card's 8dp ones (web's `.chat-embed-media`). */
private val EmbedMediaShape = RoundedCornerShape(6.dp)

/**
 * A message's link preview cards (docs/media.md "Link embeds"), drawn from blocks in order;
 * unknown blocks were skipped when parsed. Cards that arrive while the row is on screen settle
 * in ([arriving]: every card, for the composer's preview); ones already there when it appears do
 * not. [onRemove] adds the composer's ×, which sends the message without cards.
 */
@Composable internal fun MessageEmbeds(embeds: List<ChatEmbed>, onRemove: (() -> Unit)? = null, arriving: Boolean = false) {
    // What the row showed when it first appeared, so only cards added since then animate.
    val shown = remember { if (arriving) emptySet() else embeds.map { it.url }.toSet() }
    if (embeds.isEmpty()) return
    Box(Modifier.padding(top = 6.dp).widthIn(max = 432.dp).fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            embeds.forEach { embed ->
                key(embed.url) {
                    EmbedCard(embed, if (embed.url in shown) Modifier else Modifier.entrance(MOTION_BASE, dy = 4.dp), removable = onRemove != null)
                }
            }
        }
        onRemove?.let { remove ->
            Box(
                Modifier.align(Alignment.TopEnd).padding(4.dp).size(32.dp).clip(MaterialTheme.shapes.small)
                    .clickable(role = Role.Button, onClickLabel = "Remove link preview", onClick = remove)
                    .semantics { contentDescription = "Remove link preview" },
                contentAlignment = Alignment.Center,
            ) {
                Box(Modifier.size(24.dp).background(SurfaceRaised, MaterialTheme.shapes.small).border(1.dp, Border, MaterialTheme.shapes.small), contentAlignment = Alignment.Center) {
                    Icon(painterResource(R.drawable.lucide_x), null, Modifier.size(14.dp), tint = TextMuted)
                }
            }
        }
    }
}

@Composable private fun EmbedCard(embed: ChatEmbed, modifier: Modifier, removable: Boolean) {
    val uriHandler = LocalUriHandler.current
    val heading = embed.blocks.firstNotNullOfOrNull { (it as? EmbedBlock.Title)?.text }
        ?: embed.blocks.firstNotNullOfOrNull { (it as? EmbedBlock.Author)?.name } ?: embed.site.name
    val thumbnail = embed.blocks.firstNotNullOfOrNull { block -> (block as? EmbedBlock.Images)?.takeIf { it.thumbnail } }
    Row(
        modifier.fillMaxWidth().clip(MaterialTheme.shapes.small).background(Surface).border(1.dp, Border, MaterialTheme.shapes.small)
            .padding(start = 12.dp, end = if (removable) 36.dp else 12.dp, top = 10.dp, bottom = 12.dp)
            .semantics { contentDescription = "${embed.site.name}: $heading" },
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(
                Modifier.clickable(role = Role.Button, onClickLabel = "Open ${embed.site.domain}") { openExternalLink(uriHandler, embed.url) },
                horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(embed.site.name, Modifier.weight(1f, fill = false), color = TextMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                // The domain comes from the posted link, so a page can't pass itself off as another site.
                if (embed.site.name != embed.site.domain) Text(embed.site.domain, color = TextMuted.copy(alpha = 0.75f), fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            embed.blocks.forEach { block ->
                when (block) {
                    is EmbedBlock.Author -> EmbedAuthor(block, uriHandler)
                    is EmbedBlock.Title -> Text(
                        block.text,
                        if (block.url == null) Modifier else Modifier.clickable(role = Role.Button) { openExternalLink(uriHandler, block.url) },
                        color = Text, fontSize = 14.sp, lineHeight = 18.sp, fontWeight = FontWeight.Bold,
                    )
                    is EmbedBlock.Text -> EmbedText(block.text)
                    is EmbedBlock.Images -> if (!block.thumbnail) EmbedImages(block, uriHandler)
                    is EmbedBlock.Video -> EmbedVideo(block, heading)
                    is EmbedBlock.Player -> EmbedPlayer(block, heading, embed.url, uriHandler)
                    is EmbedBlock.Footer -> {
                        val line = listOfNotNull(block.text?.takeIf { it.isNotEmpty() }, embedFooterDate(block.timestamp)).joinToString(" · ")
                        if (line.isNotEmpty()) Text(line, color = TextMuted, fontSize = 11.sp)
                    }
                }
            }
        }
        thumbnail?.items?.first()?.let { image ->
            EmbedPicture(
                image.url, image.alt,
                Modifier.size(80.dp).clip(EmbedMediaShape).background(SurfaceRaised)
                    .clickable(role = Role.Button, onClickLabel = "Open ${embed.site.domain}") { openExternalLink(uriHandler, embed.url) },
            )
        }
    }
}

/** A 20dp round avatar, the bold name and the muted handle; opens the author's link when it has one. */
@Composable private fun EmbedAuthor(block: EmbedBlock.Author, uriHandler: UriHandler) {
    Row(
        if (block.url == null) Modifier else Modifier.clickable(role = Role.Button) { openExternalLink(uriHandler, block.url) },
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        block.icon?.let { EmbedPicture(it, null, Modifier.size(20.dp).clip(CircleShape).background(SurfaceRaised)) }
        Text(block.name, Modifier.weight(1f, fill = false), color = Text, fontSize = 13.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
        block.handle?.takeIf { it.isNotEmpty() }?.let { Text(it, color = TextMuted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
    }
}

/** Body text: plain, line breaks kept, links found by the shared rules (`data/Links.kt`), at most 8 lines. */
@Composable private fun EmbedText(text: String) {
    val links = remember(text) { linkRanges(text) }
    if (links.isEmpty()) return Text(text, color = Text, fontSize = 13.sp, lineHeight = 18.sp, maxLines = 8, overflow = TextOverflow.Ellipsis)
    val uriHandler = LocalUriHandler.current
    val annotated = remember(text, links, uriHandler) { messageText(text, emptyList(), links = links, openLink = { openExternalLink(uriHandler, it) }) }
    MessageLinkText(annotated, color = Text, fontSize = 13.sp, lineHeight = 18.sp, maxLines = 8)
}

/** One image sized like web's `fit`; two to four in a two-column grid that keeps to the same 300dp height. */
@Composable private fun EmbedImages(block: EmbedBlock.Images, uriHandler: UriHandler) {
    val items = block.items
    @Composable fun Cell(index: Int, modifier: Modifier) {
        val image = items[index]
        EmbedPicture(image.url, image.alt, modifier.clip(EmbedMediaShape).background(SurfaceRaised)
            .clickable(role = Role.Button, onClickLabel = "Open image") { openExternalLink(uriHandler, image.url) })
    }
    val gap = 4.dp
    when (items.size) {
        1 -> {
            val frame = embedFrame(items[0].width, items[0].height)
            Cell(0, Modifier.widthIn(max = frame.width.dp).fillMaxWidth().aspectRatio(frame.aspectRatio))
        }
        2 -> Row(Modifier.fillMaxWidth().height(220.dp), horizontalArrangement = Arrangement.spacedBy(gap)) {
            Cell(0, Modifier.weight(1f).fillMaxHeight()); Cell(1, Modifier.weight(1f).fillMaxHeight())
        }
        3 -> Row(Modifier.fillMaxWidth().height(148.dp * 2 + gap), horizontalArrangement = Arrangement.spacedBy(gap)) {
            Cell(0, Modifier.weight(1f).fillMaxHeight())
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(gap)) {
                Cell(1, Modifier.fillMaxWidth().weight(1f)); Cell(2, Modifier.fillMaxWidth().weight(1f))
            }
        }
        else -> Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(gap)) {
            listOf(0, 2).forEach { first ->
                Row(Modifier.fillMaxWidth().height(148.dp), horizontalArrangement = Arrangement.spacedBy(gap)) {
                    Cell(first, Modifier.weight(1f).fillMaxHeight()); Cell(first + 1, Modifier.weight(1f).fillMaxHeight())
                }
            }
        }
    }
}

/** An embed image, avatar or poster through the attachment image loader; these signed URLs are their own cache key. */
@Composable private fun EmbedPicture(url: String, alt: String?, modifier: Modifier, scale: ContentScale = ContentScale.Crop) {
    val context = LocalContext.current
    val request = remember(url) { AttachmentImages.request(context, url, "embed:$url") }
    AsyncImage(model = request, contentDescription = alt?.takeIf { it.isNotEmpty() }, imageLoader = AttachmentImages.loader(context), modifier = modifier, contentScale = scale)
}

/** The eggshell play button over a poster (web's `.chat-embed-play`). */
@Composable private fun BoxScope.PlayBadge() {
    Box(Modifier.align(Alignment.Center).size(52.dp).background(Action, CircleShape), contentAlignment = Alignment.Center) {
        Icon(painterResource(R.drawable.lucide_play), null, Modifier.size(22.dp), tint = OnAction)
    }
}

/**
 * A direct MP4/WebM in the app's own player: Play shows controls; a `gif` loops muted without
 * controls from the start, and stays a still poster (Play starts it) when animations are off.
 */
@Composable private fun EmbedVideo(block: EmbedBlock.Video, title: String) {
    val frame = embedFrame(block.width, block.height)
    val still = reducedMotion()
    var started by remember(block.url) { mutableStateOf(false) }
    val playing = started || (block.gif && !still)
    Box(
        Modifier.widthIn(max = frame.width.dp).fillMaxWidth().aspectRatio(frame.aspectRatio).clip(EmbedMediaShape).background(SurfaceRaised)
            .then(if (playing) Modifier else Modifier.clickable(role = Role.Button, onClickLabel = "Play $title") { started = true })
            .semantics { contentDescription = title },
    ) {
        block.poster?.let { EmbedPicture(it, null, Modifier.fillMaxSize()) }
        if (playing) EmbedVideoPlayer(block.url, block.gif)
        else {
            PlayBadge()
            durationLabel(block.durationMs?.toLong())?.let {
                Text(it, Modifier.align(Alignment.BottomEnd).padding(6.dp).background(Blackout.copy(alpha = 0.72f), MaterialTheme.shapes.extraSmall)
                    .padding(horizontal = 5.dp, vertical = 2.dp), color = Text, fontSize = 10.sp, fontWeight = FontWeight.Bold)
            }
        }
    }
}

/** Plays while the app is visible and pauses when it is stopped, like attachment videos; released with the row. */
@OptIn(UnstableApi::class)
@Composable private fun BoxScope.EmbedVideoPlayer(url: String, gif: Boolean) {
    val context = LocalContext.current
    var failed by remember(url) { mutableStateOf(false) }
    val player = remember(url) {
        ExoPlayer.Builder(context).build().apply {
            // A GIF is silent, so it leaves other apps' audio alone.
            setAudioAttributes(AudioAttributes.DEFAULT, !gif)
            if (gif) { volume = 0f; repeatMode = Player.REPEAT_MODE_ONE }
            setMediaItem(MediaItem.fromUri(url))
            addListener(object : Player.Listener {
                override fun onPlayerError(error: PlaybackException) { failed = true }
            })
            prepare()
        }
    }
    DisposableEffect(player) { onDispose { player.release() } }
    LifecycleStartEffect(player) {
        player.playWhenReady = true
        onStopOrDispose { player.pause() }
    }
    if (failed) return Text("This video can’t be played here.", Modifier.align(Alignment.Center).padding(12.dp), color = TextMuted, fontSize = 12.sp)
    AndroidView(
        factory = {
            PlayerView(it).apply {
                useController = !gif
                setShowBuffering(PlayerView.SHOW_BUFFERING_WHEN_PLAYING)
                setShutterBackgroundColor(android.graphics.Color.TRANSPARENT)
                if (gif) resizeMode = androidx.media3.ui.AspectRatioFrameLayout.RESIZE_MODE_ZOOM
            }
        },
        update = { it.player = player },
        modifier = Modifier.matchParentSize(),
    )
}

/**
 * A YouTube, Vimeo or Twitch player: the poster with Play, which loads the block's `play` page
 * (Caper's embed host only, see [embedPlayUrl]) in place. Without a usable page, Play opens the
 * link in the browser instead.
 */
@Composable private fun EmbedPlayer(block: EmbedBlock.Player, title: String, link: String, uriHandler: UriHandler) {
    val frame = embedFrame(block.width, block.height)
    val page = remember(block.play) { embedPlayUrl(block.play) }
    var playing by remember(block.provider, block.id) { mutableStateOf(false) }
    Box(
        Modifier.widthIn(max = frame.width.dp).fillMaxWidth().aspectRatio(frame.aspectRatio).clip(EmbedMediaShape).background(Blackout)
            .then(if (playing && page != null) Modifier else Modifier.clickable(role = Role.Button, onClickLabel = "Play $title") {
                if (page != null) playing = true else openExternalLink(uriHandler, link)
            })
            .semantics { contentDescription = title },
    ) {
        if (playing && page != null) EmbedPlayerPage(page, uriHandler, Modifier.matchParentSize())
        else {
            block.poster?.let { EmbedPicture(it, null, Modifier.fillMaxSize()) }
            PlayBadge()
        }
    }
}

/**
 * Caper's player page in a web view. The providers' players need script, so JavaScript is on,
 * but there is no bridge to app code (no `addJavascriptInterface`), no file or content access,
 * no third-party cookies, and every top-level navigation away from the player host opens in the
 * system browser. Players keep settings in web storage, so it is on while playing; cookies, web
 * storage and caches are cleared when it closes. The app's own requests never use the web view's
 * cookie store.
 */
@SuppressLint("SetJavaScriptEnabled")
@Composable private fun EmbedPlayerPage(url: String, uriHandler: UriHandler, modifier: Modifier) {
    val latestHandler by rememberUpdatedState(uriHandler)
    AndroidView(
        factory = { context ->
            // A device whose WebView is missing or updating plays it in the browser instead.
            val web = runCatching { WebView(context) }.getOrNull()
            if (web == null) View(context).also { openExternalLink(latestHandler, url) }
            else web.apply {
                setBackgroundColor(Blackout.toArgb())
                settings.apply {
                    javaScriptEnabled = true
                    domStorageEnabled = true
                    allowFileAccess = false
                    allowContentAccess = false
                    cacheMode = WebSettings.LOAD_NO_CACHE
                    mediaPlaybackRequiresUserGesture = false
                    javaScriptCanOpenWindowsAutomatically = false
                    setSupportMultipleWindows(false)
                    setGeolocationEnabled(false)
                }
                CookieManager.getInstance().setAcceptThirdPartyCookies(this, false)
                webChromeClient = WebChromeClient()
                webViewClient = object : WebViewClient() {
                    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                        // The provider's player frame loads inside the page; only top-level navigations leave it.
                        if (!request.isForMainFrame) return false
                        val target = request.url.toString()
                        if (embedPlayerNavigation(target)) return false
                        if (isWebLink(target)) openExternalLink(latestHandler, target)
                        return true
                    }
                }
                loadUrl(url)
            }
        },
        onRelease = { view ->
            (view as? WebView)?.apply {
                stopLoading()
                clearHistory()
                clearCache(true)
                destroy()
            }
            CookieManager.getInstance().removeAllCookies(null)
            WebStorage.getInstance().deleteAllData()
        },
        modifier = modifier,
    )
}

private val FooterDate: DateTimeFormatter = DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM)

/** A footer timestamp as a medium date in this device's zone and locale, or null when it isn't a time. */
internal fun embedFooterDate(timestamp: String?, zone: ZoneId = ZoneId.systemDefault()): String? {
    if (timestamp.isNullOrEmpty()) return null
    val instant = runCatching { OffsetDateTime.parse(timestamp).toInstant() }.recoverCatching { Instant.parse(timestamp) }.getOrNull() ?: return null
    return FooterDate.format(instant.atZone(zone))
}
