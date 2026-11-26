package xyz.block.buzz.mobile

import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private val metadataBoxTypes = setOf(
    "meta", "ilst", "keys", "data", "uuid", "xml ", "bxml", "loci", "\u00a9xyz", "name", "chap", "udta",
)
private val containerBoxTypes = setOf(
    "moov", "trak", "mdia", "minf", "stbl", "edts", "dinf", "sinf", "schi",
)

/**
 * Android's `MediaMuxer` (AOSP `MPEG4Writer`) always writes a `moov/meta` box
 * carrying `com.android.version`, writes `moov/udta` when a location is set,
 * and only reserves about 3 KiB for the `moov` atom, so anything longer than a
 * few seconds ends up with `mdat` before `moov`. The relay rejects all three
 * shapes, so a packaged voice note or transcoded video is not uploadable until
 * the envelope is rewritten.
 */
class AndroidMediaSanitizerMp4Test {
    @Test
    fun `scrubMp4File produces a fast-start envelope with valid chunk offsets`() {
        for (moovFirst in listOf(true, false)) {
            for (co64 in listOf(false, true)) {
                val case = "moovFirst=$moovFirst co64=$co64"
                val original = androidMuxerMp4(moovFirst = moovFirst, co64 = co64)
                val removedBytes = metadataBytes(original)

                val scrubbed = scrub(original)

                val before = topLevelBoxes(original)
                val after = topLevelBoxes(scrubbed)
                val expectedOrder = if (moovFirst) {
                    listOf("ftyp", "free", "moov", "mdat")
                } else {
                    listOf("ftyp", "moov", "free", "mdat")
                }
                assertEquals(expectedOrder, after.map { it.type }, case)
                assertEquals(original.size - removedBytes, scrubbed.size, case)

                val oldMdatStart = before.single { it.type == "mdat" }.start
                val newMdatStart = after.single { it.type == "mdat" }.start
                assertEquals(
                    chunkOffsets(original).map { it + newMdatStart - oldMdatStart },
                    chunkOffsets(scrubbed),
                    "$case chunk offsets must track the shifted mdat",
                )
                assertContentEquals(mdatPayload(original), mdatPayload(scrubbed), case)
                assertFalse(scrubbed.hasAscii("com.android.version"), case)
                assertFalse(scrubbed.hasAscii("+37.7749-122.4194/"), case)
            }
        }
    }

    @Test
    fun `scrubMp4File shrinks a 64-bit mdat header so WebKit accepts the file`() {
        // MediaMuxer writes mdat with a 64-bit size field. WebKitGTK rejects
        // such a file outright (MEDIA_ERR_SRC_NOT_SUPPORTED) even though ffmpeg
        // and ExoPlayer play it, which is why desktop playback of every voice
        // note failed while the phone was fine.
        for (moovFirst in listOf(true, false)) {
            val case = "moovFirst=$moovFirst"
            val original = androidMuxerMp4(moovFirst = moovFirst, co64 = true, largeMdat = true)
            assertEquals(16, topLevelBoxes(original).single { it.type == "mdat" }.headerSize, case)

            val scrubbed = scrub(original)

            val before = topLevelBoxes(original).single { it.type == "mdat" }
            val mdat = topLevelBoxes(scrubbed).single { it.type == "mdat" }
            assertEquals(8, mdat.headerSize, "$case mdat must use a compact size field")
            // Exact offsets, measured from the box geometry: an offset that
            // merely lands "somewhere inside mdat" would not catch a missing
            // header-size term in the sanitizer's shift.
            val payloadShift = mdat.bodyStart - before.bodyStart
            assertEquals(
                chunkOffsets(original).map { it + payloadShift },
                chunkOffsets(scrubbed),
                "$case chunk offsets must follow the shrunk header",
            )
            assertContentEquals(mdatPayload(original), mdatPayload(scrubbed), case)
        }
    }

    @Test
    fun `scrubMp4File fails closed for fragmented containers`() {
        // moof/sidx offsets are not rewritten, so a fragmented input must not
        // be silently turned into a fast-start file with stale fragment offsets.
        val fragmented = canonicalMp4() + box("moof", ByteArray(32) { 9 })
        assertFailsWith<IllegalArgumentException> { scrub(fragmented) }
    }

    @Test
    fun `scrubMp4File repairs a real MediaMuxer envelope`() {
        // ffmpeg-encoded H.264/AAC, laid out with mdat before moov the way
        // MPEG4Writer does once its ~3 KiB moov reserve is exhausted. The
        // `moov/meta` box and the two `udta` boxes (one injected, one ffmpeg's
        // own encoder tag) reproduce what the muxer writes there.
        val fixture = fixtureBytes("voice-note-android-muxer.mp4")

        val scrubbed = scrub(fixture)

        val before = topLevelBoxes(fixture)
        val after = topLevelBoxes(scrubbed)
        assertTrue(
            after.indexOfFirst { it.type == "moov" } < after.indexOfFirst { it.type == "mdat" },
            "moov must lead mdat",
        )
        assertEquals(fixture.size - removedMetadataBytes(fixture), scrubbed.size)
        val delta = after.single { it.type == "mdat" }.start - before.single { it.type == "mdat" }.start
        assertEquals(
            chunkOffsets(fixture).map { it + delta },
            chunkOffsets(scrubbed),
            "sample offsets must follow the relocated mdat",
        )
        val mdat = after.single { it.type == "mdat" }
        assertTrue(
            chunkOffsets(scrubbed).all { it >= mdat.bodyStart && it < mdat.end },
            "every chunk offset must land inside mdat",
        )
        assertContentEquals(mdatPayload(fixture), mdatPayload(scrubbed))
        assertEquals(emptyList(), metadataBoxes(scrubbed))
        assertFalse(scrubbed.hasAscii("com.android.version"))
        assertFalse(scrubbed.hasAscii("+37.7749-122.4194/"))
    }

    @Test
    fun `scrubMp4File is idempotent`() {
        val file = tempMp4(androidMuxerMp4(moovFirst = false, co64 = false))
        try {
            AndroidMediaSanitizer.scrubMp4File(file)
            val once = file.readBytes()

            AndroidMediaSanitizer.scrubMp4File(file)

            assertContentEquals(once, file.readBytes())
        } finally {
            file.delete()
        }
    }

    @Test
    fun `scrubMp4File leaves a canonical envelope untouched`() {
        val canonical = canonicalMp4()
        val file = tempMp4(canonical)
        try {
            AndroidMediaSanitizer.scrubMp4File(file)

            assertContentEquals(canonical, file.readBytes())
        } finally {
            file.delete()
        }
    }

    @Test
    fun `scrubMp4File drops metadata nested inside a container`() {
        val nested = box(
            "mdia",
            box("mdhd", ByteArray(24) { 3 }) +
                box("udta", "+37.7749-122.4194/".encodeToByteArray()) +
                box("stbl", box("meta", "com.android.version".encodeToByteArray())),
        )
        val original = ftypBox() +
            box(
                "moov",
                box("mvhd", ByteArray(100) { 1 }) +
                    box(
                        "trak",
                        box("tkhd", ByteArray(84) { 2 }) +
                            box("loci", "+37.7749+122.4194/".encodeToByteArray()) +
                            nested,
                    ),
            ) +
            box("mdat", ByteArray(64) { it.toByte() })

        val scrubbed = scrub(original)

        val moov = topLevelBoxes(scrubbed).single { it.type == "moov" }
        val trak = childBoxes(scrubbed, moov).single { it.type == "trak" }
        assertEquals(listOf("tkhd", "mdia"), childBoxes(scrubbed, trak).map { it.type })
        val mdia = childBoxes(scrubbed, trak).single { it.type == "mdia" }
        assertEquals(listOf("mdhd", "stbl"), childBoxes(scrubbed, mdia).map { it.type })
        val stbl = childBoxes(scrubbed, mdia).single { it.type == "stbl" }
        assertEquals(emptyList<Box>(), childBoxes(scrubbed, stbl))
        assertFalse(scrubbed.hasAscii("+37.7749"))
        assertFalse(scrubbed.hasAscii("com.android.version"))
    }

    @Test
    fun `scrubMp4File fails closed for malformed containers`() {
        val truncatedChild = ftypBox() +
            box("moov", box("mvhd", ByteArray(8)) + byteArrayOf(0, 0, 0, 40, 'm'.code.toByte(), 'e'.code.toByte(), 't'.code.toByte(), 'a'.code.toByte()))
        val childShorterThanHeader = ftypBox() +
            box("moov", byteArrayOf(0, 0, 0, 4, 'm'.code.toByte(), 'e'.code.toByte(), 't'.code.toByte(), 'a'.code.toByte()))
        val partialHeader = ftypBox() + box("moov", box("mvhd", ByteArray(8)) + byteArrayOf(0, 0, 0, 0))
        val noMdat = ftypBox() + box("moov", box("mvhd", ByteArray(8)))
        val twoMoovs = ftypBox() +
            box("moov", box("mvhd", ByteArray(8))) +
            box("moov", box("mvhd", ByteArray(8))) +
            box("mdat", ByteArray(8))
        val offsetTablePastEnd = ftypBox() +
            box(
                "moov",
                box("trak", box("stbl", box("stco", ByteArray(4) + int32(4) + int32(1)))),
            ) +
            box("mdat", ByteArray(8))

        val malformed = listOf(
            "empty file" to ByteArray(0),
            "truncated child" to truncatedChild,
            "child shorter than its header" to childShorterThanHeader,
            "partial box header" to partialHeader,
            "no mdat" to noMdat,
            "two moov boxes" to twoMoovs,
            "chunk offset table past its box" to offsetTablePastEnd,
        )
        for ((name, bytes) in malformed) {
            val file = tempMp4(bytes)
            try {
                assertFailsWith<IllegalArgumentException>(name) {
                    AndroidMediaSanitizer.scrubMp4File(file)
                }
                assertContentEquals(bytes, file.readBytes(), "$name must not be replaced")
            } finally {
                file.delete()
            }
        }
    }

    /**
     * `ftyp` + reserved `free` space + `moov` + `mdat`, in either the
     * fast-start order a short recording gets or the trailing order a longer
     * one gets once the moov outgrows the reserve.
     */
    private fun androidMuxerMp4(
        moovFirst: Boolean,
        co64: Boolean,
        largeMdat: Boolean = false,
    ): ByteArray {
        val ftyp = ftypBox()
        val reserved = box("free", ByteArray(2048))
        val mdatBody = ByteArray(512) { (it * 7).toByte() }
        val mdat = if (largeMdat) box64("mdat", mdatBody) else box("mdat", mdatBody)
        // Chunk offsets point at the payload, which starts after whichever
        // header shape the muxer wrote.
        val payloadOffset = if (largeMdat) 16L else 8L
        // Chunk offsets are absolute, and the moov has a fixed width whatever
        // the offsets hold, so measuring a placeholder moov is exact.
        val placeholder = moovBox(co64, listOf(0L, 0L))
        val mdatStart = if (moovFirst) {
            ftyp.size + reserved.size + placeholder.size
        } else {
            ftyp.size + reserved.size
        }
        val firstChunk = mdatStart + payloadOffset
        val moov = moovBox(co64, listOf(firstChunk, firstChunk + 256L))
        return if (moovFirst) ftyp + reserved + moov + mdat else ftyp + reserved + mdat + moov
    }

    private fun moovBox(co64: Boolean, chunkOffsets: List<Long>): ByteArray {
        val offsetTable = if (co64) co64Box(chunkOffsets) else stcoBox(chunkOffsets)
        val meta = box(
            "meta",
            box("hdlr", "mdta".encodeToByteArray()) + box("keys", "com.android.version".encodeToByteArray()),
        )
        val udta = box(
            "udta",
            box(
                byteArrayOf(0xA9.toByte(), 'x'.code.toByte(), 'y'.code.toByte(), 'z'.code.toByte()),
                "+37.7749-122.4194/".encodeToByteArray(),
            ),
        )
        val stbl = box(
            "stbl",
            box("stsd", ByteArray(16)) +
                box("stts", ByteArray(16)) +
                box("stsc", ByteArray(16)) +
                box("stsz", ByteArray(20)) +
                offsetTable,
        )
        val trak = box(
            "trak",
            box("tkhd", ByteArray(84) { 2 }) +
                box(
                    "mdia",
                    box("mdhd", ByteArray(24) { 3 }) +
                        box(
                            "minf",
                            box("smhd", ByteArray(8)) +
                                box("dinf", box("dref", box("url ", ByteArray(4)))) +
                                stbl,
                        ),
                ),
        )
        return box("moov", box("mvhd", ByteArray(100) { 1 }) + meta + udta + trak)
    }

    private fun canonicalMp4(): ByteArray = ftypBox() +
        box(
            "moov",
            box("mvhd", ByteArray(100) { 1 }) +
                box(
                    "trak",
                    box("tkhd", ByteArray(84) { 2 }) +
                        box("mdia", box("mdhd", ByteArray(24) { 3 }) + box("minf", box("stbl", stcoBox(listOf(0L, 0L))))),
                ),
        ) +
        box("mdat", ByteArray(512) { it.toByte() })

    private fun scrub(bytes: ByteArray): ByteArray {
        val file = tempMp4(bytes)
        try {
            AndroidMediaSanitizer.scrubMp4File(file)
            return file.readBytes()
        } finally {
            file.delete()
        }
    }

    private fun tempMp4(bytes: ByteArray): File {
        val file = Files.createTempFile("buzz-sanitizer", ".mp4").toFile()
        file.writeBytes(bytes)
        return file
    }

    private fun metadataBytes(bytes: ByteArray): Int {
        val moov = topLevelBoxes(bytes).single { it.type == "moov" }
        return childBoxes(bytes, moov).filter { it.type == "meta" || it.type == "udta" }.sumOf { it.size }
    }

    private fun removedMetadataBytes(bytes: ByteArray): Int {
        val removed = mutableListOf<Int>()
        fun walk(boxes: List<Box>) {
            for (box in boxes) {
                when (box.type) {
                    in metadataBoxTypes -> removed += box.size
                    in containerBoxTypes -> walk(childBoxes(bytes, box))
                }
            }
        }
        walk(topLevelBoxes(bytes))
        return removed.sum()
    }

    private fun metadataBoxes(bytes: ByteArray): List<String> {
        val found = mutableListOf<String>()
        fun walk(boxes: List<Box>) {
            for (box in boxes) {
                when (box.type) {
                    in metadataBoxTypes -> found += box.type
                    in containerBoxTypes -> walk(childBoxes(bytes, box))
                }
            }
        }
        walk(topLevelBoxes(bytes))
        return found
    }

    private fun chunkOffsets(bytes: ByteArray): List<Long> {
        val moov = topLevelBoxes(bytes).single { it.type == "moov" }
        val offsets = mutableListOf<Long>()
        for (trak in childBoxes(bytes, moov).filter { it.type == "trak" }) {
            val mdia = childBoxes(bytes, trak).single { it.type == "mdia" }
            val minf = childBoxes(bytes, mdia).single { it.type == "minf" }
            val stbl = childBoxes(bytes, minf).single { it.type == "stbl" }
            for (table in childBoxes(bytes, stbl)) {
                val entrySize = when (table.type) {
                    "stco" -> 4
                    "co64" -> 8
                    else -> continue
                }
                val count = readUnsignedInt(bytes, table.bodyStart + 4).toInt()
                repeat(count) { index ->
                    val at = table.bodyStart + 8 + index * entrySize
                    offsets += if (entrySize == 4) {
                        readUnsignedInt(bytes, at)
                    } else {
                        readUnsignedLong(bytes, at)
                    }
                }
            }
        }
        return offsets
    }

    private fun fixtureBytes(name: String): ByteArray =
        requireNotNull(javaClass.getResourceAsStream("/fixtures/android/$name")) {
            "Missing fixture: $name"
        }.use { it.readBytes() }

    private fun mdatPayload(bytes: ByteArray): ByteArray {
        val mdat = topLevelBoxes(bytes).single { it.type == "mdat" }
        return bytes.copyOfRange(mdat.bodyStart, mdat.end)
    }

    private class Box(val type: String, val start: Int, val headerSize: Int, val end: Int) {
        val bodyStart: Int get() = start + headerSize
        val size: Int get() = end - start
    }

    private fun topLevelBoxes(bytes: ByteArray): List<Box> = boxes(bytes, 0, bytes.size)

    private fun childBoxes(bytes: ByteArray, parent: Box): List<Box> =
        boxes(bytes, parent.bodyStart, parent.end)

    private fun boxes(bytes: ByteArray, start: Int, end: Int): List<Box> {
        val result = mutableListOf<Box>()
        var offset = start
        while (offset < end) {
            require(end - offset >= 8) { "truncated box" }
            val compact = readUnsignedInt(bytes, offset)
            val type = String(bytes, offset + 4, 4, StandardCharsets.ISO_8859_1)
            val headerSize = if (compact == 1L) 16 else 8
            val size = when {
                compact == 1L -> readUnsignedLong(bytes, offset + 8)
                compact == 0L -> (end - offset).toLong()
                else -> compact
            }
            require(size >= headerSize && offset + size <= end) { "invalid box size" }
            result += Box(type, offset, headerSize, (offset + size).toInt())
            offset += size.toInt()
        }
        return result
    }

    private fun ByteArray.hasAscii(value: String): Boolean {
        val needle = value.encodeToByteArray()
        for (start in 0..size - needle.size) {
            if (needle.indices.all { this[start + it] == needle[it] }) return true
        }
        return false
    }

    private fun ftypBox(): ByteArray = box("ftyp", "isom\u0000\u0000\u0002isommp42".encodeToByteArray())

    private fun stcoBox(offsets: List<Long>): ByteArray =
        box("stco", ByteArray(4) + int32(offsets.size) + offsets.map { int32(it.toInt()) }.concat())

    private fun co64Box(offsets: List<Long>): ByteArray =
        box("co64", ByteArray(4) + int32(offsets.size) + offsets.map { long64(it) }.concat())

    private fun box(type: String, payload: ByteArray): ByteArray =
        box(type.toByteArray(StandardCharsets.US_ASCII), payload)

    private fun box(type: ByteArray, payload: ByteArray): ByteArray {
        val output = ByteArrayOutputStream()
        output.write(int32(payload.size + 8))
        output.write(type)
        output.write(payload)
        return output.toByteArray()
    }

    /** A box with the 64-bit size field MediaMuxer writes for `mdat`. */
    private fun box64(type: String, payload: ByteArray): ByteArray {
        val output = ByteArrayOutputStream()
        output.write(int32(1))
        output.write(type.toByteArray(StandardCharsets.US_ASCII))
        output.write(long64(payload.size + 16L))
        output.write(payload)
        return output.toByteArray()
    }

    private fun List<ByteArray>.concat(): ByteArray {
        val output = ByteArrayOutputStream()
        for (part in this) output.write(part)
        return output.toByteArray()
    }

    private fun int32(value: Int): ByteArray = byteArrayOf(
        (value ushr 24).toByte(),
        (value ushr 16).toByte(),
        (value ushr 8).toByte(),
        value.toByte(),
    )

    private fun long64(value: Long): ByteArray = int32((value ushr 32).toInt()) + int32(value.toInt())

    private fun readUnsignedInt(bytes: ByteArray, offset: Int): Long =
        ((bytes[offset].toLong() and 0xFF) shl 24) or
            ((bytes[offset + 1].toLong() and 0xFF) shl 16) or
            ((bytes[offset + 2].toLong() and 0xFF) shl 8) or
            (bytes[offset + 3].toLong() and 0xFF)

    private fun readUnsignedLong(bytes: ByteArray, offset: Int): Long {
        var value = 0L
        for (index in 0 until 8) {
            value = (value shl 8) or (bytes[offset + index].toLong() and 0xFF)
        }
        return value
    }
}
