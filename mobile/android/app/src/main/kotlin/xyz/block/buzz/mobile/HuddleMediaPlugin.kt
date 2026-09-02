package xyz.block.buzz.mobile

import android.Manifest
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * Foreground-only native seam for mobile Huddle media.
 *
 * Owns microphone permission, foreground communication routing, and the native
 * realtime Opus engine. PCM remains native; Flutter only handles compressed
 * Huddle v2 packets.
 */
internal class HuddleMediaPlugin(
    private val activity: Activity,
    messenger: BinaryMessenger,
) {
    private val channel = MethodChannel(messenger, CHANNEL_NAME)
    private val audioManager = activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val mainHandler = Handler(Looper.getMainLooper())
    private var previousAudioMode: Int? = null
    private var previousCommunicationDevice: AudioDeviceInfo? = null
    private var selectedCommunicationDevice = false
    private var previousSpeakerphoneOn: Boolean? = null
    private var pendingPermissionResult: MethodChannel.Result? = null
    private var audioEngine: HuddleAudioEngine? = null
    private var hasAudioFocus = false
    private val audioFocusListener = AudioManager.OnAudioFocusChangeListener { change ->
        val isInterrupted = change != AudioManager.AUDIOFOCUS_GAIN
        audioEngine?.setInterrupted(isInterrupted)
        mainHandler.post {
            if (audioEngine != null) {
                channel.invokeMethod(
                    "interruptionChanged",
                    mapOf("interrupted" to isInterrupted),
                )
            }
        }
    }

    init {
        channel.setMethodCallHandler(::handleMethodCall)
    }

    private fun handleMethodCall(
        call: MethodCall,
        result: MethodChannel.Result,
    ) {
        when (call.method) {
            "getCapabilities" -> result.success(capabilities())
            "requestMicrophonePermission" -> requestMicrophonePermission(result)
            "openSystemSettings" -> openSystemSettings(result)
            "prepare" -> prepare(call.arguments, result)
            "start" -> start(result)
            "setMuted" -> setMuted(call.arguments, result)
            "setSpeakerEnabled" -> setSpeakerEnabled(call.arguments, result)
            "playRemoteOpusFrame" -> playRemoteOpusFrame(call.arguments, result)
            "removeRemotePeer" -> removeRemotePeer(call.arguments, result)
            "stop" -> stop(result)
            else -> result.notImplemented()
        }
    }

    private fun capabilities(): Map<String, Any> {
        val supportsOpus = HuddleAudioEngine.isSupported()
        return mapOf(
            "platform" to "android",
            "audioSession" to true,
            "microphonePermission" to true,
            "capture" to supportsOpus,
            "playback" to supportsOpus,
            "opusEncoding" to supportsOpus,
            "opusDecoding" to supportsOpus,
        )
    }

    private fun requestMicrophonePermission(result: MethodChannel.Result) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M || hasMicrophonePermission()) {
            result.success("granted")
            return
        }
        if (pendingPermissionResult != null) {
            result.error(
                "permission_request_in_progress",
                "A microphone permission request is already active.",
                null,
            )
            return
        }
        pendingPermissionResult = result
        activity.requestPermissions(
            arrayOf(Manifest.permission.RECORD_AUDIO),
            MICROPHONE_PERMISSION_REQUEST,
        )
    }

    fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ): Boolean {
        if (requestCode != MICROPHONE_PERMISSION_REQUEST) return false
        val result = pendingPermissionResult ?: return true
        pendingPermissionResult = null
        val microphoneIndex = permissions.indexOf(Manifest.permission.RECORD_AUDIO)
        val granted = microphoneIndex >= 0 &&
            grantResults.getOrNull(microphoneIndex) == PackageManager.PERMISSION_GRANTED
        result.success(if (granted) "granted" else "denied")
        return true
    }

    /**
     * Open this app's Android settings screen so the user can grant a
     * previously denied microphone permission. Mirrors the iOS recovery path.
     */
    private fun openSystemSettings(result: MethodChannel.Result) {
        val intent = Intent(
            Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
            Uri.fromParts("package", activity.packageName, null),
        ).apply { addFlags(Intent.FLAG_ACTIVITY_NEW_TASK) }
        try {
            activity.startActivity(intent)
            result.success(true)
        } catch (error: ActivityNotFoundException) {
            result.success(false)
        }
    }

    private fun prepare(arguments: Any?, result: MethodChannel.Result) {
        val values = arguments as? Map<*, *>
        if (values?.get("protocolVersion") != 2 ||
            values["sampleRateHz"] != 48_000 ||
            values["channels"] != 1 ||
            values["frameSamples"] != 960
        ) {
            result.error(
                "invalid_configuration",
                "Expected the fixed Huddle Opus v2 media configuration.",
                null,
            )
            return
        }
        if (!hasMicrophonePermission()) {
            result.error(
                "microphone_permission_denied",
                "Microphone permission is required for a Huddle.",
                null,
            )
            return
        }
        if (!HuddleAudioEngine.isSupported()) {
            result.error(
                "unsupported",
                "This Android device does not expose Opus encode/decode codecs.",
                null,
            )
            return
        }

        try {
            if (previousAudioMode == null) previousAudioMode = audioManager.mode
            audioManager.mode = AudioManager.MODE_IN_COMMUNICATION
            @Suppress("DEPRECATION")
            val focusResult = audioManager.requestAudioFocus(
                audioFocusListener,
                AudioManager.STREAM_VOICE_CALL,
                AudioManager.AUDIOFOCUS_GAIN_TRANSIENT,
            )
            check(focusResult == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
                "Android did not grant communication audio focus."
            }
            hasAudioFocus = true
            selectForegroundOutput()
            result.success(
                mapOf(
                    "audioSessionPrepared" to true,
                    "sampleRateHz" to 48_000,
                    "channels" to 1,
                    "frameSamples" to 960,
                ),
            )
        } catch (error: RuntimeException) {
            restoreAudioSession()
            result.error(
                "audio_session_failed",
                "Unable to prepare the Android Huddle audio session.",
                error.message,
            )
        }
    }

    private fun start(result: MethodChannel.Result) {
        if (previousAudioMode == null) {
            result.error(
                "invalid_state",
                "Prepare Huddle audio before starting it.",
                null,
            )
            return
        }
        if (audioEngine != null) {
            result.error("invalid_state", "Huddle audio is already running.", null)
            return
        }

        try {
            val engine = HuddleAudioEngine(
                onLocalPacket = ::emitLocalPacket,
                onFailure = ::emitNativeFailure,
                onDiagnostics = ::emitCaptureDiagnostics,
                diagnosticsEnabled =
                    (activity.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0,
            )
            engine.start()
            audioEngine = engine
            result.success(null)
        } catch (error: Throwable) {
            result.error(
                "media_start_failed",
                "Unable to start Android Huddle audio.",
                error.message,
            )
        }
    }

    private fun setMuted(arguments: Any?, result: MethodChannel.Result) {
        val muted = (arguments as? Map<*, *>)?.get("muted") as? Boolean
        if (muted == null) {
            result.error("invalid_arguments", "Missing Huddle mute state.", null)
            return
        }
        try {
            val engine = audioEngine ?: error("Huddle audio is not running.")
            engine.setMuted(muted)
            result.success(null)
        } catch (error: Throwable) {
            result.error("invalid_state", error.message, null)
        }
    }

    private fun setSpeakerEnabled(arguments: Any?, result: MethodChannel.Result) {
        val enabled = (arguments as? Map<*, *>)?.get("enabled") as? Boolean
        if (enabled == null) {
            result.error("invalid_arguments", "Missing Huddle speaker state.", null)
            return
        }
        if (previousAudioMode == null || audioEngine == null) {
            result.error("invalid_state", "Huddle audio is not running.", null)
            return
        }

        try {
            routeToBuiltInOutput(enabled)
            result.success(null)
        } catch (error: RuntimeException) {
            result.error("audio_route_failed", "Unable to change Huddle audio output.", error.message)
        }
    }

    private fun playRemoteOpusFrame(arguments: Any?, result: MethodChannel.Result) {
        val values = arguments as? Map<*, *>
        val peerIndex = (values?.get("peerIndex") as? Number)?.toInt()
        val sequence = (values?.get("sequence") as? Number)?.toInt()
        val timestamp48k = (values?.get("timestamp48k") as? Number)?.toLong()
        val levelDbov = (values?.get("levelDbov") as? Number)?.toInt()
        val opus = values?.get("opus") as? ByteArray
        if (peerIndex == null || peerIndex !in 0..255 ||
            sequence == null || sequence !in 0..0xffff ||
            timestamp48k == null || timestamp48k !in 0..0xffff_ffffL ||
            levelDbov == null || levelDbov !in -127..0 ||
            opus == null || opus.isEmpty() || opus.size > MAX_OPUS_PACKET_BYTES
        ) {
            result.error("invalid_arguments", "Malformed remote Huddle Opus packet.", null)
            return
        }

        try {
            val engine = audioEngine ?: error("Huddle audio is not running.")
            engine.enqueueRemote(
                HuddleRemoteOpusPacket(
                    peerIndex = peerIndex,
                    sequence = sequence,
                    timestamp48k = timestamp48k,
                    levelDbov = levelDbov,
                    opus = opus.copyOf(),
                ),
            )
            result.success(null)
        } catch (error: Throwable) {
            result.error("playback_failed", error.message, null)
        }
    }

    private fun removeRemotePeer(arguments: Any?, result: MethodChannel.Result) {
        val peerIndex = ((arguments as? Map<*, *>)?.get("peerIndex") as? Number)?.toInt()
        if (peerIndex == null || peerIndex !in 0..255) {
            result.error("invalid_arguments", "Missing Huddle peer index.", null)
            return
        }
        try {
            val engine = audioEngine ?: error("Huddle audio is not running.")
            engine.removeRemotePeer(peerIndex)
            result.success(null)
        } catch (error: Throwable) {
            result.error("playback_failed", error.message, null)
        }
    }

    private fun stop(result: MethodChannel.Result) {
        try {
            audioEngine?.stop()
            audioEngine = null
            restoreAudioSession()
            result.success(null)
        } catch (error: RuntimeException) {
            result.error(
                "audio_session_stop_failed",
                "Unable to stop the Android Huddle audio session.",
                error.message,
            )
        }
    }

    fun dispose() {
        runCatching { audioEngine?.stop() }
        audioEngine = null
        restoreAudioSession()
        pendingPermissionResult?.error(
            "disposed",
            "Huddle media was disposed during microphone permission.",
            null,
        )
        pendingPermissionResult = null
        channel.setMethodCallHandler(null)
    }

    private fun emitLocalPacket(packet: HuddleLocalOpusPacket) {
        mainHandler.post {
            if (audioEngine == null) return@post
            channel.invokeMethod(
                "localOpusFrame",
                mapOf(
                    "sequence" to packet.sequence,
                    "timestamp48k" to packet.timestamp48k,
                    "levelDbov" to packet.levelDbov,
                    "flags" to packet.flags,
                    "opus" to packet.opus,
                ),
            )
        }
    }

    private fun emitNativeFailure(code: String, message: String) {
        mainHandler.post {
            if (audioEngine == null) return@post
            channel.invokeMethod(
                "nativeError",
                mapOf("code" to code, "message" to message),
            )
        }
    }

    private fun emitCaptureDiagnostics(diagnostics: HuddleCaptureDiagnostics) {
        mainHandler.post {
            if (audioEngine == null) return@post
            channel.invokeMethod(
                "captureDiagnostics",
                mapOf(
                    "frameCount" to diagnostics.frameCount,
                    "rmsDbovHistogram" to diagnostics.rmsDbovHistogram,
                    "peakDbovHistogram" to diagnostics.peakDbovHistogram,
                    "maxPeakDbov" to diagnostics.maxPeakDbov,
                    "audioSource" to diagnostics.audioSource,
                    "audioSourceLabel" to diagnostics.audioSourceLabel,
                    "deviceId" to diagnostics.deviceId,
                    "deviceType" to diagnostics.deviceType,
                    "deviceLabel" to diagnostics.deviceLabel,
                    "acousticEchoCancelerAvailable" to
                        diagnostics.acousticEchoCancelerAvailable,
                    "acousticEchoCancelerEnabled" to
                        diagnostics.acousticEchoCancelerEnabled,
                    "noiseSuppressorAvailable" to diagnostics.noiseSuppressorAvailable,
                    "noiseSuppressorEnabled" to diagnostics.noiseSuppressorEnabled,
                    "automaticGainControlAvailable" to
                        diagnostics.automaticGainControlAvailable,
                    "automaticGainControlEnabled" to
                        diagnostics.automaticGainControlEnabled,
                ),
            )
        }
    }

    @Suppress("DEPRECATION")
    private fun selectForegroundOutput() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            if (selectedCommunicationDevice) return
            previousCommunicationDevice = audioManager.communicationDevice
            val currentType = previousCommunicationDevice?.type
            if (currentType in externalCommunicationDeviceTypes) return
            val earpiece = audioManager.availableCommunicationDevices.firstOrNull {
                it.type == AudioDeviceInfo.TYPE_BUILTIN_EARPIECE
            } ?: return
            selectedCommunicationDevice = audioManager.setCommunicationDevice(earpiece)
            return
        }
        if (previousSpeakerphoneOn == null) {
            previousSpeakerphoneOn = audioManager.isSpeakerphoneOn
            audioManager.isSpeakerphoneOn = false
        }
    }

    @Suppress("DEPRECATION")
    private fun routeToBuiltInOutput(useSpeaker: Boolean) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            if (previousCommunicationDevice == null) {
                previousCommunicationDevice = audioManager.communicationDevice
            }
            val targetType = if (useSpeaker) {
                AudioDeviceInfo.TYPE_BUILTIN_SPEAKER
            } else {
                AudioDeviceInfo.TYPE_BUILTIN_EARPIECE
            }
            val target = audioManager.availableCommunicationDevices.firstOrNull {
                it.type == targetType
            } ?: error("The requested built-in Huddle output is unavailable.")
            check(audioManager.setCommunicationDevice(target)) {
                "Android rejected the requested Huddle output."
            }
            selectedCommunicationDevice = true
            return
        }

        if (previousSpeakerphoneOn == null) {
            previousSpeakerphoneOn = audioManager.isSpeakerphoneOn
        }
        audioManager.isSpeakerphoneOn = useSpeaker
    }

    @Suppress("DEPRECATION")
    private fun restoreForegroundOutput() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            if (selectedCommunicationDevice) {
                val previous = previousCommunicationDevice
                if (previous != null &&
                    audioManager.availableCommunicationDevices.any { it.id == previous.id }
                ) {
                    audioManager.setCommunicationDevice(previous)
                } else {
                    audioManager.clearCommunicationDevice()
                }
            }
            previousCommunicationDevice = null
            selectedCommunicationDevice = false
            return
        }
        previousSpeakerphoneOn?.let { audioManager.isSpeakerphoneOn = it }
        previousSpeakerphoneOn = null
    }

    private fun restoreAudioSession() {
        if (hasAudioFocus) {
            @Suppress("DEPRECATION")
            audioManager.abandonAudioFocus(audioFocusListener)
            hasAudioFocus = false
        }
        runCatching { restoreForegroundOutput() }
        runCatching {
            previousAudioMode?.let { audioManager.mode = it }
        }
        previousAudioMode = null
    }

    private fun hasMicrophonePermission(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.M ||
            activity.checkSelfPermission(Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED

    private companion object {
        const val CHANNEL_NAME = "buzz/huddle_media"
        const val MICROPHONE_PERMISSION_REQUEST = 7342
        const val MAX_OPUS_PACKET_BYTES = 4088

        val externalCommunicationDeviceTypes = setOf(
            AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
            AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
            AudioDeviceInfo.TYPE_BLE_HEADSET,
            AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
            AudioDeviceInfo.TYPE_WIRED_HEADSET,
            AudioDeviceInfo.TYPE_USB_DEVICE,
            AudioDeviceInfo.TYPE_USB_HEADSET,
        )
    }
}
