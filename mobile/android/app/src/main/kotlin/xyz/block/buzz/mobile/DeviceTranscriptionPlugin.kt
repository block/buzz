package xyz.block.buzz.mobile

import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel

/** Fail before capture: Android's generic recognizer cannot guarantee file input. */
class DeviceTranscriptionPlugin(messenger: BinaryMessenger) {
    private val channel = MethodChannel(messenger, "buzz/transcription")

    init {
        channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "cancel" -> result.success(null)
                "prepare", "transcribe" -> result.error(
                    "unavailable",
                    "Transcribing saved recordings is not available on Android yet.",
                    null,
                )
                else -> result.notImplemented()
            }
        }
    }

    fun dispose() { channel.setMethodCallHandler(null) }
}
