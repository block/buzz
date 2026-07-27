package xyz.block.buzz.mobile

import java.io.ByteArrayOutputStream
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
}
