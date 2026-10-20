package xyz.block.buzz.mobile

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMuxer
import java.io.File
import java.nio.ByteBuffer
import java.util.UUID

internal object AndroidVoiceNotePackager {
    private const val videoMimeType = "video/avc"
    private const val videoWidth = 16
    private const val videoHeight = 16
    private const val videoBitRate = 8_000
    private const val videoFrameRate = 1
    private const val dequeueTimeoutUs = 10_000L
    private const val encoderTimeoutNs = 30_000_000_000L
    private const val copyBufferSize = 1024 * 1024

    fun packageForUpload(
        sourcePath: String,
        cacheDirectory: File,
    ): String {
        val source = File(sourcePath)
        require(source.isFile) { "The recording could not be found." }

        val audio = findAudioTrack(sourcePath)
        require(audio.durationUs > 0) { "The recording has no playable audio." }
        require(audio.mimeType == MediaFormat.MIMETYPE_AUDIO_AAC) {
            "The recording is not AAC audio."
        }

        val videoFile = File(cacheDirectory, "${UUID.randomUUID()}-voice-note-video.mp4")
        val outputFile = File(cacheDirectory, "${UUID.randomUUID()}.mp4")
        try {
            writeBlackVideoTrack(videoFile, audio.durationUs)
            muxEnvelope(
                audioPath = sourcePath,
                audioTrackIndex = audio.index,
                videoPath = videoFile.absolutePath,
                outputPath = outputFile.absolutePath,
            )
            return outputFile.absolutePath
        } catch (error: Exception) {
            outputFile.delete()
            throw error
        } finally {
            videoFile.delete()
        }
    }

    private data class AudioTrack(
        val index: Int,
        val durationUs: Long,
        val mimeType: String,
    )

    private fun findAudioTrack(sourcePath: String): AudioTrack {
        val extractor = MediaExtractor()
        try {
            extractor.setDataSource(sourcePath)
            for (index in 0 until extractor.trackCount) {
                val format = extractor.getTrackFormat(index)
                val mimeType = format.getString(MediaFormat.KEY_MIME) ?: continue
                if (!mimeType.startsWith("audio/")) continue
                val durationUs = if (format.containsKey(MediaFormat.KEY_DURATION)) {
                    format.getLong(MediaFormat.KEY_DURATION)
                } else {
                    0L
                }
                return AudioTrack(index, durationUs, mimeType)
            }
            throw IllegalArgumentException("The recording does not contain an audio track.")
        } finally {
            extractor.release()
        }
    }

    private fun writeBlackVideoTrack(
        outputFile: File,
        durationUs: Long,
    ) {
        val codec = MediaCodec.createEncoderByType(videoMimeType)
        var muxer: MediaMuxer? = null
        try {
            val colorFormat = codec.codecInfo
                .getCapabilitiesForType(videoMimeType)
                .colorFormats
                .firstOrNull {
                    it == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible ||
                        it == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar ||
                        it == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420SemiPlanar
                }
                ?: throw IllegalStateException("This device cannot create the voice note envelope.")
            val format = MediaFormat.createVideoFormat(
                videoMimeType,
                videoWidth,
                videoHeight,
            ).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, colorFormat)
                setInteger(MediaFormat.KEY_BIT_RATE, videoBitRate)
                setInteger(MediaFormat.KEY_FRAME_RATE, videoFrameRate)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
            }
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            codec.start()

            muxer = MediaMuxer(
                outputFile.absolutePath,
                MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4,
            )
            val frame = blackYuv420Frame()
            val frameTimesUs = longArrayOf(0L, durationUs)
            val bufferInfo = MediaCodec.BufferInfo()
            var nextFrame = 0
            var inputEnded = false
            var outputEnded = false
            var outputTrack = -1
            var muxerStarted = false
            val encoderDeadlineNs = System.nanoTime() + encoderTimeoutNs

            while (!outputEnded) {
                check(System.nanoTime() - encoderDeadlineNs < 0) {
                    "Timed out creating the voice note envelope."
                }
                if (!inputEnded) {
                    val inputIndex = codec.dequeueInputBuffer(dequeueTimeoutUs)
                    if (inputIndex >= 0) {
                        val inputBuffer = codec.getInputBuffer(inputIndex)
                            ?: throw IllegalStateException("Unable to create the video envelope.")
                        inputBuffer.clear()
                        if (nextFrame < frameTimesUs.size) {
                            inputBuffer.put(frame)
                            codec.queueInputBuffer(
                                inputIndex,
                                0,
                                frame.size,
                                frameTimesUs[nextFrame],
                                0,
                            )
                            nextFrame += 1
                        } else {
                            codec.queueInputBuffer(
                                inputIndex,
                                0,
                                0,
                                durationUs + 1,
                                MediaCodec.BUFFER_FLAG_END_OF_STREAM,
                            )
                            inputEnded = true
                        }
                    }
                }

                when (val outputIndex = codec.dequeueOutputBuffer(bufferInfo, dequeueTimeoutUs)) {
                    MediaCodec.INFO_TRY_AGAIN_LATER -> Unit
                    MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        check(!muxerStarted) { "The video encoder format changed twice." }
                        outputTrack = muxer.addTrack(codec.outputFormat)
                        muxer.start()
                        muxerStarted = true
                    }
                    else -> if (outputIndex >= 0) {
                        val outputBuffer = codec.getOutputBuffer(outputIndex)
                            ?: throw IllegalStateException("Unable to read the video envelope.")
                        if (bufferInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                            bufferInfo.size = 0
                        }
                        if (bufferInfo.size > 0) {
                            check(muxerStarted && outputTrack >= 0) {
                                "The video envelope has no output format."
                            }
                            outputBuffer.position(bufferInfo.offset)
                            outputBuffer.limit(bufferInfo.offset + bufferInfo.size)
                            muxer.writeSampleData(outputTrack, outputBuffer, bufferInfo)
                        }
                        outputEnded =
                            bufferInfo.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                        codec.releaseOutputBuffer(outputIndex, false)
                    }
                }
            }

            if (muxerStarted) muxer.stop()
        } finally {
            try {
                codec.stop()
            } catch (_: Exception) {
                // Best-effort encoder cleanup after a packaging failure.
            }
            codec.release()
            try {
                muxer?.release()
            } catch (_: Exception) {
                // Best-effort muxer cleanup after a packaging failure.
            }
        }
    }

    private fun blackYuv420Frame(): ByteArray {
        val yPlaneSize = videoWidth * videoHeight
        return ByteArray(yPlaneSize * 3 / 2) { index ->
            if (index < yPlaneSize) 16 else 128.toByte()
        }
    }

    private fun muxEnvelope(
        audioPath: String,
        audioTrackIndex: Int,
        videoPath: String,
        outputPath: String,
    ) {
        val audioExtractor = MediaExtractor()
        val videoExtractor = MediaExtractor()
        var muxer: MediaMuxer? = null
        try {
            audioExtractor.setDataSource(audioPath)
            audioExtractor.selectTrack(audioTrackIndex)
            videoExtractor.setDataSource(videoPath)
            val videoTrackIndex = (0 until videoExtractor.trackCount).firstOrNull { index ->
                videoExtractor.getTrackFormat(index)
                    .getString(MediaFormat.KEY_MIME)
                    ?.startsWith("video/") == true
            } ?: throw IllegalStateException("The video envelope has no video track.")
            videoExtractor.selectTrack(videoTrackIndex)

            muxer = MediaMuxer(outputPath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
            val destinationVideoTrack = muxer.addTrack(
                videoExtractor.getTrackFormat(videoTrackIndex),
            )
            val destinationAudioTrack = muxer.addTrack(
                audioExtractor.getTrackFormat(audioTrackIndex),
            )
            muxer.start()
            copyTrack(videoExtractor, muxer, destinationVideoTrack)
            copyTrack(audioExtractor, muxer, destinationAudioTrack)
            muxer.stop()
        } finally {
            audioExtractor.release()
            videoExtractor.release()
            try {
                muxer?.release()
            } catch (_: Exception) {
                // Best-effort muxer cleanup after a packaging failure.
            }
        }
    }

    private fun copyTrack(
        extractor: MediaExtractor,
        muxer: MediaMuxer,
        destinationTrack: Int,
    ) {
        val buffer = ByteBuffer.allocate(copyBufferSize)
        val bufferInfo = MediaCodec.BufferInfo()
        while (true) {
            buffer.clear()
            val sampleSize = extractor.readSampleData(buffer, 0)
            if (sampleSize < 0) return
            bufferInfo.offset = 0
            bufferInfo.size = sampleSize
            bufferInfo.presentationTimeUs = extractor.sampleTime
            bufferInfo.flags = extractor.sampleFlags
            muxer.writeSampleData(destinationTrack, buffer, bufferInfo)
            extractor.advance()
        }
    }
}
