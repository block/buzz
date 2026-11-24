package xyz.block.buzz.mobile

import android.content.ComponentName
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
import java.io.FileInputStream
import java.util.Locale

class ComposerTranscriptionPlugin(private val context: Context, messenger: BinaryMessenger) {
    private val channel = MethodChannel(messenger, "buzz/composer_transcription")
    private val sessions = mutableMapOf<String, Session>()
    private val handler = Handler(Looper.getMainLooper())

    // EXTRA_AUDIO_SOURCE is optional. An unknown service can silently capture
    // the microphone instead, so enable only a file-input service we verified.
    private fun available(): Boolean {
        if (Build.VERSION.SDK_INT < 33 || !SpeechRecognizer.isOnDeviceRecognitionAvailable(context)) return false
        val resources = context.resources
        val id = resources.getIdentifier("config_defaultOnDeviceSpeechRecognitionService", "string", "android")
        if (id == 0) return false
        val service = ComponentName.unflattenFromString(resources.getString(id))
        return service?.packageName == "com.google.android.as" &&
            service.className == "com.google.android.apps.miphone.aiai.app.AiAiSpeechRecognitionService"
    }

    init {
        channel.setMethodCallHandler { call, result ->
            val id = call.argument<String>("id") ?: ""
            when (call.method) {
                "isAvailable" -> result.success(available())
                "prepare" -> if (available()) result.success(null)
                    else result.error("unavailable", "On-device recognition is unavailable.", null)
                "cancel" -> {
                    sessions[id]?.finish(null, "Cancelled")
                    result.success(null)
                }
                "transcribe" -> try {
                    check(available())
                    val path = requireNotNull(call.argument<String>("path"))
                    val session = Session(id, result, SpeechRecognizer.createOnDeviceSpeechRecognizer(context))
                    sessions[id]?.finish(null, "Cancelled")
                    sessions[id] = session
                    session.start(path)
                } catch (e: Exception) {
                    if (sessions.containsKey(id)) sessions[id]?.finish(null, e.message ?: "Recognition failed")
                    else result.error("transcription", e.message, null)
                }
                else -> result.notImplemented()
            }
        }
    }

    private inner class Session(val id: String, val result: MethodChannel.Result, val recognizer: SpeechRecognizer) : RecognitionListener {
        var input: ParcelFileDescriptor? = null
        val segments = mutableListOf<String>()
        val timeout = Runnable { finish(null, "Recognition timed out.") }

        fun start(path: String) {
            // The dictation recorder writes mono 16 kHz PCM16 WAV. Pass the
            // descriptor positioned at the data chunk, excluding RIFF metadata.
            FileInputStream(path).use { file ->
                val header = java.io.DataInputStream(file)
                check(header.readInt() == 0x52494646)
                header.readInt()
                check(header.readInt() == 0x57415645)
                while (true) {
                    val tag = header.readInt()
                    val size = Integer.reverseBytes(header.readInt()).toLong() and 0xffffffffL
                    if (tag == 0x64617461) break
                    file.channel.position(file.channel.position() + size + size % 2)
                }
                input = ParcelFileDescriptor.dup(file.fd)
            }
            val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
                putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
                putExtra(RecognizerIntent.EXTRA_LANGUAGE, Locale.getDefault().toLanguageTag())
                putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE, input)
                putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_CHANNEL_COUNT, 1)
                putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
                putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_SAMPLING_RATE, 16000)
                putExtra(RecognizerIntent.EXTRA_SEGMENTED_SESSION, RecognizerIntent.EXTRA_AUDIO_SOURCE)
            }
            recognizer.setRecognitionListener(this)
            handler.postDelayed(timeout, 360000)
            recognizer.startListening(intent)
        }

        fun finish(text: String?, error: String?) {
            if (sessions[id] !== this) return
            sessions.remove(id)
            handler.removeCallbacks(timeout)
            recognizer.destroy()
            input?.close()
            if (error == null) result.success(text) else result.error("transcription", error, null)
        }
        fun best(bundle: Bundle?) = bundle?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull().orEmpty()
        override fun onResults(results: Bundle?) = finish(null, "Recognizer did not complete the file-input session.")
        override fun onSegmentResults(results: Bundle) { segments.add(best(results)) }
        override fun onEndOfSegmentedSession() = finish(segments.joinToString(" "), null)
        override fun onError(error: Int) = finish(null, "Android speech recognition returned error $error.")
        override fun onReadyForSpeech(params: Bundle?) {}
        override fun onBeginningOfSpeech() {}
        override fun onRmsChanged(rmsdB: Float) {}
        override fun onBufferReceived(buffer: ByteArray?) {}
        override fun onEndOfSpeech() {}
        override fun onPartialResults(results: Bundle?) {}
        override fun onEvent(eventType: Int, params: Bundle?) {}
    }
    fun dispose() {
        channel.setMethodCallHandler(null)
        sessions.values.toList().forEach { it.finish(null, "Cancelled") }
    }
}
