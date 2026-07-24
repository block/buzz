package xyz.block.buzz.mobile

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.ColorSpace
import android.graphics.ImageDecoder
import android.media.MediaExtractor
import android.media.MediaMuxer
import android.os.Build
import androidx.annotation.RequiresApi
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer
import java.util.UUID

internal object AndroidImageProcessor {
    fun decodeSrgbBitmap(bytes: ByteArray): Bitmap? {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            decodeSrgbBitmapWithColorManagement(bytes)
        } else {
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
        }
    }

    @RequiresApi(Build.VERSION_CODES.O)
    private fun decodeSrgbBitmapWithColorManagement(bytes: ByteArray): Bitmap? {
        val decoded = decodeColorManagedBitmap(bytes) ?: return null
        val srgb = ColorSpace.get(ColorSpace.Named.SRGB)
        if (decoded.config == Bitmap.Config.ARGB_8888 && decoded.colorSpace == srgb) return decoded

        val srgbBitmap = Bitmap.createBitmap(
            decoded.width,
            decoded.height,
            Bitmap.Config.ARGB_8888,
            decoded.hasAlpha(),
            srgb,
        )
        Canvas(srgbBitmap).drawBitmap(decoded, 0f, 0f, null)
        return srgbBitmap
    }

    @RequiresApi(Build.VERSION_CODES.O)
    private fun decodeColorManagedBitmap(bytes: ByteArray): Bitmap? {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            runCatching {
                val source = ImageDecoder.createSource(ByteBuffer.wrap(bytes))
                ImageDecoder.decodeBitmap(source) { decoder, _, _ ->
                    decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                    decoder.setTargetColorSpace(ColorSpace.get(ColorSpace.Named.SRGB))
                }
            }.getOrNull()?.let { return it }
        }

        val options = BitmapFactory.Options().apply {
            inPreferredColorSpace = ColorSpace.get(ColorSpace.Named.SRGB)
        }
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
    }

    fun encodeAndScrub(
        bitmap: Bitmap,
        format: Bitmap.CompressFormat,
    ): ByteArray? {
        val output = ByteArrayOutputStream()
        if (!bitmap.compress(format, 100, output)) return null

        return when (format) {
            Bitmap.CompressFormat.PNG -> AndroidMediaSanitizer.scrubPng(output.toByteArray())
            Bitmap.CompressFormat.JPEG -> AndroidMediaSanitizer.scrubJpeg(output.toByteArray())
            else -> error("Unsupported upload image format: $format")
        }
    }
}

class MainActivity : FlutterActivity() {
    private var mediaUploadChannel: MethodChannel? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        mediaUploadChannel = MethodChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            MEDIA_UPLOAD_CHANNEL,
        ).also { channel ->
            channel.setMethodCallHandler { call, result ->
                when (call.method) {
                    SANITIZE_IMAGE_FOR_UPLOAD_METHOD -> {
                        handleSanitizeImageForUpload(call.arguments, result)
                    }
                    TRANSCODE_IMAGE_TO_JPEG_METHOD -> {
                        handleTranscodeImageToJpeg(call.arguments, result)
                    }
                    TRANSCODE_VIDEO_TO_MP4_METHOD -> {
                        handleTranscodeVideoToMp4(call.arguments, result)
                    }
                    else -> result.notImplemented()
                }
            }
        }
    }

    private fun handleSanitizeImageForUpload(
        arguments: Any?,
        result: MethodChannel.Result,
    ) {
        val payload = arguments as? Map<*, *> ?: run {
            invalidArguments(result, "Expected image bytes and mime type.")
            return
        }
        val bytes = payload["bytes"] as? ByteArray ?: run {
            invalidArguments(result, "Expected raw image bytes.")
            return
        }
        val mimeType = payload["mimeType"] as? String ?: run {
            invalidArguments(result, "Expected image mime type.")
            return
        }

        val format = sanitizeCompressFormatFor(mimeType)
        if (format == null) {
            result.error(
                "sanitize_failed",
                "Unable to sanitize picked image.",
                mimeType,
            )
            return
        }

        transformImageBytes(
            bytes = bytes,
            result = result,
            format = format,
            errorCode = "sanitize_failed",
            encodeFailureMessage = "Unable to sanitize picked image.",
            errorDetails = mimeType,
        )
    }

    private fun handleTranscodeImageToJpeg(
        arguments: Any?,
        result: MethodChannel.Result,
    ) {
        val bytes = arguments as? ByteArray ?: run {
            invalidArguments(result, "Expected raw image bytes.")
            return
        }

        transformImageBytes(
            bytes = bytes,
            result = result,
            format = Bitmap.CompressFormat.JPEG,
            errorCode = "transcode_failed",
            encodeFailureMessage = "Unable to convert picked image to JPEG.",
        )
    }

    private fun sanitizeCompressFormatFor(
        mimeType: String,
    ): Bitmap.CompressFormat? {
        return when (mimeType) {
            "image/jpeg" -> Bitmap.CompressFormat.JPEG
            "image/png", "image/webp" -> Bitmap.CompressFormat.PNG
            else -> null
        }
    }

    private fun transformImageBytes(
        bytes: ByteArray,
        result: MethodChannel.Result,
        format: Bitmap.CompressFormat,
        errorCode: String,
        encodeFailureMessage: String,
        errorDetails: Any? = null,
    ) {
        val bitmap = AndroidImageProcessor.decodeSrgbBitmap(bytes) ?: run {
            result.error(
                errorCode,
                "Unable to decode picked image.",
                null,
            )
            return
        }

        val transformedBytes = try {
            AndroidImageProcessor.encodeAndScrub(bitmap, format)
        } catch (_: IllegalArgumentException) {
            null
        } ?: run {
            result.error(
                errorCode,
                encodeFailureMessage,
                errorDetails,
            )
            return
        }

        result.success(transformedBytes)
    }

    private fun handleTranscodeVideoToMp4(
        arguments: Any?,
        result: MethodChannel.Result,
    ) {
        val sourcePath = arguments as? String ?: run {
            invalidArguments(result, "Expected source file path as String.")
            return
        }

        Thread {
            val outputFile = File(cacheDir, "${UUID.randomUUID()}.mp4")
            var muxer: MediaMuxer? = null
            val extractor = MediaExtractor()
            try {
                extractor.setDataSource(sourcePath)
                muxer = MediaMuxer(outputFile.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)

                val trackIndices = mutableMapOf<Int, Int>()
                var copiedVideo = false
                var copiedAudio = false
                for (i in 0 until extractor.trackCount) {
                    val format = extractor.getTrackFormat(i)
                    val mime = format.getString(android.media.MediaFormat.KEY_MIME) ?: continue
                    val isVideo = mime.startsWith("video/")
                    val isAudio = mime.startsWith("audio/")
                    if ((!isVideo && !isAudio) || (isVideo && copiedVideo) || (isAudio && copiedAudio)) {
                        continue
                    }
                    val newIndex = muxer.addTrack(format)
                    trackIndices[i] = newIndex
                    extractor.selectTrack(i)
                    copiedVideo = copiedVideo || isVideo
                    copiedAudio = copiedAudio || isAudio
                }

                muxer.start()
                val buffer = ByteBuffer.allocate(1024 * 1024) // 1MB buffer
                val bufferInfo = android.media.MediaCodec.BufferInfo()

                while (true) {
                    val sampleSize = extractor.readSampleData(buffer, 0)
                    if (sampleSize < 0) break
                    val muxerTrack = trackIndices[extractor.sampleTrackIndex]
                    if (muxerTrack == null) {
                        extractor.advance()
                        continue
                    }
                    bufferInfo.offset = 0
                    bufferInfo.size = sampleSize
                    bufferInfo.presentationTimeUs = extractor.sampleTime
                    bufferInfo.flags = extractor.sampleFlags
                    muxer.writeSampleData(muxerTrack, buffer, bufferInfo)
                    extractor.advance()
                }

                muxer.stop()
                result.success(outputFile.absolutePath)
            } catch (e: Exception) {
                outputFile.delete()
                result.error(
                    "transcode_failed",
                    e.message ?: "Video transcoding failed.",
                    null,
                )
            } finally {
                try { muxer?.release() } catch (_: Exception) {}
                extractor.release()
            }
        }.start()
    }

    private fun invalidArguments(
        result: MethodChannel.Result,
        message: String,
    ) {
        result.error("invalid_arguments", message, null)
    }

    companion object {
        private const val MEDIA_UPLOAD_CHANNEL = "buzz/media_upload"
        private const val SANITIZE_IMAGE_FOR_UPLOAD_METHOD = "sanitizeImageForUpload"
        private const val TRANSCODE_IMAGE_TO_JPEG_METHOD = "transcodeImageToJpeg"
        private const val TRANSCODE_VIDEO_TO_MP4_METHOD = "transcodeVideoToMp4"
    }
}
