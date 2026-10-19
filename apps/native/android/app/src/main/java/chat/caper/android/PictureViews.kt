package chat.caper.android

import android.content.Context
import android.net.Uri
import android.os.Build
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.mimeTypes
import androidx.compose.ui.draganddrop.toAndroidDragEvent
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.FileDrop
import chat.caper.android.model.AppUiState
import chat.caper.android.model.PROFILE_PICTURE_KEY
import chat.caper.android.model.Picture
import chat.caper.android.model.PictureSaveUi
import chat.caper.android.model.SpaceDetail
import chat.caper.android.model.caperAvatarIndex
import chat.caper.android.model.pictureOf
import chat.caper.android.ui.*
import coil3.compose.AsyncImage
import coil3.gif.AnimatedImageDecoder
import coil3.gif.GifDecoder
import coil3.request.ImageRequest
import coil3.size.Scale
import chat.caper.android.data.AttachmentEdit
import chat.caper.android.data.isAnimatedImage
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.shape.CircleShape
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Custom profile pictures are rounded squares: corner radius 30% of the side (`--radius-avatar`). */
internal val PictureShape = RoundedCornerShape(percent = 30)

/**
 * A custom profile picture or space icon request. Both caches are keyed by the picture's asset
 * ID, never its URL, which re-signs daily. The memory entry is per display size (decoded near
 * [px], so long member lists stay small) and per animated or still; the disk entry holds the
 * downloaded original once. GIFs and animated WebP animate unless [animate] is false.
 */
internal fun pictureRequest(context: Context, picture: Picture, px: Int, animate: Boolean): ImageRequest =
    ImageRequest.Builder(context).data(picture.url)
        .memoryCacheKey("picture:${picture.asset}:$px" + if (animate) ":animated" else "")
        .diskCacheKey("picture:${picture.asset}")
        // Fill the square, so a non-square GIF still covers the circle once cropped.
        .size(px).scale(Scale.FILL)
        .apply { if (animate) decoderFactory(if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) AnimatedImageDecoder.Factory() else GifDecoder.Factory()) }
        .build()

/**
 * [picture] filling its (already clipped) box, aspect-filled and centered. It animates unless
 * the system's Remove animations is on, which shows a still first frame. Nothing draws while it
 * loads or after a failure, so the mascot or initial beneath shows through; [loaded] reports
 * whether the picture is on screen. A new URL for the same asset reuses the cache, and retries
 * one that failed.
 */
@Composable internal fun PictureImage(
    picture: Picture, size: Dp, modifier: Modifier = Modifier, contentDescription: String? = null, loaded: (Boolean) -> Unit = {},
) {
    val context = LocalContext.current
    val px = with(LocalDensity.current) { size.roundToPx() }.coerceIn(1, 1024)
    val animate = !reducedMotion()
    val request = remember(picture, px, animate) { pictureRequest(context, picture, px, animate) }
    AsyncImage(
        model = request, contentDescription = contentDescription, imageLoader = AttachmentImages.loader(context),
        modifier = modifier, contentScale = ContentScale.Crop,
        onSuccess = { loaded(true) }, onError = { loaded(false) },
    )
}

/**
 * A space tile's face: the owner's icon filling the tile (its shape clips it), over the initial,
 * which shows while the icon loads, when it fails, and when there is none.
 */
@Composable internal fun SpaceTileFace(
    name: String, iconAsset: String?, iconUrl: String?, size: Dp, color: Color, fontSize: TextUnit = TextUnit.Unspecified,
) {
    val picture = pictureOf(iconAsset, iconUrl)
    var shown by remember(picture) { mutableStateOf(false) }
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        if (!shown) Text(initialOf(name), fontWeight = FontWeight.Black, color = color, fontSize = fontSize)
        if (picture != null) PictureImage(picture, size, Modifier.fillMaxSize()) { shown = it }
    }
}

/** The picture frame's side. */
private val FrameSide = 112.dp

/**
 * Edit profile's picture in its frame, with an edit badge on the corner. Without a custom
 * picture the saved mascot holds up "change me!" and the frame opens the system photo picker
 * (images only; GIFs stay animated); with one, it offers a new picture or removal. The change
 * runs in the view model, so it finishes (or fails here) even if the dialog closes.
 */
@Composable internal fun ProfilePictureSection(state: AppUiState, viewModel: CaperViewModel) {
    val account = state.account ?: return
    val save = state.pictureSaves[PROFILE_PICTURE_KEY]
    val epoch = viewModel.accountEpoch
    val editPicture = pictureEditor(viewModel, epoch, null)
    var pickerEpoch by remember { mutableStateOf<Long?>(null) }
    // An error from an earlier visit doesn't greet the next one; a change in flight keeps showing.
    LaunchedEffect(Unit) { viewModel.dismissPictureError(PROFILE_PICTURE_KEY) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        if (uri != null && pickerEpoch == viewModel.accountEpoch) editPicture(uri)
    }
    val name = account.displayName ?: account.username ?: "You"
    // Set even when this server sends no URL to show it with, so it can still be removed.
    val hasPicture = !account.avatarAsset.isNullOrEmpty()
    var gallery by remember(epoch) { mutableStateOf(false) }
    if (gallery) AvatarGallery(account.avatarAsset, epoch, viewModel) { gallery = false }
    PictureFrame(
        save = save,
        unavailable = state.uploadsChecked && !state.uploadsEnabled,
        hasPicture = hasPicture,
        label = "Change profile picture", noun = "picture",
        drop = editPicture,
        edit = account.avatarUrl?.let { url -> { editPicture(Uri.parse(url)) } },
        change = { pickerEpoch = epoch; runCatching { picker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) } },
        remove = { viewModel.removePicture() },
        history = { gallery = true },
    ) {
        if (hasPicture) {
            Avatar(name, FrameSide, avatarId = account.avatarId, avatarAsset = account.avatarAsset, avatarUrl = account.avatarUrl)
        } else {
            ChangeMeMascot(name, account.avatarId, FrameSide)
        }
    }
}

/** Edit profile's frame without a custom picture: the saved mascot holding up a "change me!" sign. */
@Composable private fun ChangeMeMascot(name: String, avatarId: Int?, size: Dp) {
    Box(Modifier.size(size)) {
        Avatar(name, size, avatarId = avatarId)
        if (caperAvatarIndex(avatarId) != null) Image(painterResource(R.drawable.caper_change_me), null, Modifier.matchParentSize())
    }
}

/** Manage space's icon (owner only): the icon or initial tile in its frame. */
@Composable internal fun SpaceIconSection(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel) {
    val space = detail.space
    if (space.demo || state.account?.id != space.ownerId) return
    val save = state.pictureSaves[space.id]
    val epoch = viewModel.accountEpoch
    val editPicture = pictureEditor(viewModel, epoch, space.id)
    var pickerTarget by remember { mutableStateOf<Pair<Long, String>?>(null) }
    LaunchedEffect(space.id) { viewModel.dismissPictureError(space.id) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        if (uri != null && pickerTarget == (viewModel.accountEpoch to space.id)) editPicture(uri)
    }
    Text("Icon", fontSize = 13.sp, fontWeight = FontWeight.Bold)
    PictureFrame(
        save = save,
        unavailable = state.uploadsChecked && !state.uploadsEnabled,
        hasPicture = !space.iconAsset.isNullOrEmpty(),
        label = "Change space icon", noun = "icon",
        drop = editPicture,
        edit = space.iconUrl?.let { url -> { editPicture(Uri.parse(url)) } },
        change = { pickerTarget = epoch to space.id; runCatching { picker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) } },
        remove = { viewModel.removePicture(space.id) },
    ) {
        SpaceTileFace(space.name, space.iconAsset, space.iconUrl, FrameSide, TextMuted, fontSize = 34.sp)
    }
}

/**
 * The picture framed in the rounded square it has everywhere, with the eggshell edit badge on
 * its corner. The frame picks a picture directly, or (with a custom one set) opens a menu to
 * choose another or remove it. Progress and the inline error show beneath.
 */
@Composable private fun PictureFrame(
    save: PictureSaveUi?, unavailable: Boolean, hasPicture: Boolean, label: String, noun: String,
    change: () -> Unit, remove: () -> Unit, drop: (Uri) -> Unit,
    edit: (() -> Unit)? = null, history: (() -> Unit)? = null,
    picture: @Composable () -> Unit,
) {
    val busy = save?.busy == true
    // With uploads unavailable, a custom picture can still be removed.
    val enabled = !busy && (hasPicture || history != null || !unavailable)
    var menu by remember { mutableStateOf(false) }
    var dropping by remember { mutableStateOf(false) }
    val canDrop by rememberUpdatedState(!busy && !unavailable)
    val onDrop by rememberUpdatedState(drop)
    val activity = LocalActivity.current
    val dropTarget = remember(activity) {
        object : DragAndDropTarget {
            override fun onEntered(event: DragAndDropEvent) { dropping = canDrop }
            override fun onExited(event: DragAndDropEvent) { dropping = false }
            override fun onEnded(event: DragAndDropEvent) { dropping = false }
            override fun onDrop(event: DragAndDropEvent): Boolean {
                dropping = false
                if (!canDrop) return false
                val drag = event.toAndroidDragEvent()
                val clip = drag.clipData ?: return false
                val uri = (0 until clip.itemCount).firstNotNullOfOrNull { clip.getItemAt(it).uri } ?: return false
                activity?.requestDragAndDropPermissions(drag)
                menu = false
                onDrop(uri)
                return true
            }
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box {
            Box(
                Modifier
                    .dragAndDropTarget({ canDrop && FileDrop.carriesFiles(it.mimeTypes()) }, dropTarget)
                    .semantics { contentDescription = label }
                    .clickable(enabled = enabled, role = Role.Button) { if (hasPicture || history != null) menu = true else change() }
                    // Room for the badge, which sits over the frame's corner.
                    .padding(end = 6.dp, bottom = 6.dp),
            ) {
                Box(
                    Modifier.size(FrameSide).alpha(if (busy) 0.55f else 1f).clip(PictureShape)
                        .background(SurfaceRaised).border(if (dropping) 2.dp else 1.dp, if (dropping) Terracotta else Border, PictureShape),
                    contentAlignment = Alignment.Center,
                ) { picture() }
                Box(
                    Modifier.align(Alignment.BottomEnd).offset(6.dp, 6.dp)
                        .background(Surface, RoundedCornerShape(10.dp)).padding(3.dp)
                        .background(if (enabled) Action else Border, RoundedCornerShape(8.dp)).size(28.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(painterResource(R.drawable.lucide_pencil), null, Modifier.size(15.dp), tint = if (enabled) OnAction else TextMuted)
                }
            }
            DropdownMenu(menu, { menu = false }, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                if (hasPicture) DropdownMenuItem({ Text("Edit $noun") }, { menu = false; edit?.invoke() }, enabled = !busy && !unavailable && edit != null)
                DropdownMenuItem(
                    { Text("Choose a new $noun") }, { menu = false; change() }, enabled = !busy && !unavailable,
                    leadingIcon = { Icon(painterResource(R.drawable.lucide_image), null, Modifier.size(16.dp)) },
                )
                if (history != null) DropdownMenuItem({ Text("Previous pictures") }, { menu = false; history() }, enabled = !busy)
                if (hasPicture) DropdownMenuItem(
                    { Text("Remove $noun") }, { menu = false; remove() },
                    enabled = !busy,
                    leadingIcon = { Icon(painterResource(R.drawable.lucide_trash_2), null, Modifier.size(16.dp)) },
                )
            }
        }
        val status = when {
            save?.uploading == true -> "Uploading…"
            save?.removing == true -> "Removing…"
            unavailable -> "Picture uploads aren’t available right now."
            else -> null
        }
        save?.error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
            ?: status?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 12.sp) }
    }
}

@Composable private fun AvatarGallery(current: String?, epoch: Long, model: CaperViewModel, close: () -> Unit) {
    val expected = remember { current }
    var pictures by remember { mutableStateOf(emptyList<chat.caper.android.model.AvatarHistoryPicture>()) }
    var selected by remember { mutableStateOf<chat.caper.android.model.AvatarHistoryPicture?>(null) }
    var cursor by remember { mutableStateOf<String?>(null) }
    var loaded by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var stale by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    suspend fun load() {
        busy = true; error = null
        try {
            val page = model.avatarHistory(epoch, cursor)
            pictures = (pictures + page.pictures).distinctBy { it.avatarAsset }
            cursor = page.nextCursor; loaded = true
        } catch (e: kotlinx.coroutines.CancellationException) { throw e }
        catch (_: Exception) { error = "Couldn’t load pictures. Please retry." }
        finally { busy = false }
    }
    LaunchedEffect(Unit) { load() }
    AlertDialog(onDismissRequest = { if (!saving) close() }, title = { Text("Previous pictures") }, text = {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.size(144.dp).clip(CircleShape).background(SurfaceRaised), contentAlignment = Alignment.Center) {
                selected?.let { PictureImage(Picture(it.avatarAsset, it.avatarUrl), 144.dp, Modifier.fillMaxSize()) }
                    ?: Text("Select a picture", color = TextMuted)
            }
            LazyVerticalGrid(GridCells.Fixed(3), Modifier.fillMaxWidth().heightIn(max = 240.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items(pictures, key = { it.avatarAsset }) { item ->
                    Column(Modifier.clickable(enabled = !saving) { selected = item }, horizontalAlignment = Alignment.CenterHorizontally) {
                        Box(Modifier.aspectRatio(1f).clip(PictureShape).border(2.dp, if (selected == item) Terracotta else Border, PictureShape)) {
                            PictureImage(Picture(item.avatarAsset, item.avatarUrl), 80.dp, Modifier.fillMaxSize(), "Previous picture")
                            if (selected == item) Text("✓", Modifier.align(Alignment.TopEnd).background(Terracotta), color = chat.caper.android.ui.Text)
                        }
                        if (item.avatarAsset == expected) Text("Current", fontSize = 11.sp)
                    }
                }
            }
            if (loaded && pictures.isEmpty()) Text("No previous pictures yet.")
            if (busy) CircularProgressIndicator(Modifier.size(24.dp))
            error?.let { Text(it, color = ErrorText) }
            if (!busy && !saving && !stale && (error != null || cursor != null)) TextButton({ scope.launch { load() } }) { Text(if (error != null) "Retry" else "Load more") }
            if (stale) TextButton({ close() }) { Text("Close and reopen to refresh") }
        }
    }, confirmButton = {
        Button(onClick = { scope.launch {
            saving = true; error = null
            try { model.selectAvatar(epoch, selected!!.avatarAsset, expected); close() }
            catch (e: kotlinx.coroutines.CancellationException) { throw e }
            catch (e: Exception) {
                stale = e is chat.caper.android.data.ApiException && e.status == 409
                error = if (stale) "Your picture changed elsewhere. Reopen to refresh." else "Couldn’t save picture. Please retry."
            } finally { saving = false }
        } }, enabled = !saving && !busy && !stale && selected != null && selected?.avatarAsset != expected,
            colors = ButtonDefaults.buttonColors(containerColor = Action, contentColor = OnAction)) { Text(if (saving) "Saving…" else "Save picture") }
    }, dismissButton = { TextButton(close, enabled = !saving) { Text("Cancel") } })
}

/** A private, cancellable local draft. Ownership transfers to the upload only on Done. */
@Composable private fun pictureEditor(viewModel: CaperViewModel, epoch: Long, spaceId: String?): (Uri) -> Unit {
    val context = LocalContext.current
    var source by remember(epoch, spaceId) { mutableStateOf<Uri?>(null) }
    val uri = source
    if (uri != null) key(epoch, spaceId, uri) {
        val directory = remember { File(context.cacheDir, "picture-editor-${UUID.randomUUID()}") }
        var file by remember { mutableStateOf<File?>(null) }
        var error by remember { mutableStateOf<String?>(null) }
        var animated by remember { mutableStateOf(false) }
        var gif by remember { mutableStateOf(false) }
        var consent by remember { mutableStateOf(false) }
        var handedOff by remember { mutableStateOf(false) }
        LaunchedEffect(uri) {
            try {
                val copied = viewModel.copyPictureForEditor(uri, directory)
                animated = withContext(Dispatchers.IO) {
                    val header = copied.inputStream().use { stream ->
                        val buffer = ByteArray(64 * 1024)
                        var count = 0
                        while (count < buffer.size) {
                            val n = stream.read(buffer, count, buffer.size - count)
                            if (n < 0) break
                            count += n
                        }
                        buffer.copyOf(count)
                    }
                    gif = String(header.take(6).toByteArray(), Charsets.US_ASCII).startsWith("GIF8")
                    gif ||
                        isAnimatedImage("image/webp", header) || isAnimatedImage("image/png", header)
                }
                file = copied
            } catch (failure: kotlinx.coroutines.CancellationException) { throw failure }
            catch (_: Exception) { error = "Couldn’t open this picture. Please choose it again." }
            finally {
                if (file == null) withContext(NonCancellable + Dispatchers.IO) { directory.deleteRecursively() }
            }
        }
        DisposableEffect(directory) { onDispose { if (!handedOff) directory.deleteRecursively() } }
        fun upload(edit: AttachmentEdit?) {
            val local = file ?: return
            handedOff = true
            viewModel.changePicture(Uri.fromFile(local), spaceId, epoch, edit, directory)
            source = null
        }
        when {
            error != null -> AlertDialog(onDismissRequest = { source = null }, title = { Text("Picture unavailable") },
                text = { Text(error!!) }, confirmButton = { TextButton({ source = null }) { Text("Close") } })
            file == null -> AlertDialog(onDismissRequest = { source = null }, title = { Text("Opening picture…") },
                confirmButton = { TextButton({ source = null }) { Text("Cancel") } })
            animated && !consent -> AlertDialog(onDismissRequest = { source = null }, title = { Text("Animated picture") },
                text = { Text(if (gif) "Cropping or adjusting saves a still frame and removes animation. Keep the original GIF to preserve its animation, or edit a still frame."
                    else "Editing saves a still frame and removes animation. Animated pictures must be GIFs to keep their animation; cancel and choose a GIF to preserve it.") },
                confirmButton = { if (gif) TextButton({ upload(null) }) { Text("Keep original animation") } },
                dismissButton = { Row {
                    TextButton({ source = null }) { Text("Cancel") }
                    TextButton({ consent = true }) { Text("Edit still frame") }
                } })
            else -> AttachmentEditor(Uri.fromFile(file).toString(), "image", "Picture", null,
                cancel = { source = null }, save = { edit, preview ->
                    preview?.recycle()
                    upload(edit ?: AttachmentEdit.NONE)
                }, initialTool = EditTool.CROP, pictureMode = true)
        }
    }
    return { if (viewModel.accountEpoch == epoch) source = it }
}
