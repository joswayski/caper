package chat.caper.android

import org.junit.Assert.*
import org.junit.Test

class AttachmentGalleryTest {
    @Test fun `two visuals split the gap and three reserve the spanning left cell`() {
        val two = attachmentGalleryCells(2, 480f)
        assertEquals(238f, two[0].width, 0.001f)
        assertEquals(242f, two[1].x, 0.001f)
        assertEquals(320f, two[0].height, 0.001f)
        val three = attachmentGalleryCells(3, 480f)
        assertEquals(476f * 2f / 3f, three[0].width, 0.001f)
        assertEquals(476f / 3f, three[1].width, 0.001f)
        assertEquals(320f, three[0].height, 0.001f)
        assertEquals(158f, three[1].height, 0.001f)
        assertEquals(162f, three[2].y, 0.001f)
    }

    @Test fun `larger galleries put short rows first and use gap aware row heights`() {
        val expected = mapOf(4 to listOf(2, 2), 5 to listOf(2, 3), 6 to listOf(3, 3), 7 to listOf(2, 2, 3), 8 to listOf(2, 3, 3), 9 to listOf(3, 3, 3))
        expected.forEach { (count, rows) ->
            val cells = attachmentGalleryCells(count, 480f)
            assertEquals(rows, cells.groupBy { it.y }.values.map { it.size })
            cells.forEach { assertEquals(if (count == 4) 238f else 472f / 3f, it.height, 0.001f) }
        }
    }

    @Test fun `all gallery counts fit the available width with four outside corners and no overlaps`() {
        for (available in listOf(240f, 343f, 480f, 800f)) for (count in 2..30) {
            val width = minOf(480f, available)
            val cells = attachmentGalleryCells(count, available)
            assertEquals(count, cells.size)
            assertEquals(4, cells.sumOf { cell ->
                listOf(cell.left && cell.top, cell.right && cell.top, cell.left && cell.bottom, cell.right && cell.bottom).count { it }
            })
            cells.forEachIndexed { index, cell ->
                assertTrue(cell.x >= 0f && cell.y >= 0f && cell.width > 0f && cell.height > 0f)
                assertTrue(cell.x + cell.width <= width + 0.001f)
                if (cell.right) assertEquals(width, cell.x + cell.width, 0.001f)
                cells.drop(index + 1).forEach { other ->
                    assertTrue(cell.x + cell.width + 3.999f <= other.x || other.x + other.width + 3.999f <= cell.x ||
                        cell.y + cell.height + 3.999f <= other.y || other.y + other.height + 3.999f <= cell.y)
                }
            }
        }
    }
}
