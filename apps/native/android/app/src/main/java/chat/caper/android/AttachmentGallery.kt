package chat.caper.android

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.database.ContentObserver
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.provider.Settings
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import chat.caper.android.data.AttachmentPolicy
import chat.caper.android.ui.Action
import chat.caper.android.ui.OnAction
import chat.caper.android.ui.Border
import chat.caper.android.ui.Surface
import chat.caper.android.ui.SurfaceRaised
import chat.caper.android.ui.TextMuted
import chat.caper.android.ui.Text as TextPrimary
import chat.caper.android.ui.Terracotta as Accent
import kotlinx.coroutines.*

/** Ordered, bounded selection: selected items remain removable at capacity. */
internal fun <T> toggleGallerySelection(selected: List<T>, item: T, capacity: Int): List<T> =
    if (item in selected) selected - item else if (selected.size < capacity.coerceAtLeast(0)) selected + item else selected

internal fun <T> pruneGallerySelection(selected: List<T>, accessible: (T) -> Boolean): List<T> =
    selected.filter(accessible)

private data class GalleryItem(val uri: Uri, val id: Long, val date: Long, val video: Boolean, val duration: Long)
private val collection: Uri get() = MediaStore.Files.getContentUri("external")
internal fun galleryPermissions(sdk: Int): Array<String> = when {
    sdk >= 34 -> arrayOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VIDEO, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED)
    sdk >= 33 -> arrayOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VIDEO)
    else -> arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
}
internal enum class GalleryAccess { Denied, Limited, Full }
internal fun galleryAccess(sdk: Int, granted: (String) -> Boolean): GalleryAccess {
    val requested = galleryPermissions(sdk)
    val full = if (sdk >= 33) requested.take(2).all(granted) else granted(requested.first())
    return if (full) GalleryAccess.Full else if (requested.any(granted)) GalleryAccess.Limited else GalleryAccess.Denied
}
private fun permissions() = galleryPermissions(Build.VERSION.SDK_INT)
private fun Context.granted(permission: String) = checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
private fun Context.access() = galleryAccess(Build.VERSION.SDK_INT) { granted(it) } != GalleryAccess.Denied
private fun Context.fullAccess() = galleryAccess(Build.VERSION.SDK_INT) { granted(it) } == GalleryAccess.Full

/** Cancellation reaches the provider, not just the coroutine waiting for it. */
private val thumbnailDispatcher = Dispatchers.IO.limitedParallelism(4)
private suspend fun <T> providerWork(dispatcher: CoroutineDispatcher = Dispatchers.IO, block: (CancellationSignal) -> T): T = coroutineScope {
    val signal = CancellationSignal()
    val cancellation = launch(start = CoroutineStart.UNDISPATCHED) {
        try { awaitCancellation() } finally { signal.cancel() }
    }
    try { withContext(dispatcher) { signal.throwIfCanceled(); block(signal) } } finally { cancellation.cancel() }
}

private suspend fun page(context: Context, after: GalleryItem?): List<GalleryItem> = providerWork { signal ->
    val result = mutableListOf<GalleryItem>()
    val filter = "media_type IN (1,3)" + if (after != null) " AND (date_added < ? OR (date_added = ? AND _id < ?))" else ""
    val args = after?.let { arrayOf(it.date.toString(), it.date.toString(), it.id.toString()) }
    val columns = arrayOf("_id", "date_added", "media_type", "duration")
    val cursor = if (Build.VERSION.SDK_INT >= 26) {
        context.contentResolver.query(collection, columns, android.os.Bundle().apply {
            putString(android.content.ContentResolver.QUERY_ARG_SQL_SELECTION, filter)
            putStringArray(android.content.ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, args)
            putString(android.content.ContentResolver.QUERY_ARG_SQL_SORT_ORDER, "date_added DESC, _id DESC")
            putInt(android.content.ContentResolver.QUERY_ARG_LIMIT, 60)
        }, signal)
    } else null
    cursor?.use {
        while (result.size < 60 && it.moveToNext()) {
            signal.throwIfCanceled()
            val id = it.getLong(0)
            val video = it.getInt(2) == 3
            val uri = ContentUris.withAppendedId(if (video) MediaStore.Video.Media.EXTERNAL_CONTENT_URI else MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id)
            result += GalleryItem(uri, id, it.getLong(1), video, it.getLong(3))
        }
    }
    result
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun AttachmentGallery(remaining: Int, dismiss: () -> Unit, files: () -> Unit, add: (List<Uri>) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var revision by remember { mutableIntStateOf(0) }
    var selected by remember { mutableStateOf(emptyList<Uri>()) }
    var items by remember { mutableStateOf(emptyList<GalleryItem>()) }
    var allowed by remember { mutableStateOf(context.access()) }
    var full by remember { mutableStateOf(context.fullAccess()) }
    var loading by remember { mutableStateOf(true) }
    var end by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var loadFailed by remember { mutableStateOf(false) }
    var next by remember { mutableIntStateOf(0) }
    var loadedRevision by remember { mutableIntStateOf(-1) }
    var loadedCapacity by remember { mutableIntStateOf(-1) }
    val grid = rememberLazyGridState()
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { revision++ }
    DisposableEffect(lifecycle, context, allowed) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_RESUME) revision++ }
        val changes = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) { revision++ }
        }
        lifecycle.addObserver(observer)
        val registered = runCatching { context.contentResolver.registerContentObserver(collection, true, changes) }.isSuccess
        onDispose {
            lifecycle.removeObserver(observer)
            if (registered) context.contentResolver.unregisterContentObserver(changes)
        }
    }
    LaunchedEffect(revision, remaining, next) {
        val refresh = loadedRevision != revision || loadedCapacity != remaining
        if (!refresh && (end || !allowed)) return@LaunchedEffect
        if (refresh) {
            allowed = context.access(); full = context.fullAccess()
            items = emptyList(); end = false; error = null; loadFailed = false
        }
        loading = true
        try {
            if (refresh) {
                val accessible = withContext(Dispatchers.IO) {
                    pruneGallerySelection(selected) { uri ->
                        runCatching { context.contentResolver.openAssetFileDescriptor(uri, "r")?.use { true } == true }.getOrDefault(false)
                    }
                }
                if (accessible != selected) error = "Some selected media is no longer accessible. Review your selection."
                selected = accessible
            }
            if (allowed) {
                val more = page(context, items.lastOrNull())
                items = items + more; end = more.size < 60
            }
            loadedRevision = revision; loadedCapacity = remaining
        } catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) { error = "Couldn’t read available media. Review access or use Files."; loadFailed = true }
        finally { loading = false }
    }
    val nearEnd by remember { derivedStateOf { (grid.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: -1) >= items.size - 12 } }
    LaunchedEffect(nearEnd, items.size, loading) { if (nearEnd && !loading && !end && allowed && !loadFailed) next++ }
    var submitting by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val currentRemaining by rememberUpdatedState(remaining)
    val currentAdd by rememberUpdatedState(add)
    ModalBottomSheet(onDismissRequest = dismiss, containerColor = Surface,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp).heightIn(max = 640.dp)) {
            Text("Photos and videos", style = MaterialTheme.typography.titleLarge, color = TextPrimary)
            Text("${selected.size} selected · ${(remaining - selected.size).coerceAtLeast(0)} remaining", color = TextMuted,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
            Text("${AttachmentPolicy.MAX_ATTACHMENTS - remaining.coerceIn(0, AttachmentPolicy.MAX_ATTACHMENTS)} already in this draft · ${AttachmentPolicy.MAX_ATTACHMENTS} attachments maximum", color = TextMuted)
            if (selected.size >= remaining) Text(
                if (remaining == 0) "Draft is full. Remove an attachment from your draft to add more."
                else "Attachment limit reached. Deselect an item to choose another.", color = TextMuted,
            )
            if (!allowed) Text("Allow photo and video access to browse here. If denied or restricted by device policy, review Settings or use Files without library access.", color = TextMuted)
            else if (!full) Text("Showing only media Android allows Caper to access. You can choose more below.", color = TextMuted)
            Row(Modifier.fillMaxWidth()) {
                if (!full) TextButton({ permission.launch(permissions()) }, Modifier.weight(1f), enabled = !submitting) { Text(if (allowed) "Manage access" else "Allow access") }
                TextButton({ runCatching { context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))) } }, Modifier.weight(1f), enabled = !submitting) { Text("Settings") }
                TextButton(files, Modifier.weight(1f), enabled = !submitting) { Text("Files") }
            }
            error?.let { Text(it, color = TextMuted) }
            if (loadFailed) TextButton({ revision++ }, enabled = !submitting) { Text("Try loading again") }
            if (allowed && !loading && items.isEmpty() && error == null) Text(if (full) "No photos or videos found." else "No accessible photos or videos. Choose more media or use Files.", color = TextMuted)
            LazyVerticalGrid(GridCells.Fixed(3), Modifier.weight(1f, fill = false).fillMaxWidth(), state = grid,
                horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                items(items, key = { it.uri.toString() }) { item ->
                    val chosen = item.uri in selected
                    val enabled = !submitting && (chosen || selected.size < remaining)
                    Box(Modifier.aspectRatio(1f).alpha(if (enabled) 1f else 0.35f)
                        .background(SurfaceRaised).border(if (chosen) 3.dp else 0.dp, if (chosen) Accent else Border)
                        .toggleable(chosen, enabled = enabled, role = Role.Checkbox) { selected = toggleGallerySelection(selected, item.uri, remaining) }
                        .semantics { contentDescription = if (item.video) "Video, ${item.duration / 1000} seconds" else "Photo"; stateDescription = if (chosen) "Selected ${selected.indexOf(item.uri) + 1}" else if (!enabled) "Attachment limit reached" else "Not selected" }) {
                        Text(if (item.video) "Video" else "Photo", Modifier.align(Alignment.Center), color = TextMuted)
                        GalleryThumbnail(item)
                        if (item.video) Text("${item.duration / 60000}:${(item.duration / 1000 % 60).toString().padStart(2, '0')}", Modifier.align(Alignment.BottomStart).background(Surface).padding(4.dp), color = TextPrimary)
                        Text(if (chosen) "${selected.indexOf(item.uri) + 1} ✓" else "○", Modifier.align(Alignment.TopEnd).background(if (chosen) Accent else Surface).padding(4.dp), color = TextPrimary)
                    }
                }
            }
            if (loading) LinearProgressIndicator(Modifier.fillMaxWidth())
            Button(onClick = {
                submitting = true
                scope.launch {
                    val valid = withContext(Dispatchers.IO) {
                        pruneGallerySelection(selected) { uri -> runCatching { context.contentResolver.openAssetFileDescriptor(uri, "r")?.use { true } == true }.getOrDefault(false) }
                    }
                    if (valid.size > currentRemaining) { error = "Your draft changed. Deselect items to fit the remaining slots."; submitting = false }
                    else if (valid == selected) { currentAdd(valid); dismiss() }
                    else { selected = valid; error = "Some selected media is no longer accessible. Review your selection."; submitting = false; revision++ }
                }
            }, enabled = selected.isNotEmpty() && selected.size <= remaining && !submitting && !loading,
                shape = MaterialTheme.shapes.small, modifier = Modifier.fillMaxWidth().padding(vertical = 12.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Action, contentColor = OnAction)) { Text("Add ${selected.size}") }
        }
    }
}

@Composable private fun GalleryThumbnail(item: GalleryItem) {
    val context = LocalContext.current
    val bitmap by produceState<Bitmap?>(null, item.uri) {
        value = try {
            providerWork(thumbnailDispatcher) { signal ->
                if (Build.VERSION.SDK_INT >= 29) context.contentResolver.loadThumbnail(item.uri, android.util.Size(256, 256), signal)
                else {
                    signal.setOnCancelListener {
                        if (item.video) MediaStore.Video.Thumbnails.cancelThumbnailRequest(context.contentResolver, item.id)
                        else MediaStore.Images.Thumbnails.cancelThumbnailRequest(context.contentResolver, item.id)
                    }
                    if (item.video) MediaStore.Video.Thumbnails.getThumbnail(context.contentResolver, item.id, MediaStore.Video.Thumbnails.MINI_KIND, null)
                    else MediaStore.Images.Thumbnails.getThumbnail(context.contentResolver, item.id, MediaStore.Images.Thumbnails.MINI_KIND, null)
                }
            }
        } catch (cancel: CancellationException) { throw cancel } catch (_: Exception) { null }
    }
    bitmap?.let { Image(it.asImageBitmap(), null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop) }
}
