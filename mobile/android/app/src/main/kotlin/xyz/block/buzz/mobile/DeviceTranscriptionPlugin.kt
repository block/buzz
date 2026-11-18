package xyz.block.buzz.mobile

import android.content.Context
import android.content.Intent
import android.media.AudioFormat
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.io.RandomAccessFile
import java.util.Locale

/** Transcribes local PCM audio through the explicitly on-device recognizer. */
class DeviceTranscriptionPlugin(private val context: Context, messenger: BinaryMessenger) {
    private val channel = MethodChannel(messenger, "buzz/transcription")
    private val handler = Handler(Looper.getMainLooper())
    private var recognizer: SpeechRecognizer? = null
    private var owner: String? = null
    private var reply: MethodChannel.Result? = null
    private var pipe: Array<ParcelFileDescriptor>? = null
    private val segments = mutableListOf<String>()
    private val timeout = Runnable { fail("Transcription timed out. Your recording is saved.") }

    init {
        channel.setMethodCallHandler { call, result ->
            val id = call.argument<String>("id")
            if (id == null) { result.error("arguments", "Invalid transcription request.", null) }
            else if (call.method == "cancel") {
                if (owner == id) fail("Transcription cancelled.")
                result.success(null)
            } else if (call.method != "prepare" && call.method != "transcribe") {
                result.notImplemented()
            } else if (Build.VERSION.SDK_INT < 33 || !SpeechRecognizer.isOnDeviceRecognitionAvailable(context)) {
                result.error("unavailable", "On-device file transcription requires Android 13 and an installed speech service.", null)
            } else if (call.method == "prepare") {
                result.success(null)
            } else if (owner != null) {
                result.error("busy", "Another transcription is active.", null)
            } else {
                val path = call.argument<String>("path")
                if (path == null) result.error("arguments", "No recording was supplied.", null)
                else start(id, File(path), result)
            }
        }
    }

    @androidx.annotation.RequiresApi(33)
    private fun start(id: String, file: File, result: MethodChannel.Result) {
        owner = id
        reply = result
        segments.clear()
        try {
            // Validate before starting a service. The record plugin writes RIFF WAV,
            // whose data chunk need not start at byte 44.
            val pcm = readTranscriptionWave(file)
            val descriptors = ParcelFileDescriptor.createPipe()
            pipe = descriptors
            val speech = SpeechRecognizer.createOnDeviceSpeechRecognizer(context)
            recognizer = speech
            speech.setRecognitionListener(object : RecognitionListener {
                override fun onReadyForSpeech(params: Bundle?) {}
                override fun onBeginningOfSpeech() {}
                override fun onRmsChanged(rmsdB: Float) {}
                override fun onBufferReceived(buffer: ByteArray?) {}
                override fun onEndOfSpeech() {}
                override fun onError(error: Int) {
                    if (owner == id) fail("On-device transcription failed ($error). Check that your speech language is downloaded.")
                }
                override fun onResults(results: Bundle?) {
                    if (owner != id) return
                    // Non-segmented providers return the complete utterance here.
                    complete(results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull().orEmpty())
                }
                override fun onPartialResults(partialResults: Bundle?) {}
                override fun onEvent(eventType: Int, params: Bundle?) {}
                override fun onSegmentResults(segmentResults: Bundle) {
                    if (owner == id) segmentResults.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()?.let(segments::add)
                }
                override fun onEndOfSegmentedSession() {
                    if (owner == id) complete(segments.joinToString(" "))
                }
            })
            val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
                .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
                .putExtra(RecognizerIntent.EXTRA_LANGUAGE, Locale.getDefault().toLanguageTag())
                .putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE, descriptors[0])
                .putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_CHANNEL_COUNT, 1)
                .putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
                .putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_SAMPLING_RATE, 16000)
                .putExtra(RecognizerIntent.EXTRA_SEGMENTED_SESSION, RecognizerIntent.EXTRA_AUDIO_SOURCE)
            speech.startListening(intent)
            handler.postDelayed(timeout, 60000)
            Thread {
                try {
                    ParcelFileDescriptor.AutoCloseOutputStream(descriptors[1]).use { it.write(pcm) }
                } catch (_: Exception) {
                    handler.post { if (owner == id) fail("The speech service could not read the saved recording.") }
                }
            }.start()
        } catch (_: Exception) { fail("Could not start on-device transcription.") }
    }

    private fun complete(text: String) {
        val result = reply
        cleanup()
        result?.success(text.trim())
    }
    private fun fail(message: String) {
        val result = reply
        cleanup()
        result?.error("transcription_failed", message, null)
    }
    private fun cleanup() {
        owner = null
        reply = null
        handler.removeCallbacks(timeout)
        recognizer?.destroy()
        recognizer = null
        pipe?.forEach { runCatching { it.close() } }
        pipe = null
    }
    fun dispose() { fail("Transcription cancelled."); channel.setMethodCallHandler(null) }
}

/** Validates the recorder's bounded mono PCM WAV, skipping optional RIFF chunks. */
internal fun readTranscriptionWave(file: File): ByteArray = RandomAccessFile(file, "r").use { input ->
    fun fourCC(): String = ByteArray(4).also(input::readFully).toString(Charsets.US_ASCII)
    fun uint32(): Long = Integer.reverseBytes(input.readInt()).toLong() and 0xffffffffL
    fun uint16(): Int = java.lang.Short.reverseBytes(input.readShort()).toInt() and 0xffff
    require(input.length() in 44..10_000_000 && fourCC() == "RIFF")
    uint32()
    require(fourCC() == "WAVE")
    var validFormat = false
    var data: ByteArray? = null
    while (input.filePointer + 8 <= input.length()) {
        val chunk = fourCC()
        val length = uint32()
        val end = input.filePointer + length
        require(end <= input.length())
        if (chunk == "fmt ") {
            require(length >= 16)
            require(uint16() == 1 && uint16() == 1 && uint32() == 16000L)
            uint32(); uint16()
            require(uint16() == 16)
            validFormat = true
        } else if (chunk == "data") {
            require(length > 0 && length % 2 == 0L)
            data = ByteArray(length.toInt()).also(input::readFully)
        }
        input.seek(end + length % 2)
    }
    require(validFormat)
    requireNotNull(data)
}
