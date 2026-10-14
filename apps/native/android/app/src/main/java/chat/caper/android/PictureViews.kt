package chat.caper.android

import android.content.Context
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.model.AppUiState
import chat.caper.android.model.PROFILE_PICTURE_KEY
import chat.caper.android.model.Picture
import chat.caper.android.model.PictureSaveUi
import chat.caper.android.model.SpaceDetail
import chat.caper.android.model.pictureOf
import chat.caper.android.ui.*
import coil3.compose.AsyncImage
import coil3.gif.AnimatedImageDecoder
import coil3.gif.GifDecoder
import coil3.request.ImageRequest
import coil3.size.Scale

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

/**
 * Edit profile's picture: the current one large (or the saved mascot), Change picture (the
 * system photo picker, images only; GIFs stay animated) and, while one is set, Remove picture.
 * The change runs in the view model, so it finishes (or fails here) even if the dialog closes.
 */
@Composable internal fun ProfilePictureSection(state: AppUiState, viewModel: CaperViewModel) {
    val account = state.account ?: return
    val save = state.pictureSaves[PROFILE_PICTURE_KEY]
    val epoch = viewModel.accountEpoch
    // An error from an earlier visit doesn't greet the next one; a change in flight keeps showing.
    LaunchedEffect(Unit) { viewModel.dismissPictureError(PROFILE_PICTURE_KEY) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        if (uri != null) viewModel.changePicture(uri, epoch = epoch)
    }
    PictureControls(
        save = save,
        unavailable = state.uploadsChecked && !state.uploadsEnabled,
        // Set even when this server sends no URL to show it with, so it can still be removed.
        hasPicture = !account.avatarAsset.isNullOrEmpty(),
        changeLabel = "Change picture", removeLabel = "Remove picture",
        hint = "PNG, JPEG, WebP or GIF. GIFs up to 5 MB keep their animation.",
        change = { runCatching { picker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) } },
        remove = { viewModel.removePicture() },
    ) {
        Avatar(account.displayName ?: account.username ?: "You", 72.dp, avatarId = account.avatarId,
            avatarAsset = account.avatarAsset, avatarUrl = account.avatarUrl)
    }
}

/** Manage space's icon (owner only): the icon or initial tile, Change icon and, while one is set, Remove icon. */
@Composable internal fun SpaceIconSection(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel) {
    val space = detail.space
    if (space.demo || state.account?.id != space.ownerId) return
    val save = state.pictureSaves[space.id]
    val epoch = viewModel.accountEpoch
    LaunchedEffect(space.id) { viewModel.dismissPictureError(space.id) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        if (uri != null) viewModel.changePicture(uri, space.id, epoch)
    }
    Text("Icon", fontSize = 13.sp, fontWeight = FontWeight.Bold)
    PictureControls(
        save = save,
        unavailable = state.uploadsChecked && !state.uploadsEnabled,
        hasPicture = !space.iconAsset.isNullOrEmpty(),
        changeLabel = "Change icon", removeLabel = "Remove icon",
        hint = "Shown in the space list for everyone in ${space.name}.",
        change = { runCatching { picker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) } },
        remove = { viewModel.removePicture(space.id) },
    ) {
        Surface(Modifier.size(64.dp), color = SurfaceRaised, shape = MaterialTheme.shapes.medium, border = BorderStroke(1.dp, Border)) {
            SpaceTileFace(space.name, space.iconAsset, space.iconUrl, 64.dp, TextMuted, fontSize = 24.sp)
        }
    }
}

/** The picture beside its quiet actions (wrapping on narrow phones), with progress and the inline error under them. */
@OptIn(ExperimentalLayoutApi::class)
@Composable private fun PictureControls(
    save: PictureSaveUi?, unavailable: Boolean, hasPicture: Boolean,
    changeLabel: String, removeLabel: String, hint: String, change: () -> Unit, remove: () -> Unit,
    picture: @Composable () -> Unit,
) {
    val busy = save?.busy == true
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        picture()
        Spacer(Modifier.width(16.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                OutlinedButton(change, enabled = !busy && !unavailable, shape = MaterialTheme.shapes.small,
                    border = BorderStroke(1.dp, Border), colors = neutralOutlinedButtonColors()) {
                    PendingLabel(changeLabel, "Uploading…", save?.uploading == true)
                }
                if (hasPicture) TextButton(remove, enabled = !busy, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors(TextMuted)) {
                    PendingLabel(removeLabel, "Removing…", save?.removing == true)
                }
            }
            Text(if (unavailable) "Picture uploads aren’t available right now." else hint, color = TextMuted, fontSize = 11.sp, lineHeight = 15.sp)
        }
    }
    save?.error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
}
