package xyz.block.buzz.mobile

import java.io.InputStream
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction

/** Reads at most one KiB plus a sentinel byte, without a plaintext cache file. */
internal object IdentityFileReader {
    fun read(input: InputStream): String = input.use {
        val buffer = ByteArray(1025)
        try {
            var length = 0
            while (length < buffer.size) {
                val count = input.read(buffer, length, buffer.size - length)
                if (count < 0) break
                if (count == 0) {
                    // Defensive progress for providers that return zero for a
                    // nonempty read; this still never reads past the sentinel.
                    val byte = input.read()
                    if (byte < 0) break
                    buffer[length++] = byte.toByte()
                } else {
                    length += count
                }
            }
            require(length <= 1024) { "Identity document exceeds size limit" }
            Charsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(buffer, 0, length)).toString()
        } finally {
            buffer.fill(0)
        }
    }
}
