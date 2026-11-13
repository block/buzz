package xyz.block.buzz.mobile

import android.media.MediaCodecInfo.CodecCapabilities
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class AndroidVoiceNotePackagerSelectionTest {
    @Test
    fun skipsSurfaceOnlyAndWrongSizeEncoders() {
        val surface = AndroidVoiceNotePackager.EncoderCandidate(
            "surface", true, intArrayOf(CodecCapabilities.COLOR_FormatSurface),
        )
        for (color in listOf(
            CodecCapabilities.COLOR_FormatYUV420Flexible,
            CodecCapabilities.COLOR_FormatYUV420Planar,
            CodecCapabilities.COLOR_FormatYUV420SemiPlanar,
        )) {
            val wrongSize = AndroidVoiceNotePackager.EncoderCandidate("large", false, intArrayOf(color))
            val usable = AndroidVoiceNotePackager.EncoderCandidate("usable", true, intArrayOf(color))
            assertEquals("usable" to color, AndroidVoiceNotePackager.selectEncoder(sequenceOf(surface, wrongSize, usable)))
        }
        assertFailsWith<IllegalStateException> {
            AndroidVoiceNotePackager.selectEncoder(sequenceOf(surface))
        }
    }
}
