package xyz.block.buzz.mobile

import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class DeviceTranscriptionWaveTest {
    private fun wave(rate: Int = 16000, channels: Short = 1): ByteArray {
        val body = ByteArrayOutputStream()
        body.write("WAVEJUNK".toByteArray())
        body.write(byteArrayOf(1, 0, 0, 0, 42, 0)) // odd chunk length and pad
        body.write("fmt ".toByteArray())
        body.write(ByteBuffer.allocate(20).order(ByteOrder.LITTLE_ENDIAN)
            .putInt(16).putShort(1).putShort(channels).putInt(rate)
            .putInt(rate * 2).putShort(2).putShort(16).array())
        body.write("data".toByteArray())
        body.write(byteArrayOf(4, 0, 0, 0, 1, 2, 3, 4))
        return "RIFF".toByteArray() + ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
            .putInt(body.size()).array() + body.toByteArray()
    }
    private fun withFile(bytes: ByteArray, action: (File) -> Unit) {
        val file = File.createTempFile("transcription", ".wav")
        try { file.writeBytes(bytes); action(file) } finally { file.delete() }
    }
    @Test fun skipsMetadataChunksInsteadOfAssuming44ByteHeader() {
        withFile(wave()) { assertArrayEquals(byteArrayOf(1, 2, 3, 4), readTranscriptionWave(it)) }
    }
    @Test fun rejectsWrongFormatAndTruncatedAudio() {
        for (bytes in listOf(wave(rate = 44100), wave(channels = 2), wave().dropLast(1).toByteArray())) {
            withFile(bytes) { file -> assertThrows(IllegalArgumentException::class.java) { readTranscriptionWave(file) } }
        }
    }
}
