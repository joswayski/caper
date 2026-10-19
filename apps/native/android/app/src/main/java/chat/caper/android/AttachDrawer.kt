package chat.caper.android

import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.provider.Settings
import android.util.LruCache
import android.util.Size
import android.view.HapticFeedbackConstants
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.keyframes
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.lifecycle.compose.LifecycleResumeEffect
import chat.caper.android.data.AttachCapacity
import chat.caper.android.data.AttachmentPolicy
import chat.caper.android.data.LibraryAccess
import chat.caper.android.data.LibraryPermissions
import chat.caper.android.model.DraftAttachmentUi
import chat.caper.android.ui.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** What the attach drawer reads from and sends: a channel/thread draft or a profile DM draft. */
internal class AttachDrawerHost(
    val files: List<DraftAttachmentUi>,
    val remove: (String) -> Unit,
    val canSend: Boolean,
    val send: () -> Unit,
)

internal data class LibraryItem(val id: Long, val uri: Uri, val video: Boolean, val durationMs: Long)

/** Every photo and video the app may read, newest first. Rows are small, so even large libraries list quickly. */
private fun queryLibrary(context: Context): List<LibraryItem> {
    val image = MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE
    val video = MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO
    val projection = arrayOf(MediaStore.Files.FileColumns._ID, MediaStore.Files.FileColumns.MEDIA_TYPE, "duration")
    val items = mutableListOf<LibraryItem>()
    context.contentResolver.query(
        MediaStore.Files.getContentUri("external"), projection,
        "${MediaStore.Files.FileColumns.MEDIA_TYPE} IN (?, ?)", arrayOf(image.toString(), video.toString()),
        // Ties (a burst, or several saved at once) fall back to the newest row, so reloads keep the order.
        "${MediaStore.Files.FileColumns.DATE_ADDED} DESC, ${MediaStore.Files.FileColumns._ID} DESC",
    )?.use { cursor ->
        val idColumn = cursor.getColumnIndexOrThrow(MediaStore.Files.FileColumns._ID)
        val typeColumn = cursor.getColumnIndexOrThrow(MediaStore.Files.FileColumns.MEDIA_TYPE)
        val durationColumn = cursor.getColumnIndex("duration")
        while (cursor.moveToNext()) {
            val id = cursor.getLong(idColumn)
            val isVideo = cursor.getInt(typeColumn) == video
            val base = if (isVideo) MediaStore.Video.Media.EXTERNAL_CONTENT_URI else MediaStore.Images.Media.EXTERNAL_CONTENT_URI
            val duration = if (isVideo && durationColumn >= 0) cursor.getLong(durationColumn) else 0L
            items += LibraryItem(id, ContentUris.withAppendedId(base, id), isVideo, duration)
        }
    }
    return items
}

private fun loadThumbnail(context: Context, item: LibraryItem, px: Int): Bitmap? = runCatching {
    val resolver = context.contentResolver
    when {
        Build.VERSION.SDK_INT >= 29 -> resolver.loadThumbnail(item.uri, Size(px, px), null)
        item.video -> @Suppress("DEPRECATION") MediaStore.Video.Thumbnails.getThumbnail(resolver, item.id, MediaStore.Video.Thumbnails.MINI_KIND, null)
        else -> @Suppress("DEPRECATION") MediaStore.Images.Thumbnails.getThumbnail(resolver, item.id, MediaStore.Images.Thumbnails.MINI_KIND, null)
    }
}.getOrNull()

/**
 * A private file the system camera writes a photo or video into, shared with it through the app's
 * FileProvider (`res/xml/notification_media_paths.xml`). Captures older than a day are cleared.
 */
internal fun cameraCaptureUri(context: Context, video: Boolean): Uri? = runCatching {
    val directory = File(context.cacheDir, "camera").apply { mkdirs() }
    directory.listFiles()?.filter { System.currentTimeMillis() - it.lastModified() > 24 * 60 * 60 * 1000L }?.forEach { it.delete() }
    val stamp = SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date())
    val file = File(directory, if (video) "Video-$stamp.mp4" else "Photo-$stamp.jpg")
    FileProvider.getUriForFile(context, "${context.packageName}.notification-media", file)
}.getOrNull()

private fun formatDuration(ms: Long): String {
    val seconds = ms / 1000
    return "%d:%02d".format(seconds / 60, seconds % 60)
}

/**
 * The composer's attach drawer: recent photos and videos (camera first) to tap into the draft,
 * Files, and Send. Tapping adds to the draft at once, so uploads start while the person keeps
 * choosing; tapping again removes it. At the limit, other tiles dim and a tap nudges the capacity
 * line instead. Library access is asked for the first time the drawer opens; without it, the
 * system photo picker still chooses items one at a time.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun AttachDrawer(
    host: AttachDrawerHost,
    add: (List<Uri>) -> Unit,
    gallery: () -> Unit,
    files: () -> Unit,
    camera: (video: Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val activity = LocalActivity.current
    val view = LocalView.current
    val still = reducedMotion()
    val scope = rememberCoroutineScope()
    val sdk = Build.VERSION.SDK_INT
    val requested = remember { LibraryPermissions.requested(sdk) }
    fun currentAccess() = LibraryPermissions.access(sdk) { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }
    var access by remember { mutableStateOf(currentAccess()) }
    var asked by rememberSaveable { mutableStateOf(false) }
    var blocked by remember { mutableStateOf(false) }
    var library by remember { mutableStateOf<List<LibraryItem>?>(null) }
    var revision by remember { mutableIntStateOf(0) }
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        asked = true
        access = currentAccess()
        // Denied with no rationale to show means Android won't ask again: Settings is the way back.
        blocked = access == LibraryAccess.NONE && activity != null &&
            requested.none { permission -> ActivityCompat.shouldShowRequestPermissionRationale(activity, permission) }
        revision++
    }
    LaunchedEffect(Unit) {
        if (access == LibraryAccess.NONE && !asked) runCatching { request.launch(requested.toTypedArray()) }.onFailure { asked = true }
    }
    // Back from Settings or the system's selection screen, read access and the library again.
    LifecycleResumeEffect(Unit) {
        access = currentAccess()
        revision++
        onPauseOrDispose {}
    }
    LaunchedEffect(access, revision) {
        library = if (access == LibraryAccess.NONE) emptyList()
        else withContext(Dispatchers.IO) { runCatching { queryLibrary(context) }.getOrDefault(emptyList()) }
    }
    DisposableEffect(access) {
        if (access == LibraryAccess.NONE) return@DisposableEffect onDispose {}
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) { revision++ }
        }
        runCatching { context.contentResolver.registerContentObserver(MediaStore.Files.getContentUri("external"), true, observer) }
        onDispose { context.contentResolver.unregisterContentObserver(observer) }
    }
    val thumbnails = remember {
        object : LruCache<Long, ImageBitmap>(24 * 1024 * 1024) {
            override fun sizeOf(key: Long, value: ImageBitmap) = value.width * value.height * 4
        }
    }

    val used = host.files.size
    val full = used >= AttachmentPolicy.MAX_ATTACHMENTS
    // Library URI -> (position in the draft, draft key).
    val selected = remember(host.files) {
        buildMap { host.files.forEachIndexed { index, file -> file.pickedUri?.let { if (it !in this) put(it, index + 1 to file.key) } } }
    }
    val nudge = remember { Animatable(0f) }
    fun refuse() {
        view.performHapticFeedback(if (Build.VERSION.SDK_INT >= 30) HapticFeedbackConstants.REJECT else HapticFeedbackConstants.LONG_PRESS)
        if (!still) scope.launch {
            nudge.snapTo(0f)
            nudge.animateTo(0f, keyframes {
                durationMillis = MOTION_SLOW
                -4f at 52; 4f at 104; -4f at 156; 4f at 208
            })
        }
    }
    val hasCamera = remember { context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY) }
    val height = with(LocalDensity.current) { LocalWindowInfo.current.containerSize.height.toDp() } * 0.7f

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = Surface, contentColor = Text,
    ) {
        Box(Modifier.fillMaxWidth().height(height)) {
            Column(Modifier.fillMaxSize()) {
                // Header: title and the capacity line, which shakes when a full drawer refuses a file.
                Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("Attach", fontSize = 15.sp, fontWeight = FontWeight.Bold)
                    Spacer(Modifier.weight(1f))
                    Row(
                        Modifier.graphicsLayer { translationX = nudge.value.dp.toPx() }
                            .semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
                        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Row(horizontalArrangement = Arrangement.spacedBy(3.dp), modifier = Modifier.clearAndSetSemantics {}) {
                            repeat(AttachmentPolicy.MAX_ATTACHMENTS) { index ->
                                Box(Modifier.size(6.dp).background(if (full) Terracotta else if (index < used) Text else Border, CircleShape))
                            }
                        }
                        Text(AttachCapacity.label(used), fontSize = 12.sp, color = if (full) TerracottaBright else TextMuted)
                    }
                }
                AttachCapacity.hint(used)?.let {
                    Text(it, Modifier.padding(horizontal = 16.dp), fontSize = 12.sp, color = TextMuted)
                }
                Spacer(Modifier.height(10.dp))
                Box(Modifier.weight(1f).fillMaxWidth()) {
                    when {
                        access == LibraryAccess.NONE && !asked -> CircularProgressIndicator(Modifier.align(Alignment.Center).size(24.dp), color = TextMuted, strokeWidth = 2.dp)
                        access == LibraryAccess.NONE -> Column(
                            Modifier.align(Alignment.Center).padding(24.dp),
                            horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Text(
                                if (blocked) "Caper can’t see your photos. Allow access in Settings, or choose items one at a time."
                                else "Allow photo access to pick recent photos and videos here, or choose items one at a time.",
                                color = TextMuted, fontSize = 14.sp, textAlign = TextAlign.Center,
                            )
                            Button(
                                {
                                    if (blocked) runCatching {
                                        context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)))
                                    } else runCatching { request.launch(requested.toTypedArray()) }
                                },
                                shape = MaterialTheme.shapes.small,
                                colors = ButtonDefaults.buttonColors(containerColor = Action, contentColor = OnAction),
                            ) { Text(if (blocked) "Open Settings" else "Allow access", fontWeight = FontWeight.Bold) }
                            TextButton({ if (full) refuse() else gallery() }) { Text("Choose photos and videos", color = Text) }
                            if (hasCamera) TextButton({ if (full) refuse() else camera(false) }) { Text("Take a photo", color = Text) }
                        }
                        else -> Column(Modifier.fillMaxSize()) {
                            if (access == LibraryAccess.PARTIAL) Row(
                                Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Text("Caper can see only the photos you allowed.", Modifier.weight(1f), fontSize = 12.sp, color = TextMuted)
                                TextButton({ runCatching { request.launch(requested.toTypedArray()) } }) {
                                    Text("Manage", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = Text)
                                }
                            }
                            LazyVerticalGrid(
                                GridCells.Adaptive(88.dp), Modifier.fillMaxSize().semantics { testTag = "attach-drawer-grid" },
                                // Keep the last row clear of the floating Send button.
                                contentPadding = PaddingValues(start = 2.dp, end = 2.dp, bottom = 76.dp),
                                horizontalArrangement = Arrangement.spacedBy(2.dp), verticalArrangement = Arrangement.spacedBy(2.dp),
                            ) {
                                if (hasCamera) item(key = "camera") { CameraTile(full, ::refuse, camera) }
                                items(library.orEmpty(), key = { it.id }) { item ->
                                    val selection = selected[item.uri.toString()]
                                    LibraryTile(item, selection?.first, dimmed = full && selection == null, thumbnails) {
                                        when {
                                            selection != null -> host.remove(selection.second)
                                            full -> refuse()
                                            else -> {
                                                view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
                                                add(listOf(item.uri))
                                            }
                                        }
                                    }
                                }
                            }
                            if (library?.isEmpty() == true) Text("No photos or videos yet.", Modifier.padding(16.dp), color = TextMuted, fontSize = 13.sp)
                        }
                    }
                }
                HorizontalDivider(color = Border)
                Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp).navigationBarsPadding()) {
                    DrawerTab("Photos", R.drawable.lucide_images, selected = true, Modifier.weight(1f)) {}
                    DrawerTab("Files", R.drawable.lucide_paperclip, selected = false, Modifier.weight(1f)) { if (full) refuse() else files() }
                }
            }
            if (host.files.isNotEmpty()) {
                FilledIconButton(
                    { host.send(); onDismiss() },
                    Modifier.align(Alignment.BottomEnd).padding(end = 16.dp, bottom = 76.dp).navigationBarsPadding().size(52.dp)
                        .entrance(MOTION_BASE, scale = 0.94f)
                        .semantics { contentDescription = if (host.canSend) "Send message" else "Uploading attachments" },
                    enabled = host.canSend, shape = CircleShape,
                    colors = IconButtonDefaults.filledIconButtonColors(
                        containerColor = Action, contentColor = OnAction,
                        disabledContainerColor = SurfaceRaised, disabledContentColor = TextMuted,
                    ),
                ) { Icon(painterResource(R.drawable.lucide_send), null, Modifier.size(20.dp)) }
            }
        }
    }
}

@Composable private fun DrawerTab(title: String, icon: Int, selected: Boolean, modifier: Modifier, onClick: () -> Unit) {
    Column(
        modifier.heightIn(min = 52.dp).clip(MaterialTheme.shapes.small)
            .clickable(onClickLabel = title, onClick = onClick)
            .semantics(mergeDescendants = true) { this.selected = selected; role = Role.Tab },
        horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center,
    ) {
        Icon(painterResource(icon), null, Modifier.size(20.dp), tint = if (selected) Text else TextMuted)
        Text(title, fontSize = 11.sp, color = if (selected) Text else TextMuted, fontWeight = if (selected) FontWeight.Bold else FontWeight.Normal)
    }
}

@Composable private fun CameraTile(full: Boolean, refuse: () -> Unit, camera: (Boolean) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Box(
        Modifier.aspectRatio(1f).background(SurfaceRaised).alpha(if (full) 0.4f else 1f)
            .clickable(onClickLabel = "Camera") { if (full) refuse() else menu = true }
            .semantics { contentDescription = "Camera"; role = Role.Button },
        contentAlignment = Alignment.Center,
    ) {
        Icon(painterResource(R.drawable.lucide_camera), null, Modifier.size(26.dp), tint = TextMuted)
        DropdownMenu(menu, { menu = false }, containerColor = SurfaceRaised, border = BorderStroke(1.dp, Border)) {
            DropdownMenuItem(text = { Text("Take photo", fontSize = 13.sp) }, onClick = { menu = false; camera(false) })
            DropdownMenuItem(text = { Text("Record video", fontSize = 13.sp) }, onClick = { menu = false; camera(true) })
        }
    }
}

@Composable private fun LibraryTile(item: LibraryItem, order: Int?, dimmed: Boolean, cache: LruCache<Long, ImageBitmap>, onClick: () -> Unit) {
    val context = LocalContext.current
    val px = with(LocalDensity.current) { 120.dp.roundToPx() }
    val bitmap by produceState(cache.get(item.id), item.id) {
        if (value == null) value = withContext(Dispatchers.IO) { loadThumbnail(context, item, px)?.asImageBitmap() }?.also { cache.put(item.id, it) }
    }
    Box(
        Modifier.aspectRatio(1f).background(SurfaceRaised).alpha(if (dimmed) 0.4f else 1f)
            .clickable(onClick = onClick)
            .semantics(mergeDescendants = true) {
                contentDescription = if (item.video) "Video, ${item.durationMs / 1000} seconds" else "Photo"
                stateDescription = order?.let { "Attached $it" } ?: if (dimmed) "Attachment limit reached" else ""
                selected = order != null
                role = Role.Checkbox
                testTag = "attach-drawer-tile"
            },
    ) {
        bitmap?.let { Image(it, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop) }
        if (order != null) Box(Modifier.fillMaxSize().background(Blackout.copy(alpha = 0.25f)).border(3.dp, Terracotta))
        Box(Modifier.align(Alignment.TopEnd).padding(6.dp).size(24.dp)) {
            if (order != null) Box(
                Modifier.fillMaxSize().entrance(MOTION_BASE, scale = 0.94f).background(Terracotta, CircleShape),
                contentAlignment = Alignment.Center,
            ) { Text("$order", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = Text) }
            else Box(Modifier.fillMaxSize().background(Blackout.copy(alpha = 0.35f), CircleShape).border(1.5.dp, Text.copy(alpha = 0.9f), CircleShape))
        }
        if (item.video) Text(
            formatDuration(item.durationMs),
            Modifier.align(Alignment.BottomStart).padding(5.dp).background(Blackout.copy(alpha = 0.7f), RoundedCornerShape(4.dp)).padding(horizontal = 5.dp, vertical = 2.dp),
            fontSize = 11.sp, fontWeight = FontWeight.Bold, color = Text,
        )
    }
}
