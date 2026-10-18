package chat.caper.android

/** Gap-aware gallery cells in dp, in attachment order. Edge flags own only outside corners. */
internal data class AttachmentGalleryCell(
    val x: Float, val y: Float, val width: Float, val height: Float,
    val left: Boolean, val top: Boolean, val right: Boolean, val bottom: Boolean,
)

internal fun attachmentGalleryCells(count: Int, availableWidth: Float): List<AttachmentGalleryCell> {
    require(count >= 2)
    val width = minOf(availableWidth, 480f)
    val gap = 4f
    val height = width * 2f / 3f
    if (count == 2) {
        val cellWidth = (width - gap) / 2f
        return List(2) { index ->
            AttachmentGalleryCell(index * (cellWidth + gap), 0f, cellWidth, height, index == 0, true, index == 1, true)
        }
    }
    if (count == 3) {
        val left = (width - gap) * 2f / 3f
        val right = (width - gap) / 3f
        val halfHeight = (height - gap) / 2f
        return listOf(
            AttachmentGalleryCell(0f, 0f, left, height, true, true, false, true),
            AttachmentGalleryCell(left + gap, 0f, right, halfHeight, false, true, true, false),
            AttachmentGalleryCell(left + gap, halfHeight + gap, right, halfHeight, false, false, true, true),
        )
    }
    val rows = (count + 2) / 3
    val shortRows = rows * 3 - count
    val rowHeight = if (count == 4) (width - gap) / 2f else (width - 2f * gap) / 3f
    return buildList {
        repeat(rows) { row ->
            val columns = if (row < shortRows) 2 else 3
            val cellWidth = (width - gap * (columns - 1)) / columns
            repeat(columns) { column ->
                add(AttachmentGalleryCell(
                    column * (cellWidth + gap), row * (rowHeight + gap), cellWidth, rowHeight,
                    column == 0, row == 0, column == columns - 1, row == rows - 1,
                ))
            }
        }
    }
}
