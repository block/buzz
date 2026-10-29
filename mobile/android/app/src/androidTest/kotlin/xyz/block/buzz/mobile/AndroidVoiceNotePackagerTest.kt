package xyz.block.buzz.mobile

import android.media.MediaExtractor
import android.media.MediaFormat
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@RunWith(AndroidJUnit4::class)
class AndroidVoiceNotePackagerTest {
    @Test
    fun packagesAacWithPlayableAudioAndVideoTracks() {
        val directory = File.createTempFile("voice-package", "test").apply {
            delete()
            mkdirs()
        }
        try {
            val input = File(directory, "voice.m4a")
            input.writeBytes(requireNotNull(javaClass.classLoader!!.getResourceAsStream(
                "fixtures/android/voice-tone.m4a",
            )).use { it.readBytes() })
            val output = AndroidVoiceNotePackager.packageForUpload(input.path, directory)
            val extractor = MediaExtractor()
            try {
                extractor.setDataSource(output)
                assertEquals(2, extractor.trackCount)
                val types = mutableSetOf<String>()
                for (index in 0 until extractor.trackCount) {
                    val format = extractor.getTrackFormat(index)
                    types.add(requireNotNull(format.getString(MediaFormat.KEY_MIME)))
                    assertTrue(format.getLong(MediaFormat.KEY_DURATION) > 0)
                    extractor.selectTrack(index)
                    assertTrue(extractor.sampleSize > 0)
                    extractor.unselectTrack(index)
                }
                assertEquals(setOf("audio/mp4a-latm", "video/avc"), types)
            } finally {
                extractor.release()
            }
            assertTrue(input.isFile)
        } finally {
            directory.deleteRecursively()
        }
    }
}
