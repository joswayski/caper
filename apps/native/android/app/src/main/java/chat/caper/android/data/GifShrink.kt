package chat.caper.android.data

import kotlin.math.floor
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * Shrinking an animated GIF wider or taller than 1024 px: every frame is composited,
 * center-cropped square, scaled down, given its own palette of the colours that changed and
 * re-encoded with its timing. Pure Kotlin (no Android APIs), the same algorithm as the web's
 * `gif-shrink.ts`. Pixels are packed ARGB ints.
 */
object GifShrink {
    /** A shrunk GIF's edge, like a re-encoded still's. */
    const val EDGE = 512

    class Failure(message: String) : Exception(message)

    class Shrunk(val bytes: ByteArray, val edge: Int)

    /** [bytes] as a centered square of at most 512 px, every frame and its timing kept. */
    fun shrink(bytes: ByteArray, cancelled: () -> Boolean = { false }): Shrunk {
        val header = header(bytes)
        val edge = minOf(EDGE, header.width, header.height)
        return Shrunk(encode(bytes, edge, cancelled), edge)
    }

    private class Header(val width: Int, val height: Int, val palette: IntArray?, val start: Int)

    private fun u8(bytes: ByteArray, at: Int) = if (at < bytes.size) bytes[at].toInt() and 0xFF else 0
    private fun u16(bytes: ByteArray, at: Int) = u8(bytes, at) or (u8(bytes, at + 1) shl 8)

    private fun palette(bytes: ByteArray, at: Int, entries: Int) = IntArray(entries) { index ->
        val offset = at + index * 3
        (0xFF shl 24) or (u8(bytes, offset) shl 16) or (u8(bytes, offset + 1) shl 8) or u8(bytes, offset + 2)
    }

    private fun header(bytes: ByteArray): Header {
        val signature = if (bytes.size >= 6) String(bytes, 0, 6, Charsets.US_ASCII) else ""
        if (bytes.size < 13 || (signature != "GIF87a" && signature != "GIF89a")) throw Failure("not a GIF")
        val width = u16(bytes, 6)
        val height = u16(bytes, 8)
        if (width == 0 || height == 0) throw Failure("empty GIF")
        val packed = u8(bytes, 10)
        var start = 13
        var global: IntArray? = null
        if (packed and 0x80 != 0) {
            val entries = 2 shl (packed and 7)
            global = palette(bytes, start, entries)
            start += entries * 3
        }
        return Header(width, height, global, start)
    }

    /** The data sub-blocks starting at [at], joined, and where they end. */
    private fun subBlocks(bytes: ByteArray, at: Int): Pair<ByteArray, Int> {
        var length = 0
        var cursor = at
        while (cursor < bytes.size && bytes[cursor].toInt() != 0) {
            length += u8(bytes, cursor)
            cursor += u8(bytes, cursor) + 1
        }
        val data = ByteArray(length)
        var offset = 0
        var block = at
        while (block < cursor && block < bytes.size) {
            val size = u8(bytes, block)
            val count = min(size, bytes.size - block - 1).coerceAtLeast(0)
            System.arraycopy(bytes, block + 1, data, offset, count)
            offset += count
            block += size + 1
        }
        return data.copyOf(offset) to cursor + 1
    }

    /** GIF LZW: codes from [data] into [out] (left as is where the data ends early). */
    private fun lzwDecode(data: ByteArray, minSize: Int, out: IntArray) {
        if (minSize < 2 || minSize > 11) return
        val clear = 1 shl minSize
        val end = clear + 1
        val prefix = IntArray(4096)
        val suffix = IntArray(4096)
        val stack = IntArray(4097)
        for (code in 0 until clear) suffix[code] = code
        var size = minSize + 1
        var mask = (1 shl size) - 1
        var next = clear + 2
        var old = -1
        var first = 0
        var bits = 0
        var accumulator = 0
        var written = 0
        for (byte in data) {
            if (written >= out.size) return
            accumulator = accumulator or ((byte.toInt() and 0xFF) shl bits)
            bits += 8
            while (bits >= size) {
                var code = accumulator and mask
                accumulator = accumulator ushr size
                bits -= size
                if (code == clear) {
                    size = minSize + 1
                    mask = (1 shl size) - 1
                    next = clear + 2
                    old = -1
                    continue
                }
                if (code == end) return
                if (old == -1) {
                    if (code >= clear) return
                    if (written < out.size) out[written++] = code
                    old = code
                    first = code
                    continue
                }
                if (code > next) return
                val current = code
                var depth = 0
                if (code == next) {
                    stack[depth++] = first
                    code = old
                }
                while (code >= clear && depth < 4096) {
                    stack[depth++] = suffix[code]
                    code = prefix[code]
                }
                first = code
                stack[depth++] = first
                while (depth > 0 && written < out.size) out[written++] = stack[--depth]
                if (next < 4096) {
                    prefix[next] = old
                    suffix[next] = first
                    next++
                    if (next == mask + 1 && size < 12) {
                        size++
                        mask = (1 shl size) - 1
                    }
                }
                old = current
            }
        }
    }

    /**
     * Every frame of [bytes] as the full, composited ARGB canvas a viewer sees, with its delay in
     * hundredths of a second. [frame] must use the canvas before returning: it is reused.
     * Returns the frame count.
     */
    fun decode(bytes: ByteArray, cancelled: () -> Boolean = { false }, frame: (IntArray, Int, Int) -> Unit): Int {
        val header = header(bytes)
        val width = header.width
        val height = header.height
        val canvas = IntArray(width * height)
        var saved: IntArray? = null
        var delay = 0
        var transparent = -1
        var disposal = 0
        var previous: IntArray? = null // x, y, w, h, disposal
        var count = 0
        var at = header.start
        while (at < bytes.size) {
            if (cancelled()) throw Failure("cancelled")
            val block = u8(bytes, at++)
            if (block == 0x3B) break
            if (block == 0x21) {
                val label = u8(bytes, at++)
                if (label == 0xF9 && u8(bytes, at) >= 4) {
                    val packed = u8(bytes, at + 1)
                    disposal = (packed shr 2) and 7
                    delay = u16(bytes, at + 2)
                    transparent = if (packed and 1 != 0) u8(bytes, at + 4) else -1
                }
                at = subBlocks(bytes, at).second
                continue
            }
            if (block != 0x2C || at + 9 > bytes.size) break
            val x = u16(bytes, at)
            val y = u16(bytes, at + 2)
            val w = u16(bytes, at + 4)
            val h = u16(bytes, at + 6)
            val packed = u8(bytes, at + 8)
            at += 9
            var colours = header.palette
            if (packed and 0x80 != 0) {
                val entries = 2 shl (packed and 7)
                colours = palette(bytes, at, entries)
                at += entries * 3
            }
            val minSize = u8(bytes, at++)
            val (data, end) = subBlocks(bytes, at)
            at = end
            previous?.let { rect ->
                if (rect[4] == 2) clearRect(canvas, width, height, rect)
                else if (rect[4] == 3) saved?.copyInto(canvas)
            }
            if (disposal == 3) saved = canvas.copyOf()
            val indices = IntArray(w * h)
            if (transparent >= 0) indices.fill(transparent)
            lzwDecode(data, minSize, indices)
            if (colours != null) draw(canvas, width, height, intArrayOf(x, y, w, h), indices, colours, transparent, packed and 0x40 != 0)
            frame(canvas, delay, count++)
            previous = intArrayOf(x, y, w, h, disposal)
            delay = 0
            transparent = -1
            disposal = 0
        }
        if (count == 0) throw Failure("no frames")
        return count
    }

    private fun clearRect(canvas: IntArray, width: Int, height: Int, rect: IntArray) {
        for (row in rect[1] until min(height, rect[1] + rect[3])) {
            if (rect[0] >= width) break
            canvas.fill(0, row * width + rect[0], row * width + min(width, rect[0] + rect[2]))
        }
    }

    /** Interlaced images store rows 0, 8, 16… then 4, 12… then 2, 6… then 1, 3…. */
    private fun rowOrder(h: Int, interlaced: Boolean): IntArray {
        if (!interlaced) return IntArray(h) { it }
        val rows = ArrayList<Int>(h)
        for ((first, step) in listOf(0 to 8, 4 to 8, 2 to 4, 1 to 2)) {
            var row = first
            while (row < h) { rows.add(row); row += step }
        }
        return rows.toIntArray()
    }

    private fun draw(
        canvas: IntArray, width: Int, height: Int, rect: IntArray, indices: IntArray, colours: IntArray,
        transparent: Int, interlaced: Boolean,
    ) {
        val (rx, ry, rw, rh) = rect
        val rows = rowOrder(rh, interlaced)
        for (line in 0 until rh) {
            val y = ry + rows[line]
            if (y >= height) continue
            for (column in 0 until rw) {
                val x = rx + column
                if (x >= width) break
                val index = indices[line * rw + column]
                if (index == transparent || index >= colours.size) continue
                canvas[y * width + x] = colours[index]
            }
        }
    }

    /**
     * The centered square of [source] (width × height) averaged down to [edge] × [edge]. Colour
     * is weighted by alpha so transparent edges don't darken; a pixel is opaque when at least
     * half its area was.
     */
    fun squareScale(source: IntArray, width: Int, height: Int, edge: Int, out: IntArray) {
        val side = min(width, height)
        val left = (width - side) / 2
        val top = (height - side) / 2
        val from = IntArray(edge) { floor(it.toDouble() * side / edge).toInt() }
        val to = IntArray(edge) { maxOf(from[it] + 1, floor((it + 1).toDouble() * side / edge).toInt()) }
        for (oy in 0 until edge) for (ox in 0 until edge) {
            var r = 0L
            var g = 0L
            var b = 0L
            var a = 0L
            for (y in from[oy] until to[oy]) {
                var at = (top + y) * width + left + from[ox]
                for (x in from[ox] until to[ox]) {
                    val pixel = source[at++]
                    val alpha = (pixel ushr 24).toLong()
                    r += ((pixel shr 16) and 0xFF) * alpha
                    g += ((pixel shr 8) and 0xFF) * alpha
                    b += (pixel and 0xFF) * alpha
                    a += alpha
                }
            }
            val area = (to[oy] - from[oy]).toLong() * (to[ox] - from[ox])
            out[oy * edge + ox] = if (a * 2 < area * 255) 0
            else (0xFF shl 24) or ((r / a).toInt() shl 16) or ((g / a).toInt() shl 8) or (b / a).toInt()
        }
    }

    /** Colours are binned at 5 bits per channel; the palette is a median cut of the bins. */
    private fun bin(pixel: Int) = (((pixel shr 19) and 31) shl 10) or (((pixel shr 11) and 31) shl 5) or ((pixel shr 3) and 31)

    private class Histogram {
        private val counts = DoubleArray(32768)
        private val sums = DoubleArray(32768 * 3)
        private val used = ArrayList<Int>()

        fun add(pixel: Int) {
            val key = bin(pixel)
            if (counts[key] == 0.0) used.add(key)
            counts[key]++
            sums[key * 3] += ((pixel shr 16) and 0xFF).toDouble()
            sums[key * 3 + 1] += ((pixel shr 8) and 0xFF).toDouble()
            sums[key * 3 + 2] += (pixel and 0xFF).toDouble()
        }

        private class Box(val keys: List<Int>, val weight: Double, val range: Int, val axis: Int)

        private fun channel(key: Int, axis: Int) = (key shr (10 - axis * 5)) and 31

        private fun box(keys: List<Int>): Box {
            var weight = 0.0
            for (key in keys) weight += counts[key]
            var range = 0
            var axis = 0
            for (a in 0 until 3) {
                var low = 31
                var high = 0
                for (key in keys) {
                    val value = channel(key, a)
                    if (value < low) low = value
                    if (value > high) high = value
                }
                if (high - low > range) { range = high - low; axis = a }
            }
            return Box(keys, weight, range, axis)
        }

        /** Up to [size] ARGB colours, each box's weighted mean; then empties the histogram. */
        fun palette(size: Int): IntArray {
            val boxes = ArrayList<Box>()
            if (used.isNotEmpty()) boxes.add(box(used.toList()))
            while (boxes.size < size) {
                var chosen = -1
                var score = 0.0
                boxes.forEachIndexed { index, candidate ->
                    val value = candidate.weight * (candidate.range + 1)
                    if (candidate.range > 0 && value > score) { score = value; chosen = index }
                }
                if (chosen < 0) break
                val split = boxes[chosen]
                val keys = split.keys.sortedBy { channel(it, split.axis) }
                var running = 0.0
                var cut = 1
                while (cut < keys.size - 1) {
                    running += counts[keys[cut - 1]]
                    if (running >= split.weight / 2) break
                    cut++
                }
                boxes[chosen] = box(keys.subList(0, cut))
                boxes.add(chosen + 1, box(keys.subList(cut, keys.size)))
            }
            val palette = IntArray(maxOf(1, boxes.size)) { 0xFF shl 24 }
            boxes.forEachIndexed { index, box ->
                var r = 0.0
                var g = 0.0
                var b = 0.0
                for (key in box.keys) { r += sums[key * 3]; g += sums[key * 3 + 1]; b += sums[key * 3 + 2] }
                palette[index] = (0xFF shl 24) or ((r / box.weight).roundToInt() shl 16) or
                    ((g / box.weight).roundToInt() shl 8) or (b / box.weight).roundToInt()
            }
            for (key in used) {
                counts[key] = 0.0
                sums.fill(0.0, key * 3, key * 3 + 3)
            }
            used.clear()
            return palette
        }
    }

    /** Nearest palette entry per colour bin, found once per bin. */
    private class Mapper(private val palette: IntArray) {
        private val cache = IntArray(32768) { -1 }

        fun index(pixel: Int): Int {
            val key = bin(pixel)
            val cached = cache[key]
            if (cached >= 0) return cached
            val r = ((key shr 10) shl 3) or 4
            val g = (((key shr 5) and 31) shl 3) or 4
            val b = ((key and 31) shl 3) or 4
            var best = 0
            var distance = Int.MAX_VALUE
            palette.forEachIndexed { entry, colour ->
                val dr = ((colour shr 16) and 0xFF) - r
                val dg = ((colour shr 8) and 0xFF) - g
                val db = (colour and 0xFF) - b
                val d = dr * dr * 2 + dg * dg * 4 + db * db * 3
                if (d < distance) { distance = d; best = entry }
            }
            cache[key] = best
            return best
        }
    }

    private class Output {
        private var bytes = ByteArray(1 shl 16)
        var size = 0
            private set

        fun byte(value: Int) {
            if (size == bytes.size) bytes = bytes.copyOf(bytes.size * 2)
            bytes[size++] = value.toByte()
        }

        fun word(value: Int) { byte(value and 0xFF); byte((value shr 8) and 0xFF) }

        fun all(vararg values: Int) { for (value in values) byte(value) }

        fun ascii(text: String) { for (char in text) byte(char.code) }

        fun result() = bytes.copyOf(size)
    }

    /** 8-bit LZW into 255-byte sub-blocks, as omggif's writer does. */
    private fun lzwEncode(indices: IntArray, out: Output) {
        val clear = 256
        val end = 257
        out.byte(8)
        val block = IntArray(255)
        var filled = 0
        var accumulator = 0
        var bits = 0
        var size = 9
        fun emit(code: Int) {
            accumulator = accumulator or (code shl bits)
            bits += size
            while (bits >= 8) {
                block[filled++] = accumulator and 0xFF
                accumulator = accumulator ushr 8
                bits -= 8
                if (filled == 255) {
                    out.byte(255)
                    for (value in block) out.byte(value)
                    filled = 0
                }
            }
        }
        // Open addressing on (prefix shl 8 or index); 4096 codes at most.
        val keys = IntArray(8192) { -1 }
        val codes = IntArray(8192)
        var next = end + 1
        emit(clear)
        var current = indices[0]
        for (at in 1 until indices.size) {
            val index = indices[at]
            val key = (current shl 8) or index
            var slot = (key * -0x61c8864f) ushr 19
            while (keys[slot] != -1 && keys[slot] != key) slot = (slot + 1) and 8191
            if (keys[slot] == key) {
                current = codes[slot]
                continue
            }
            emit(current)
            if (next == 4096) {
                emit(clear)
                keys.fill(-1)
                next = end + 1
                size = 9
            } else {
                if (next >= 1 shl size) size++
                keys[slot] = key
                codes[slot] = next++
            }
            current = index
        }
        emit(current)
        emit(end)
        if (bits > 0) block[filled++] = accumulator and 0xFF
        if (filled > 0) {
            out.byte(filled)
            for (i in 0 until filled) out.byte(block[i])
        }
        out.byte(0)
    }

    private class Pending(val rect: IntArray, val palette: IntArray, val indices: IntArray, val delay: Int)

    /**
     * Re-encodes [bytes] as an [edge] × [edge] GIF, every frame with its delay. Each frame gets its own palette of the colours that changed; unchanged pixels are left showing
     * (transparent), unless the animation has transparency of its own, in which case every frame
     * is drawn whole on a cleared canvas.
     */
    fun encode(bytes: ByteArray, edge: Int, cancelled: () -> Boolean = { false }): ByteArray {
        val header = header(bytes)
        val width = header.width
        val height = header.height
        val side = min(width, height)
        val left = (width - side) / 2
        val top = (height - side) / 2
        var transparency = false
        decode(bytes, cancelled) { canvas, _, _ ->
            if (transparency) return@decode
            for (y in top until top + side) {
                for (x in left until left + side) if (canvas[y * width + x] ushr 24 == 0) { transparency = true; break }
                if (transparency) break
            }
        }
        val out = Output()
        out.ascii("GIF89a")
        out.word(edge)
        out.word(edge)
        // No global palette: each frame carries its own.
        out.all(0x70, 0, 0)
        out.all(0x21, 0xFF, 11)
        out.ascii("NETSCAPE2.0")
        out.all(3, 1, 0, 0, 0)
        fun write(frame: Pending) {
            // Index 255 is transparent: real transparency, or "unchanged" between frames.
            out.all(0x21, 0xF9, 4, ((if (transparency) 2 else 1) shl 2) or 1)
            out.word(min(0xFFFF, frame.delay))
            out.all(255, 0, 0x2C)
            for (value in frame.rect) out.word(value)
            out.byte(0x87)
            for (entry in 0 until 256) {
                val colour = frame.palette.getOrElse(entry) { 0 }
                out.all((colour shr 16) and 0xFF, (colour shr 8) and 0xFF, colour and 0xFF)
            }
            lzwEncode(frame.indices, out)
        }
        val scaled = IntArray(edge * edge)
        var previous: IntArray? = null
        val histogram = Histogram()
        var pending: Pending? = null
        decode(bytes, cancelled) { canvas, delay, _ ->
            pending?.let(::write)
            squareScale(canvas, width, height, edge, scaled)
            val last = previous
            fun changed(pixel: Int) = transparency || last == null || scaled[pixel] != last[pixel]
            var x0 = 0
            var y0 = 0
            var x1 = edge - 1
            var y1 = edge - 1
            if (!transparency && last != null) {
                x0 = edge; y0 = edge; x1 = -1; y1 = -1
                for (pixel in 0 until edge * edge) if (scaled[pixel] != last[pixel]) {
                    val x = pixel % edge
                    val y = pixel / edge
                    if (x < x0) x0 = x
                    if (x > x1) x1 = x
                    if (y < y0) y0 = y
                    if (y > y1) y1 = y
                }
            }
            if (x1 < 0) {
                // Nothing changed: one transparent pixel still holds the frame's time.
                pending = Pending(intArrayOf(0, 0, 1, 1), IntArray(1), intArrayOf(255), delay)
                return@decode
            }
            val w = x1 - x0 + 1
            val h = y1 - y0 + 1
            for (y in y0..y1) for (x in x0..x1) {
                val pixel = y * edge + x
                if (scaled[pixel] ushr 24 != 0 && changed(pixel)) histogram.add(scaled[pixel])
            }
            val palette = histogram.palette(255)
            val mapper = Mapper(palette)
            val indices = IntArray(w * h)
            for (y in 0 until h) for (x in 0 until w) {
                val pixel = (y0 + y) * edge + x0 + x
                indices[y * w + x] = if (scaled[pixel] ushr 24 != 0 && changed(pixel)) mapper.index(scaled[pixel]) else 255
            }
            pending = Pending(intArrayOf(x0, y0, w, h), palette, indices, delay)
            previous = scaled.copyOf()
        }
        pending?.let(::write)
        out.byte(0x3B)
        return out.result()
    }
}
