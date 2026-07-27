package xyz.block.buzz.mobile

import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class AndroidMediaSanitizerTest {
    @Test
    fun `scrubPng keeps canonical sRGB output unchanged`() {
        val fixture = fixtureBytes("bitmap-srgb.png")

        val sanitized = AndroidMediaSanitizer.scrubPng(fixture)

        assertContentEquals(fixture, sanitized)
        assertEquals(listOf("IHDR", "sRGB", "sBIT", "IDAT", "IEND"), pngChunkTypes(sanitized))
    }

    @Test
    fun `scrubPng removes Display P3 profile and trailing data`() {
        val fixture = fixtureBytes("bitmap-display-p3.png")
        val withTrailingData = fixture + "hidden location".encodeToByteArray()

        val sanitized = AndroidMediaSanitizer.scrubPng(withTrailingData)

        assertEquals(listOf("IHDR", "sBIT", "IDAT", "IEND"), pngChunkTypes(sanitized))
    }

    @Test
    fun `scrubJpeg removes Android ICC profile and trailing data`() {
        for (fixtureName in listOf("bitmap-srgb.jpg", "bitmap-display-p3.jpg")) {
            val fixture = fixtureBytes(fixtureName)
            val withTrailingData = fixture + "hidden location".encodeToByteArray()

            val sanitized = AndroidMediaSanitizer.scrubJpeg(withTrailingData)

            assertEquals(listOf(0xE0), jpegMetadataMarkers(sanitized), fixtureName)
        }
    }

    @Test
    fun `scrubbers fail closed for malformed containers`() {
        assertFailsWith<IllegalArgumentException> {
            AndroidMediaSanitizer.scrubPng(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47))
        }
        assertFailsWith<IllegalArgumentException> {
            AndroidMediaSanitizer.scrubJpeg(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte()))
        }
        assertFailsWith<IllegalArgumentException> {
            AndroidMediaSanitizer.scrubJpeg(
                byteArrayOf(
                    0xFF.toByte(),
                    0xD8.toByte(),
                    0xFF.toByte(),
                    0xD8.toByte(),
                    0x00,
                    0x02,
                    0xFF.toByte(),
                    0xD9.toByte(),
                ),
            )
        }
    }

    private fun fixtureBytes(name: String): ByteArray {
        return requireNotNull(javaClass.getResourceAsStream("/fixtures/android/$name")) {
            "Missing fixture: $name"
        }.use { it.readBytes() }
    }

    private fun pngChunkTypes(bytes: ByteArray): List<String> {
        val result = mutableListOf<String>()
        var offset = 8
        while (offset < bytes.size) {
            require(bytes.size - offset >= 12)
            val payloadLength = readUnsignedInt(bytes, offset)
            val type = bytes.decodeToString(offset + 4, offset + 8)
            result += type
            offset += payloadLength + 12
            if (type == "IEND") {
                assertEquals(bytes.size, offset)
                return result
            }
        }
        error("PNG is missing IEND")
    }

    private fun jpegMetadataMarkers(bytes: ByteArray): List<Int> {
        val result = mutableListOf<Int>()
        var offset = 2
        var inScan = false
        while (offset < bytes.size) {
            if (inScan && bytes[offset] != 0xFF.toByte()) {
                offset += 1
                continue
            }
            require(bytes[offset] == 0xFF.toByte())
            while (offset < bytes.size && bytes[offset] == 0xFF.toByte()) {
                offset += 1
            }
            require(offset < bytes.size)
            val marker = bytes[offset].toInt() and 0xFF
            offset += 1
            if (inScan && marker == 0x00) continue
            if (marker in 0xD0..0xD7 || marker == 0x01) continue
            if (marker == 0xD9) {
                assertEquals(bytes.size, offset)
                return result
            }
            val segmentLength = readUnsignedShort(bytes, offset)
            if (marker in 0xE0..0xEF || marker == 0xFE) result += marker
            offset += segmentLength
            inScan = marker == 0xDA
        }
        error("JPEG is missing EOI")
    }

    private fun readUnsignedShort(bytes: ByteArray, offset: Int): Int {
        return ((bytes[offset].toInt() and 0xFF) shl 8) or (bytes[offset + 1].toInt() and 0xFF)
    }

    private fun readUnsignedInt(bytes: ByteArray, offset: Int): Int {
        return ((bytes[offset].toInt() and 0xFF) shl 24) or
            ((bytes[offset + 1].toInt() and 0xFF) shl 16) or
            ((bytes[offset + 2].toInt() and 0xFF) shl 8) or
            (bytes[offset + 3].toInt() and 0xFF)
    }
}
