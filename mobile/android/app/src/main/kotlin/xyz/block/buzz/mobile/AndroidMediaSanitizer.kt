package xyz.block.buzz.mobile

import java.io.ByteArrayOutputStream
import java.io.File
import java.io.OutputStream
import java.io.RandomAccessFile
import java.nio.charset.StandardCharsets

internal object AndroidMediaSanitizer {
    private val pngSignature = byteArrayOf(
        0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
    )
    private val allowedPngAncillaryChunks = setOf(
        "cHRM", "gAMA", "sBIT", "sRGB", "bKGD", "hIST", "tRNS", "sPLT", "acTL", "fcTL", "fdAT",
    )

    fun scrubPng(bytes: ByteArray): ByteArray {
        require(bytes.size >= pngSignature.size && bytes.copyOfRange(0, pngSignature.size).contentEquals(pngSignature)) {
            "Invalid PNG signature"
        }

        val output = ByteArrayOutputStream(bytes.size)
        output.write(pngSignature)
        var offset = pngSignature.size
        while (offset < bytes.size) {
            require(bytes.size - offset >= PNG_CHUNK_OVERHEAD) { "Truncated PNG chunk" }
            val payloadLength = readUnsignedIntBigEndian(bytes, offset)
            require(payloadLength <= bytes.size.toLong() - offset - PNG_CHUNK_OVERHEAD) {
                "Invalid PNG chunk length"
            }
            val chunkLength = payloadLength.toInt() + PNG_CHUNK_OVERHEAD
            val typeStart = offset + 4
            val type = String(bytes, typeStart, 4, StandardCharsets.US_ASCII)
            val isAncillary = bytes[typeStart].toInt() and 0x20 != 0
            if (!isAncillary || type in allowedPngAncillaryChunks) {
                output.write(bytes, offset, chunkLength)
            }
            offset += chunkLength
            if (type == "IEND") {
                return output.toByteArray()
            }
        }

        throw IllegalArgumentException("PNG is missing IEND")
    }

    fun scrubJpeg(bytes: ByteArray): ByteArray {
        require(
            bytes.size >= 2 &&
                bytes[0] == 0xFF.toByte() &&
                (bytes[1].toInt() and 0xFF) == JPEG_SOI,
        ) {
            "Invalid JPEG signature"
        }

        val output = ByteArrayOutputStream(bytes.size)
        output.write(byteArrayOf(0xFF.toByte(), JPEG_SOI.toByte()))
        var offset = 2
        var inScan = false
        while (offset < bytes.size) {
            if (inScan && bytes[offset] != 0xFF.toByte()) {
                val nextMarker = bytes.indexOf(0xFF.toByte(), offset).let { if (it == -1) bytes.size else it }
                output.write(bytes, offset, nextMarker - offset)
                offset = nextMarker
                continue
            }
            require(bytes[offset] == 0xFF.toByte()) { "Invalid JPEG marker" }

            val markerStart = offset
            while (offset < bytes.size && bytes[offset] == 0xFF.toByte()) {
                offset += 1
            }
            require(offset < bytes.size) { "Truncated JPEG marker" }

            val marker = bytes[offset].toInt() and 0xFF
            offset += 1
            if (inScan && marker == 0x00) {
                output.write(bytes, markerStart, offset - markerStart)
                continue
            }
            if (marker in JPEG_RESTART_MARKERS || marker == JPEG_TEMP) {
                output.write(bytes, markerStart, offset - markerStart)
                continue
            }
            if (marker == JPEG_EOI) {
                output.write(bytes, markerStart, offset - markerStart)
                return output.toByteArray()
            }
            require(marker != JPEG_SOI && bytes.size - offset >= 2) { "Invalid JPEG segment" }

            val segmentLength = readUnsignedShortBigEndian(bytes, offset)
            require(segmentLength >= 2 && segmentLength <= bytes.size - offset) { "Invalid JPEG segment length" }
            val segmentEnd = offset + segmentLength
            if (shouldKeepJpegSegment(marker, bytes, offset + 2, segmentEnd)) {
                output.write(bytes, markerStart, segmentEnd - markerStart)
            }
            offset = segmentEnd
            inScan = marker == JPEG_SOS
        }

        throw IllegalArgumentException("JPEG is missing EOI")
    }

    /**
     * Rewrites an MP4 assembled by Android's `MediaMuxer` into the canonical
     * envelope the relay accepts.
     *
     * Two things about AOSP's `MPEG4Writer` output have to be undone:
     *
     * - It always writes `com.android.version` into a `moov/meta` box
     *   (`writeMoovLevelMetaBox`), and a `moov/udta` box when the caller
     *   supplies a location. The relay rejects both as private metadata
     *   channels. Metadata boxes are removed outright.
     * - It only reserves about 3 KiB of `free` space for `moov`. Once a
     *   recording has enough samples that `moov` outgrows that reserve — a few
     *   seconds of video, roughly fifteen seconds of audio — the moov lands
     *   *after* `mdat`. The relay requires fast-start, so the moov is moved in
     *   front of `mdat` here.
     *
     * Both operations move `mdat`, and `stco`/`co64` chunk offsets are absolute
     * file offsets, so every offset table is rewritten by the resulting delta.
     * The rewrite goes to a sibling temp file and replaces the original only
     * after it is complete; malformed input fails closed rather than producing
     * a container the relay would reject later.
     */
    fun scrubMp4File(file: File) {
        val target = file.absoluteFile
        val temporary = File(target.parentFile, "${target.name}.canonical")
        try {
            RandomAccessFile(target, "r").use { container ->
                temporary.outputStream().use { output ->
                    canonicaliseMp4(container, container.length(), output)
                }
            }
            require(temporary.renameTo(target)) { "Unable to replace the MP4 after scrubbing." }
        } finally {
            temporary.delete()
        }
    }

    private fun canonicaliseMp4(
        container: RandomAccessFile,
        end: Long,
        output: OutputStream,
    ) {
        require(end >= MP4_BOX_HEADER_BYTES) { "Invalid MP4: no boxes" }
        val boxes = readMp4TopLevelBoxes(container, end)
        val moovBoxes = boxes.filter { it.type == "moov" }
        val mdatBoxes = boxes.filter { it.type == "mdat" }
        require(moovBoxes.size == 1) { "Invalid MP4: expected one moov box" }
        require(mdatBoxes.size == 1) { "Invalid MP4: expected one mdat box" }
        // Fragmented containers carry their offsets in moof/sidx, which this
        // rewrite does not touch. Fail closed rather than emit a fast-start file
        // whose fragment offsets point at the wrong bytes.
        require(boxes.none { it.type == "moof" || it.type == "styp" || it.type == "sidx" }) {
            "Fragmented MP4 is not supported"
        }
        val moovBox = moovBoxes.single()
        val mdatBox = mdatBoxes.single()

        val moovBody = filterMp4Metadata(readMp4BoxBody(container, moovBox), 0)
        val moovSize = moovBody.size.toLong() + MP4_BOX_HEADER_BYTES

        // MediaMuxer writes `mdat` with a 64-bit size field. WebKit's media
        // stack refuses the whole file when it sees one (MEDIA_ERR_SRC_NOT_SUPPORTED),
        // while ffmpeg and ExoPlayer accept it, so desktop playback of every
        // envelope we produced failed. The body is far below the 32-bit limit
        // for anything we upload, so shrink the header and keep the sample data.
        val mdatBodySize = mdatBox.size - mdatBox.headerSize
        val mdatHeaderSize = if (
            mdatBox.headerSize == MP4_LARGE_BOX_HEADER_BYTES &&
            mdatBodySize + MP4_BOX_HEADER_BYTES <= Int.MAX_VALUE
        ) {
            MP4_BOX_HEADER_BYTES
        } else {
            mdatBox.headerSize
        }
        val mdatSize = mdatBodySize + mdatHeaderSize

        // Keep the original order, except that a moov which follows mdat moves
        // to just after the file type box.
        val order = boxes.filter { it !== moovBox }.toMutableList()
        val slot = if (moovBox.start < mdatBox.start) {
            boxes.indexOf(moovBox)
        } else {
            if (order.isNotEmpty() && order.first().type == "ftyp") 1 else 0
        }
        order.add(slot, moovBox)

        var offset = 0L
        var mdatStart = -1L
        for (box in order) {
            if (box === mdatBox) {
                mdatStart = offset
                break
            }
            offset += if (box === moovBox) moovSize else box.size
        }
        require(mdatStart >= 0) { "Invalid MP4: mdat is not addressable" }

        // Sample offsets point at the payload, so they follow both the box move
        // and any header shrink.
        val payloadShift =
            (mdatStart + mdatHeaderSize) - (mdatBox.start + mdatBox.headerSize)
        adjustMp4ChunkOffsets(moovBody, 0, moovBody.size, payloadShift, 0)

        for (box in order) {
            when {
                box === moovBox -> {
                    writeMp4BoxHeader(output, moovBox.type, moovBody.size.toLong())
                    output.write(moovBody)
                }
                box === mdatBox && mdatHeaderSize != mdatBox.headerSize -> {
                    writeMp4BoxHeader(output, mdatBox.type, mdatBodySize)
                    copyMp4Box(
                        container,
                        output,
                        box,
                        start = box.start + box.headerSize,
                        length = mdatBodySize,
                    )
                }
                else -> copyMp4Box(container, output, box)
            }
        }
    }

    private class Mp4Box(
        val type: String,
        val start: Long,
        val headerSize: Int,
        val size: Long,
    )

    private class Mp4ChildBox(
        val type: String,
        val start: Int,
        val headerSize: Int,
        val end: Int,
    ) {
        val bodyStart: Int get() = start + headerSize
        val size: Int get() = end - start
    }

    private fun readMp4TopLevelBoxes(container: RandomAccessFile, end: Long): List<Mp4Box> {
        val boxes = mutableListOf<Mp4Box>()
        var offset = 0L
        while (offset < end) {
            require(boxes.size < MP4_MAX_BOXES) { "Too many MP4 boxes" }
            require(end - offset >= MP4_BOX_HEADER_BYTES) { "Truncated MP4 box" }
            container.seek(offset)
            val header = ByteArray(MP4_BOX_HEADER_BYTES)
            container.readFully(header)
            val compact = readUnsignedIntBigEndian(header, 0)
            val size: Long
            val headerSize: Int
            when (compact) {
                1L -> {
                    require(end - offset >= MP4_LARGE_BOX_HEADER_BYTES) { "Truncated MP4 box" }
                    val extended = ByteArray(8)
                    container.readFully(extended)
                    size = readUnsignedLongBigEndian(extended, 0)
                    headerSize = MP4_LARGE_BOX_HEADER_BYTES
                }
                0L -> {
                    // Zero means "extends to the end of the enclosing box".
                    size = end - offset
                    headerSize = MP4_BOX_HEADER_BYTES
                }
                else -> {
                    size = compact
                    headerSize = MP4_BOX_HEADER_BYTES
                }
            }
            require(size >= headerSize && offset + size <= end) { "Invalid MP4 box size" }
            boxes += Mp4Box(String(header, 4, 4, StandardCharsets.ISO_8859_1), offset, headerSize, size)
            offset += size
        }
        return boxes
    }

    private fun readMp4BoxBody(container: RandomAccessFile, box: Mp4Box): ByteArray {
        val bodySize = box.size - box.headerSize
        require(bodySize <= MP4_MAX_MOOV_BYTES) { "MP4 moov box is too large" }
        val body = ByteArray(bodySize.toInt())
        container.seek(box.start + box.headerSize)
        container.readFully(body)
        return body
    }

    private fun readMp4ChildBoxes(bytes: ByteArray, start: Int, end: Int): List<Mp4ChildBox> {
        val boxes = mutableListOf<Mp4ChildBox>()
        var offset = start
        while (offset < end) {
            require(boxes.size < MP4_MAX_BOXES) { "Too many MP4 boxes" }
            require(end - offset >= MP4_BOX_HEADER_BYTES) { "Truncated MP4 box" }
            val compact = readUnsignedIntBigEndian(bytes, offset)
            val size: Long
            val headerSize: Int
            when (compact) {
                1L -> {
                    require(end - offset >= MP4_LARGE_BOX_HEADER_BYTES) { "Truncated MP4 box" }
                    size = readUnsignedLongBigEndian(bytes, offset + MP4_BOX_HEADER_BYTES)
                    headerSize = MP4_LARGE_BOX_HEADER_BYTES
                }
                0L -> {
                    size = (end - offset).toLong()
                    headerSize = MP4_BOX_HEADER_BYTES
                }
                else -> {
                    size = compact
                    headerSize = MP4_BOX_HEADER_BYTES
                }
            }
            require(size >= headerSize && offset + size <= end) { "Invalid MP4 box size" }
            boxes += Mp4ChildBox(
                String(bytes, offset + 4, 4, StandardCharsets.ISO_8859_1),
                offset,
                headerSize,
                (offset + size).toInt(),
            )
            offset += size.toInt()
        }
        return boxes
    }

    /** Drops metadata boxes at every level the relay would walk into. */
    private fun filterMp4Metadata(bytes: ByteArray, depth: Int): ByteArray {
        require(depth <= MP4_MAX_BOX_DEPTH) { "Too many nested MP4 boxes" }
        val output = ByteArrayOutputStream(bytes.size)
        for (box in readMp4ChildBoxes(bytes, 0, bytes.size)) {
            when {
                box.type in mp4MetadataBoxTypes -> Unit
                box.type in mp4ContainerBoxTypes -> {
                    val body = filterMp4Metadata(bytes.copyOfRange(box.bodyStart, box.end), depth + 1)
                    writeMp4BoxHeader(output, box.type, body.size.toLong())
                    output.write(body)
                }
                else -> output.write(bytes, box.start, box.size)
            }
        }
        return output.toByteArray()
    }

    private fun adjustMp4ChunkOffsets(
        bytes: ByteArray,
        start: Int,
        end: Int,
        delta: Long,
        depth: Int,
    ) {
        require(depth <= MP4_MAX_BOX_DEPTH) { "Too many nested MP4 boxes" }
        for (box in readMp4ChildBoxes(bytes, start, end)) {
            when (box.type) {
                "stco" -> adjustMp4OffsetTable(bytes, box, 4, delta)
                "co64" -> adjustMp4OffsetTable(bytes, box, 8, delta)
                in mp4ContainerBoxTypes ->
                    adjustMp4ChunkOffsets(bytes, box.bodyStart, box.end, delta, depth + 1)
            }
        }
    }

    private fun adjustMp4OffsetTable(bytes: ByteArray, box: Mp4ChildBox, entrySize: Int, delta: Long) {
        require(box.size >= box.headerSize + 8) { "Truncated MP4 chunk offset table" }
        val count = readUnsignedIntBigEndian(bytes, box.bodyStart + 4)
        val tableStart = box.bodyStart + 8
        require(count <= (box.end - tableStart) / entrySize) { "Truncated MP4 chunk offset table" }

        var position = tableStart
        var remaining = count
        while (remaining > 0) {
            if (entrySize == 4) {
                val adjusted = readUnsignedIntBigEndian(bytes, position) + delta
                require(adjusted in 0..0xFFFFFFFFL) { "MP4 chunk offset out of range" }
                writeUnsignedIntBigEndian(bytes, position, adjusted)
            } else {
                val adjusted = readUnsignedLongBigEndian(bytes, position) + delta
                require(adjusted >= 0) { "MP4 chunk offset out of range" }
                writeUnsignedLongBigEndian(bytes, position, adjusted)
            }
            position += entrySize
            remaining -= 1
        }
    }

    private fun writeMp4BoxHeader(output: OutputStream, type: String, bodySize: Long) {
        val size = bodySize + MP4_BOX_HEADER_BYTES
        require(size <= 0xFFFFFFFFL) { "MP4 box is too large" }
        val header = ByteArray(MP4_BOX_HEADER_BYTES)
        writeUnsignedIntBigEndian(header, 0, size)
        type.toByteArray(StandardCharsets.ISO_8859_1).copyInto(header, 4)
        output.write(header)
    }

    private fun copyMp4Box(
        container: RandomAccessFile,
        output: OutputStream,
        box: Mp4Box,
        start: Long = box.start,
        length: Long = box.size,
    ) {
        container.seek(start)
        val buffer = ByteArray(MP4_COPY_BUFFER_BYTES)
        var remaining = length
        while (remaining > 0) {
            val count = minOf(buffer.size.toLong(), remaining).toInt()
            container.readFully(buffer, 0, count)
            output.write(buffer, 0, count)
            remaining -= count
        }
    }

    private fun readUnsignedLongBigEndian(bytes: ByteArray, offset: Int): Long {
        require(bytes.size - offset >= 8) { "Truncated eight-byte integer" }
        var value = 0L
        for (index in 0 until 8) {
            value = (value shl 8) or (bytes[offset + index].toLong() and 0xFF)
        }
        return value
    }

    private fun writeUnsignedIntBigEndian(bytes: ByteArray, offset: Int, value: Long) {
        require(bytes.size - offset >= 4) { "Truncated four-byte integer" }
        for (index in 0 until 4) {
            bytes[offset + index] = (value ushr (24 - index * 8)).toByte()
        }
    }

    private fun writeUnsignedLongBigEndian(bytes: ByteArray, offset: Int, value: Long) {
        require(bytes.size - offset >= 8) { "Truncated eight-byte integer" }
        for (index in 0 until 8) {
            bytes[offset + index] = (value ushr (56 - index * 8)).toByte()
        }
    }

    private fun shouldKeepJpegSegment(
        marker: Int,
        bytes: ByteArray,
        payloadStart: Int,
        payloadEnd: Int,
    ): Boolean {
        val payloadLength = payloadEnd - payloadStart
        return when (marker) {
            JPEG_APP0 -> {
                if (payloadLength < 14 || !bytes.matchesAscii(payloadStart, "JFIF\u0000")) {
                    false
                } else {
                    val thumbnailWidth = bytes[payloadStart + 12].toInt() and 0xFF
                    val thumbnailHeight = bytes[payloadStart + 13].toInt() and 0xFF
                    payloadLength == 14 + 3 * thumbnailWidth * thumbnailHeight
                }
            }
            JPEG_APP14 -> payloadLength == 12 && bytes.matchesAscii(payloadStart, "Adobe")
            in JPEG_FORBIDDEN_APP_MARKERS, JPEG_APP15, JPEG_COMMENT -> false
            else -> true
        }
    }

    private fun readUnsignedShortBigEndian(bytes: ByteArray, offset: Int): Int {
        require(bytes.size - offset >= 2) { "Truncated two-byte integer" }
        return (bytes[offset].toInt() and 0xFF shl 8) or (bytes[offset + 1].toInt() and 0xFF)
    }

    private fun readUnsignedIntBigEndian(bytes: ByteArray, offset: Int): Long {
        require(bytes.size - offset >= 4) { "Truncated four-byte integer" }
        return (bytes[offset].toLong() and 0xFF shl 24) or
            (bytes[offset + 1].toLong() and 0xFF shl 16) or
            (bytes[offset + 2].toLong() and 0xFF shl 8) or
            (bytes[offset + 3].toLong() and 0xFF)
    }

    private fun ByteArray.matchesAscii(offset: Int, value: String): Boolean {
        val expected = value.toByteArray(StandardCharsets.US_ASCII)
        return size - offset >= expected.size && expected.indices.all { this[offset + it] == expected[it] }
    }

    private fun ByteArray.indexOf(value: Byte, startIndex: Int): Int {
        for (index in startIndex until size) {
            if (this[index] == value) return index
        }
        return -1
    }

    private const val PNG_CHUNK_OVERHEAD = 12
    private const val JPEG_SOI = 0xD8
    private const val JPEG_EOI = 0xD9
    private const val JPEG_SOS = 0xDA
    private const val JPEG_TEMP = 0x01
    private const val JPEG_APP0 = 0xE0
    private const val JPEG_APP14 = 0xEE
    private const val JPEG_APP15 = 0xEF
    private const val JPEG_COMMENT = 0xFE
    private val JPEG_RESTART_MARKERS = 0xD0..0xD7
    private val JPEG_FORBIDDEN_APP_MARKERS = 0xE1..0xED
    private const val MP4_BOX_HEADER_BYTES = 8
    private const val MP4_LARGE_BOX_HEADER_BYTES = 16
    private const val MP4_MAX_BOXES = 100_000
    private const val MP4_MAX_BOX_DEPTH = 32
    private const val MP4_MAX_MOOV_BYTES = 16L * 1024 * 1024
    private const val MP4_COPY_BUFFER_BYTES = 64 * 1024

    /** Private metadata channels the relay rejects in a canonical MP4. */
    private val mp4MetadataBoxTypes = setOf(
        "meta", "ilst", "keys", "data", "uuid", "xml ", "bxml", "loci", "\u00a9xyz", "name", "chap",
        // Allowed by the relay only in ffmpeg's exact empty form, which our muxer never writes.
        "udta",
    )

    /** Boxes the relay descends into; metadata can be nested at these depths. */
    private val mp4ContainerBoxTypes = setOf(
        "moov", "trak", "mdia", "minf", "stbl", "edts", "dinf", "sinf", "schi",
    )
}
