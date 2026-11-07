package chat.caper.android

import androidx.annotation.DrawableRes
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.data.FileDrop
import chat.caper.android.ui.Blackout
import chat.caper.android.ui.Border
import chat.caper.android.ui.Surface
import chat.caper.android.ui.SurfaceRaised
import chat.caper.android.ui.Terracotta
import chat.caper.android.ui.TextMuted
import chat.caper.android.ui.Text as TextColor
import kotlin.random.Random

private val cardShape = RoundedCornerShape(8.dp)

/**
 * Covers the conversation while files are dragged over it: a random Caper character hops
 * in front of three file cards that drift apart behind it. Each open picks a new character
 * and a new seed for the motion. It never takes input.
 */
@Composable internal fun FileDropOverlay(title: String, detail: String, modifier: Modifier = Modifier) {
    // System "Remove animations" shows the resting pose, without the hops or drift.
    val still = reducedMotion()
    val character = remember { caperBrandingResources.random() }
    val plan = remember { FileDrop.plan(Random.nextInt()) }
    var elapsed by remember { mutableFloatStateOf(0f) }
    LaunchedEffect(still) {
        if (still) return@LaunchedEffect
        val start = withFrameNanos { it }
        while (true) withFrameNanos { elapsed = (it - start) / 1e9f }
    }
    // Read only in graphics layers and draw blocks, so frames redraw without recomposing.
    val time = { if (still) 1f else elapsed }
    val settle = { progress: Float -> if (still) 1f else progress }
    Box(
        modifier.fillMaxSize()
            .drawBehind { drawRect(Blackout.copy(alpha = 0.82f * settle(FileDrop.progress(time(), 0f, 0.14f)))) }
            .padding(16.dp)
            .semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
        contentAlignment = Alignment.Center,
    ) {
        Column(
            Modifier
                .graphicsLayer {
                    alpha = settle(FileDrop.progress(time(), 0f, 0.12f))
                    translationY = 10.dp.toPx() * (1f - FileDrop.easeOut(settle(FileDrop.progress(time(), 0f, 0.26f))))
                }
                .widthIn(max = 360.dp)
                .shadow(24.dp, cardShape)
                .background(Surface, cardShape)
                .border(1.dp, Border, cardShape)
                .drawBehind {
                    val inset = 9.75.dp.toPx()
                    drawRect(
                        Terracotta, topLeft = Offset(inset, inset), size = Size(size.width - 2 * inset, size.height - 2 * inset),
                        style = Stroke(1.5.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(5.dp.toPx(), 4.dp.toPx()))),
                    )
                }
                .padding(start = 24.dp, end = 24.dp, top = 20.dp, bottom = 26.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            DropArt({ FileDrop.pose(plan, elapsed.toDouble(), still) }, character)
            Spacer(Modifier.height(14.dp))
            Text(title, color = TextColor, fontSize = 18.sp, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center)
            Spacer(Modifier.height(6.dp))
            Text(detail, color = TextMuted, fontSize = 13.sp, lineHeight = 18.sp, textAlign = TextAlign.Center)
        }
    }
}

/** A 176×128dp stage: the middle card first, behind the other two, then the character in front. */
@Composable private fun DropArt(pose: () -> FileDrop.Pose, @DrawableRes character: Int) {
    Box(Modifier.size(176.dp, 128.dp).clearAndSetSemantics {}) {
        listOf(1 to R.drawable.lucide_file_text, 0 to R.drawable.lucide_image, 2 to R.drawable.lucide_film).forEach { (index, icon) ->
            Box(
                Modifier
                    .graphicsLayer {
                        val card = pose().cards[index]
                        transformOrigin = TransformOrigin(0.5f, 1f)
                        rotationZ = card.turn.toFloat()
                        alpha = card.opacity.toFloat()
                        // The card's bottom edge sits 98dp down the stage.
                        translationX = (88f + card.x.toFloat() - 27f).dp.toPx()
                        translationY = (98f + card.y.toFloat() - 66f).dp.toPx()
                    }
                    .size(54.dp, 66.dp)
                    .background(SurfaceRaised, cardShape)
                    .border(1.dp, Border, cardShape),
                contentAlignment = Alignment.Center,
            ) { Icon(painterResource(icon), null, Modifier.size(20.dp), tint = TextMuted) }
        }
        Image(
            painterResource(character), null,
            Modifier
                .graphicsLayer {
                    val caper = pose().character
                    transformOrigin = TransformOrigin(0.5f, 1f)
                    scaleX = caper.scaleX.toFloat()
                    scaleY = caper.scaleY.toFloat()
                    rotationZ = caper.turn.toFloat()
                    alpha = caper.opacity.toFloat()
                    translationX = (88f - 38f).dp.toPx()
                    translationY = (128f + 4f + caper.y.toFloat() - 76f).dp.toPx()
                }
                .size(76.dp),
        )
    }
}
