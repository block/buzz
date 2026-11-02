package xyz.block.buzz.mobile

import java.io.ByteArrayInputStream
import java.io.IOException
import java.io.InputStream
import java.nio.charset.CharacterCodingException
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class IdentityFileReaderTest {
    @Test fun `reads exact limit and closes document`() {
        val input = CountingInput(ByteArray(1024) { 'a'.code.toByte() })
        assertEquals("a".repeat(1024), IdentityFileReader.read(input))
        assertEquals(1024, input.consumed)
        assertTrue(input.closed)
    }

    @Test fun `oversized document reads only sentinel and closes`() {
        for (size in listOf(1025, 1000000)) {
            val input = CountingInput(ByteArray(size) { 'a'.code.toByte() })
            assertFailsWith<IllegalArgumentException> { IdentityFileReader.read(input) }
            assertEquals(1025, input.consumed)
            assertTrue(input.closed)
        }
    }

    @Test fun `short provider reads do not truncate document`() {
        val input = CountingInput("nsec-test\n".toByteArray(), chunk = 1)
        assertEquals("nsec-test\n", IdentityFileReader.read(input))
        assertTrue(input.closed)
    }

    @Test fun `zero-length provider reads still make bounded progress`() {
        var consumed = 0
        var closed = false
        var supplied: ByteArray? = null
        val input = object : InputStream() {
            override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
                supplied = buffer
                return 0
            }
            override fun read(): Int { consumed++; return 'a'.code }
            override fun close() { closed = true }
        }
        assertFailsWith<IllegalArgumentException> { IdentityFileReader.read(input) }
        assertEquals(1025, consumed)
        assertTrue(closed)
        assertTrue(supplied!!.all { it == 0.toByte() })
    }

    @Test fun `invalid UTF8 and read failures reject and close`() {
        val malformed = CountingInput(byteArrayOf(0xc3.toByte(), 0x28))
        assertFailsWith<CharacterCodingException> { IdentityFileReader.read(malformed) }
        assertTrue(malformed.closed)
        var closed = false
        val failing = object : InputStream() {
            override fun read(): Int = throw IOException("provider detail must not escape channel")
            override fun close() { closed = true }
        }
        assertFailsWith<IOException> { IdentityFileReader.read(failing) }
        assertTrue(closed)
    }

    private class CountingInput(bytes: ByteArray, private val chunk: Int = Int.MAX_VALUE) :
        ByteArrayInputStream(bytes) {
        var consumed = 0
        var closed = false
        override fun read(buffer: ByteArray, offset: Int, length: Int): Int =
            super.read(buffer, offset, minOf(length, chunk)).also { if (it > 0) consumed += it }
        override fun close() { closed = true; super.close() }
    }
}
