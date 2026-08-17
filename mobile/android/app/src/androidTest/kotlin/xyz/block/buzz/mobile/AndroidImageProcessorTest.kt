package xyz.block.buzz.mobile

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith
import kotlin.math.abs
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

@RunWith(AndroidJUnit4::class)
class AndroidImageProcessorTest {
    companion object {
        private val ALLOWED_PNG_CHUNKS = setOf(
            "IHDR", "PLTE", "IDAT", "IEND", "cHRM", "gAMA", "sBIT", "sRGB", "bKGD", "hIST", "tRNS", "sPLT",
            "acTL", "fcTL", "fdAT",
        )
    }

    @Test
    fun supportedApiCanProcessImagesWithoutNewerPlatformApis() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) return

        val sourceBytes = fixtureBytes("bitmap-srgb.png")
        val processed = assertNotNull(AndroidImageProcessor.decodeSrgbBitmap(sourceBytes))
        assertEquals(Bitmap.Config.ARGB_8888, processed.config)

        val scrubbed = assertNotNull(
            AndroidImageProcessor.encodeAndScrub(processed, Bitmap.CompressFormat.PNG),
        )
        val chunkTypes = pngChunkTypes(scrubbed)
        assertEquals("IHDR", chunkTypes.first())
        assertEquals("IEND", chunkTypes.last())
        assertTrue(chunkTypes.all { it in ALLOWED_PNG_CHUNKS })
        assertPngPixelsPreserved(sourceBytes, scrubbed)
    }

    @Test
    fun transparentPngPixelsSurviveSrgbProcessing() {
        val sourceBytes = fixtureBytes("bitmap-srgb.png")
        val source = assertNotNull(BitmapFactory.decodeByteArray(sourceBytes, 0, sourceBytes.size))
        val processed = assertNotNull(AndroidImageProcessor.decodeSrgbBitmap(sourceBytes))
        assertTrue(processed.hasAlpha())

        val scrubbed = assertNotNull(
            AndroidImageProcessor.encodeAndScrub(processed, Bitmap.CompressFormat.PNG),
        )
        assertPngPixelsPreserved(sourceBytes, scrubbed)
        assertEquals(128, Color.alpha(source.getPixel(0, 1)))
        assertEquals(64, Color.alpha(source.getPixel(1, 1)))
        assertEquals(0, Color.alpha(source.getPixel(2, 1)))
    }

    @Test
    fun displayP3InputIsConvertedToSrgbBeforeEncodedMetadataIsScrubbed() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return

        val sourceBytes = fixtureBytes("bitmap-display-p3.png")
        val source = assertNotNull(BitmapFactory.decodeByteArray(sourceBytes, 0, sourceBytes.size))
        assertEquals(android.graphics.ColorSpace.get(android.graphics.ColorSpace.Named.DISPLAY_P3), source.colorSpace)
        val expectedSrgbPixel = source.getColor(0, 0).convert(
            android.graphics.ColorSpace.get(android.graphics.ColorSpace.Named.SRGB),
        )

        val srgbBitmap = assertNotNull(AndroidImageProcessor.decodeSrgbBitmap(sourceBytes))
        assertEquals(android.graphics.ColorSpace.get(android.graphics.ColorSpace.Named.SRGB), srgbBitmap.colorSpace)
        assertEquals(Bitmap.Config.ARGB_8888, srgbBitmap.config)
        assertColorClose(expectedSrgbPixel, srgbBitmap.getColor(0, 0), tolerance = 1.0f / 255.0f)

        for ((format, tolerance) in listOf(
            Bitmap.CompressFormat.PNG to (1.0f / 255.0f),
            Bitmap.CompressFormat.JPEG to (2.0f / 255.0f),
        )) {
            val scrubbed = assertNotNull(AndroidImageProcessor.encodeAndScrub(srgbBitmap, format))
            when (format) {
                Bitmap.CompressFormat.PNG -> {
                    assertEquals(listOf("IHDR", "sRGB", "sBIT", "IDAT", "IEND"), pngChunkTypes(scrubbed))
                }
                Bitmap.CompressFormat.JPEG -> {
                    assertEquals(listOf(0xE0), jpegMetadataMarkers(scrubbed))
                }
                else -> error("Unexpected format: $format")
            }

            val decodedOutput = assertNotNull(BitmapFactory.decodeByteArray(scrubbed, 0, scrubbed.size))
            assertEquals(
                android.graphics.ColorSpace.get(android.graphics.ColorSpace.Named.SRGB),
                decodedOutput.colorSpace,
            )
            assertColorClose(expectedSrgbPixel, decodedOutput.getColor(0, 0), tolerance)
        }
    }

    private fun assertPngPixelsPreserved(expectedBytes: ByteArray, actualBytes: ByteArray) {
        val expected = assertNotNull(BitmapFactory.decodeByteArray(expectedBytes, 0, expectedBytes.size))
        val actual = assertNotNull(BitmapFactory.decodeByteArray(actualBytes, 0, actualBytes.size))
        assertEquals(expected.width, actual.width)
        assertEquals(expected.height, actual.height)
        for (y in 0 until expected.height) {
            for (x in 0 until expected.width) {
                assertEquals(
                    expected.getPixel(x, y),
                    actual.getPixel(x, y),
                    "pixel ($x, $y)",
                )
            }
        }
    }

    private fun fixtureBytes(name: String): ByteArray {
        return requireNotNull(javaClass.getResourceAsStream("/fixtures/android/$name")) {
            "Missing fixture: $name"
        }.use { it.readBytes() }
    }

    private fun assertColorClose(expected: Color, actual: Color, tolerance: Float) {
        val expectedComponents = expected.components
        val actualComponents = actual.convert(expected.colorSpace).components
        for (index in expectedComponents.indices) {
            assertTrue(
                abs(expectedComponents[index] - actualComponents[index]) <= tolerance,
                "component $index: expected ${expectedComponents[index]}, got ${actualComponents[index]}",
            )
        }
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
