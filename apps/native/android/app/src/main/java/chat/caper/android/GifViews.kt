package chat.caper.android

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.graphics.Color as AndroidColor
import android.net.Uri
import android.os.Build
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.staggeredgrid.LazyVerticalStaggeredGrid
import androidx.compose.foundation.lazy.staggeredgrid.StaggeredGridCells
import androidx.compose.foundation.lazy.staggeredgrid.StaggeredGridItemSpan
import androidx.compose.foundation.lazy.staggeredgrid.rememberLazyStaggeredGridState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import chat.caper.android.data.KlipyAdRequest
import chat.caper.android.data.KlipyItem
import chat.caper.android.data.KlipyKind
import chat.caper.android.data.KlipyPage
import chat.caper.android.model.KlipyGif
import chat.caper.android.ui.*
import coil3.compose.AsyncImage
import coil3.gif.AnimatedImageDecoder
import coil3.gif.GifDecoder
import coil3.request.ImageRequest
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first

/**
 * KLIPY media straight from KLIPY's URLs (no Caper signer), on the attachment loader's caches.
 * Animated requests use the decoders that play animated WebP (API 28+) or GIF; a still request
 * shows a single frame. Below API 28 only GIF animates, so it is preferred there.
 */
private fun klipyAnimated(gif: String?, webp: String?): String? =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) webp ?: gif else gif ?: webp

private fun klipyRequest(context: Context, url: String, animate: Boolean): ImageRequest =
    ImageRequest.Builder(context).data(url).memoryCacheKey("klipy:$url" + if (animate) ":animated" else "").diskCacheKey("klipy:$url").apply {
        if (animate) decoderFactory(if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) AnimatedImageDecoder.Factory() else GifDecoder.Factory())
    }.build()

/**
 * A message's GIF under its text, at its aspect ratio and at most 300dp on the longer edge
 * (stickers 160dp, without a background). Reduced motion shows KLIPY's still, or the first frame.
 * A failed load leaves a quiet placeholder.
 */
@Composable internal fun MessageGif(gif: KlipyGif, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val still = reducedMotion()
    val edge = if (gif.sticker) 160f else 300f
    val scale = minOf(1f, edge / maxOf(gif.width, gif.height))
    val ratio = gif.width.toFloat() / gif.height
    val animated = klipyAnimated(gif.gif, gif.webp)
    val url = if (still) gif.still ?: animated else animated
    val animate = !still
    var failed by remember(url) { mutableStateOf(false) }
    val request = remember(url, animate) { url?.let { klipyRequest(context, it, animate) } }
    val shape = RoundedCornerShape(8.dp)
    Box(
        modifier.padding(top = 4.dp).widthIn(max = (gif.width * scale).coerceAtLeast(1f).dp).fillMaxWidth().aspectRatio(ratio)
            .then(if (gif.sticker && !failed) Modifier else Modifier.clip(shape).background(Surface).border(1.dp, Border, shape))
            .clearAndSetSemantics { contentDescription = if (failed) "${gif.label}, GIF unavailable" else gif.label; role = Role.Image },
        contentAlignment = Alignment.Center,
    ) {
        if (failed || request == null) Text("GIF unavailable", Modifier.padding(8.dp), color = TextMuted, fontSize = 12.sp)
        else AsyncImage(
            model = request, contentDescription = null, imageLoader = AttachmentImages.loader(context),
            modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Fit, onError = { failed = true },
        )
    }
}

/** The composer's GIF control, beside the emoji button; it opens the KLIPY picker. */
@Composable internal fun GifButton(
    enabled: Boolean,
    load: suspend (KlipyKind, String, Int, KlipyAdRequest?) -> KlipyPage,
    /** Sends at once; false when nothing was sent. The query is KLIPY's share trigger `q`. */
    send: (KlipyGif, KlipyKind, String) -> Boolean,
) {
    var open by rememberSaveable { mutableStateOf(false) }
    val tint = if (enabled) TextMuted else TextMuted.copy(alpha = 0.4f)
    IconButton({ open = true }, Modifier.size(48.dp).semantics { contentDescription = "Send a GIF" }, enabled = enabled) {
        Box(Modifier.border(1.5.dp, tint, RoundedCornerShape(4.dp)).padding(horizontal = 3.dp, vertical = 1.dp)) {
            Text("GIF", color = tint, fontSize = 10.sp, lineHeight = 12.sp, fontWeight = FontWeight.Bold)
        }
    }
    if (open) GifPicker(load, onDismiss = { open = false }) { gif, kind, query ->
        if (send(gif, kind, query)) open = false
    }
}

/** The fields KLIPY recommends for ads that need no permission prompt; [gridWidth] is in dp. */
private fun adRequest(context: Context, gridWidth: Int): KlipyAdRequest {
    val metrics = context.resources.displayMetrics
    return KlipyAdRequest(
        gridWidth, Build.VERSION.RELEASE.orEmpty(), Build.MANUFACTURER.orEmpty(), Build.MODEL.orEmpty(),
        metrics.widthPixels, metrics.heightPixels, metrics.density, java.util.Locale.getDefault().language,
    )
}

/**
 * KLIPY's picker: GIFs and Stickers, "Search KLIPY" (KLIPY requires that placeholder), trending
 * until something is typed, searches 300 ms after typing stops, and pages while `has_next`.
 * Results keep KLIPY's order and mix, ads included. Tapping one sends it.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable private fun GifPicker(
    load: suspend (KlipyKind, String, Int, KlipyAdRequest?) -> KlipyPage,
    onDismiss: () -> Unit,
    select: (KlipyGif, KlipyKind, String) -> Unit,
) {
    val context = LocalContext.current
    val focus = LocalFocusManager.current
    val still = reducedMotion()
    var kind by rememberSaveable { mutableStateOf(KlipyKind.GIFS) }
    var query by rememberSaveable { mutableStateOf("") }
    var results by remember { mutableStateOf<List<KlipyItem>>(emptyList()) }
    // The tab and search text the shown results answer: the share trigger's `q`.
    var shown by remember { mutableStateOf<Pair<KlipyKind, String>?>(null) }
    var hasNext by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(true) }
    var failed by remember { mutableStateOf(false) }
    var retries by remember { mutableIntStateOf(0) }
    var gridWidth by remember { mutableIntStateOf(0) }
    val grid = rememberLazyStaggeredGridState()
    val height = with(LocalDensity.current) { LocalWindowInfo.current.containerSize.height.toDp() } * 0.8f

    // One loop per tab and search: the key change cancels it, which is also the debounce.
    LaunchedEffect(kind, query, gridWidth > 0) {
        if (gridWidth == 0) return@LaunchedEffect
        val search = query.trim()
        if (shown?.first != kind) results = emptyList()
        failed = false; loading = true
        if (search.isNotEmpty()) delay(300)
        var page = 1
        while (true) {
            loading = true; failed = false
            val result = try { load(kind, search, page, adRequest(context, gridWidth)) }
                catch (cancelled: CancellationException) { throw cancelled } catch (_: Throwable) { null }
            loading = false
            if (result == null) {
                failed = true
                val seen = retries
                snapshotFlow { retries }.first { it != seen }
                continue
            }
            if (page == 1) { results = result.items; shown = kind to search; grid.scrollToItem(0) } else results = results + result.items
            hasNext = result.hasNext && result.items.isNotEmpty()
            if (!hasNext) break
            page++
            // The next page loads as the grid nears its end.
            snapshotFlow {
                val info = grid.layoutInfo
                (info.visibleItemsInfo.lastOrNull()?.index ?: -1) >= info.totalItemsCount - 8
            }.first { it }
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = Surface, contentColor = Text,
    ) {
        Column(Modifier.fillMaxWidth().height(height).padding(horizontal = 12.dp).navigationBarsPadding(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("GIFs", Modifier.weight(1f).semantics { heading() }, fontSize = 15.sp, fontWeight = FontWeight.Bold)
                Text("Powered by KLIPY", color = TextMuted, fontSize = 11.sp)
            }
            Row(Modifier.fillMaxWidth().border(1.dp, Border, MaterialTheme.shapes.small).padding(3.dp), horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                listOf(KlipyKind.GIFS to "GIFs", KlipyKind.STICKERS to "Stickers").forEach { (value, title) ->
                    val active = kind == value
                    Box(
                        Modifier.weight(1f).heightIn(min = 40.dp).clip(RoundedCornerShape(6.dp))
                            .background(if (active) SurfaceRaised else Color.Transparent)
                            .clickable(onClickLabel = title) { kind = value }
                            .semantics { selected = active; role = Role.Tab },
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(title, color = if (active) Text else TextMuted, fontSize = 13.sp, fontWeight = if (active) FontWeight.Bold else FontWeight.Normal)
                        if (active) Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth(0.4f).height(2.dp).background(TerracottaBright))
                    }
                }
            }
            OutlinedTextField(
                query, { query = it }, Modifier.fillMaxWidth(), placeholder = { Text("Search KLIPY") }, singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search), keyboardActions = KeyboardActions(onSearch = { focus.clearFocus() }),
                shape = MaterialTheme.shapes.small,
            )
            BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                val width = maxWidth.value.toInt()
                LaunchedEffect(width) { gridWidth = width }
                LazyVerticalStaggeredGrid(
                    StaggeredGridCells.Fixed(2), Modifier.fillMaxSize(), state = grid,
                    contentPadding = PaddingValues(bottom = 12.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp), verticalItemSpacing = 6.dp,
                ) {
                    // KLIPY may repeat an item across pages, so positions (not slugs) are the keys.
                    items(results.size, span = { if (results[it] is KlipyItem.Ad) StaggeredGridItemSpan.FullLine else StaggeredGridItemSpan.SingleLane }) { index ->
                        when (val item = results[index]) {
                            is KlipyItem.Ad -> KlipyAdTile(item)
                            is KlipyItem.Result -> GifTile(item, item.gif.sticker, still) {
                                select(item.gif, shown?.first ?: kind, shown?.second.orEmpty())
                            }
                        }
                    }
                    item(span = StaggeredGridItemSpan.FullLine) {
                        Box(Modifier.fillMaxWidth().heightIn(min = 72.dp), contentAlignment = Alignment.Center) {
                            when {
                                failed -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                                    Text("GIFs couldn’t load.", color = ErrorText, fontSize = 13.sp)
                                    TextButton({ retries++ }, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Retry") }
                                }
                                loading -> CircularProgressIndicator(Modifier.size(24.dp).semantics { contentDescription = "Loading GIFs" }, color = TextMuted, strokeWidth = 2.dp)
                                results.isEmpty() -> Text("No results", color = TextMuted, fontSize = 13.sp)
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable private fun GifTile(item: KlipyItem.Result, sticker: Boolean, still: Boolean, onClick: () -> Unit) {
    val context = LocalContext.current
    val url = if (still) item.tile.still ?: item.tile.url else item.tile.url
    val request = remember(url, still) { klipyRequest(context, url, animate = !still) }
    var failed by remember(url) { mutableStateOf(false) }
    val ratio = (item.tile.width.toFloat() / item.tile.height).coerceIn(0.5f, 2f)
    Box(
        Modifier.fillMaxWidth().aspectRatio(ratio).clip(MaterialTheme.shapes.small)
            .background(if (sticker) Color.Transparent else SurfaceRaised)
            .clickable(role = Role.Button, onClickLabel = "Send", onClick = onClick)
            .semantics { contentDescription = item.gif.label },
        contentAlignment = Alignment.Center,
    ) {
        if (failed) Text("GIF unavailable", color = TextMuted, fontSize = 11.sp)
        else AsyncImage(
            model = request, contentDescription = null, imageLoader = AttachmentImages.loader(context),
            modifier = Modifier.fillMaxSize(), contentScale = if (sticker) ContentScale.Fit else ContentScale.Crop, onError = { failed = true },
        )
    }
}

/** Opens an ad's link in the system browser; Caper never navigates to it. */
private fun openAdLink(context: Context, uri: Uri) {
    if (uri.scheme != "https" && uri.scheme != "http") return
    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
}

/**
 * KLIPY's ad HTML in an isolated WebView: scripts on, but no file or content access, no storage
 * or cookies, no popups and no bridge to the app. Only the ad's own subframes load inside it; a
 * tapped link opens in the system browser and nothing else navigates.
 */
@SuppressLint("SetJavaScriptEnabled")
private fun adWebView(context: Context): WebView = WebView(context).apply {
    // The app has no other WebView, so turning cookies off costs nothing and nothing persists.
    CookieManager.getInstance().setAcceptCookie(false)
    CookieManager.getInstance().setAcceptThirdPartyCookies(this, false)
    settings.javaScriptEnabled = true
    settings.allowFileAccess = false
    settings.allowContentAccess = false
    settings.domStorageEnabled = false
    settings.setGeolocationEnabled(false)
    settings.javaScriptCanOpenWindowsAutomatically = false
    settings.setSupportMultipleWindows(false)
    settings.mediaPlaybackRequiresUserGesture = true
    settings.cacheMode = WebSettings.LOAD_NO_CACHE
    isVerticalScrollBarEnabled = false
    isHorizontalScrollBarEnabled = false
    setBackgroundColor(AndroidColor.TRANSPARENT)
    webViewClient = object : WebViewClient() {
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
            if (!request.isForMainFrame) return false
            if (request.hasGesture()) openAdLink(view.context, request.url)
            return true
        }
    }
}

/** A full-width "Sponsored" row, sized to the ad or else 100dp tall. */
@Composable private fun KlipyAdTile(ad: KlipyItem.Ad) {
    Column(Modifier.fillMaxWidth().semantics(mergeDescendants = false) { contentDescription = "Sponsored" }, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Sponsored", color = TextMuted, fontSize = 10.sp, fontWeight = FontWeight.Bold)
        AndroidView(
            factory = { adWebView(it) },
            modifier = Modifier.fillMaxWidth().height((ad.height ?: 100).dp).clip(MaterialTheme.shapes.small),
            update = { view ->
                if (view.tag != ad.html) {
                    view.tag = ad.html
                    view.loadDataWithBaseURL(null, ad.html, "text/html", "utf-8", null)
                }
            },
            onRelease = { view -> view.stopLoading(); view.destroy() },
        )
    }
}

