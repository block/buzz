package xyz.block.buzz.mobile

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaCodec
import android.media.MediaCodecList
import android.media.MediaFormat
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import android.os.Process
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.ArrayDeque
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread
import kotlin.math.log10
import kotlin.math.max
import kotlin.math.roundToInt

/** One compressed microphone packet emitted to Flutter's Huddle transport. */
internal data class HuddleLocalOpusPacket(
    val sequence: Int,
    val timestamp48k: Long,
    val levelDbov: Int,
    val flags: Int,
    val opus: ByteArray,
)

/** One relay-authored packet queued for realtime native playout. */
internal data class HuddleRemoteOpusPacket(
    val peerIndex: Int,
    val sequence: Int,
    val timestamp48k: Long,
    val levelDbov: Int,
    val opus: ByteArray,
)

internal data class HuddleTalkerSelection(
    val accepted: Boolean,
    val evictedPeerIndex: Int?,
) {
    inline fun <T> allocateIfAccepted(allocate: () -> T): T? =
        if (accepted) allocate() else null
}

/// Fixed-capacity active-talker selector. Roster membership stays in Dart;
/// native decoder/track resources exist only for peers that actually send.
internal class HuddleActiveTalkerSelector(
    private val capacity: Int,
    private val nowMs: () -> Long = { System.nanoTime() / 1_000_000 },
    private val inactivityTimeoutMs: Long = 1_000,
) {
    internal data class Activity(
        val peerIndex: Int,
        val lastPacketOrdinal: Long,
        val lastPacketAtMs: Long,
        val levelDbov: Int,
    )

    private val active = mutableMapOf<Int, Activity>()
    private var ordinal = 0L

    fun activate(peerIndex: Int, levelDbov: Int): HuddleTalkerSelection {
        ordinal += 1
        val now = nowMs()
        if (active.containsKey(peerIndex)) {
            active[peerIndex] = Activity(peerIndex, ordinal, now, levelDbov)
            return HuddleTalkerSelection(accepted = true, evictedPeerIndex = null)
        }
        val weakest = if (active.size >= capacity) {
            active.values.minWithOrNull(
                compareBy<Activity> { it.levelDbov }
                    .thenBy { it.lastPacketOrdinal }
                    .thenBy { it.peerIndex },
            )
        } else {
            null
        }
        val expired = active.values
            .filter { now - it.lastPacketAtMs >= inactivityTimeoutMs }
            .minWithOrNull(compareBy<Activity> { it.lastPacketAtMs }.thenBy { it.peerIndex })
        // Equal-level packets (especially continuous -127 dBov silence) must
        // not churn decoder/jitter state. A new peer earns a slot when any
        // selected slot is inactive or the new packet is actually louder.
        if (weakest != null && expired == null && levelDbov <= weakest.levelDbov) {
            return HuddleTalkerSelection(accepted = false, evictedPeerIndex = null)
        }
        val evicted = expired?.peerIndex ?: weakest?.peerIndex
        evicted?.let(active::remove)
        active[peerIndex] = Activity(peerIndex, ordinal, now, levelDbov)
        return HuddleTalkerSelection(accepted = true, evictedPeerIndex = evicted)
    }

    fun remove(peerIndex: Int) {
        active.remove(peerIndex)
    }

    fun indices(): Set<Int> = active.keys.toSet()
}

internal sealed interface HuddlePlaybackCommand {
    data class Packet(val packet: HuddleRemoteOpusPacket) : HuddlePlaybackCommand
    data class RemovePeer(val peerIndex: Int) : HuddlePlaybackCommand
}

internal class HuddlePacketJitterQueue(
    private val capacity: Int,
    private val startPackets: Int,
) {
    private val packets = mutableListOf<HuddleRemoteOpusPacket>()
    private var lastDrainedSequence: Int? = null
    private var started = false

    fun enqueue(packet: HuddleRemoteOpusPacket) {
        val lastDrained = lastDrainedSequence
        if (lastDrained != null && !sequenceAfter(packet.sequence, lastDrained)) return
        if (packets.any { it.sequence == packet.sequence }) return
        val insertionIndex = packets.indexOfFirst { sequenceBefore(packet.sequence, it.sequence) }
        if (insertionIndex < 0) packets.add(packet) else packets.add(insertionIndex, packet)
        if (packets.size > capacity) packets.removeAt(0)
        if (packets.size >= startPackets) started = true
    }

    fun drainOne(): HuddleRemoteOpusPacket? {
        if (!started || packets.isEmpty()) return null
        return packets.removeAt(0).also { lastDrainedSequence = it.sequence }
    }

    fun sequences(): List<Int> = packets.map { it.sequence }

    private fun sequenceAfter(sequence: Int, reference: Int): Boolean {
        val distance = (sequence - reference) and 0xffff
        return distance in 1..0x7fff
    }

    private fun sequenceBefore(left: Int, right: Int): Boolean = sequenceAfter(right, left)
}

/** Aggregate, debug-only microphone telemetry. No PCM or encoded audio is retained. */
internal data class HuddleCaptureDiagnostics(
    val frameCount: Long,
    val rmsDbovHistogram: Map<String, Long>,
    val peakDbovHistogram: Map<String, Long>,
    val maxPeakDbov: Int,
    val audioSource: Int,
    val audioSourceLabel: String,
    val deviceId: Int?,
    val deviceType: Int?,
    val deviceLabel: String?,
    val acousticEchoCancelerAvailable: Boolean,
    val acousticEchoCancelerEnabled: Boolean?,
    val noiseSuppressorAvailable: Boolean,
    val noiseSuppressorEnabled: Boolean?,
    val automaticGainControlAvailable: Boolean,
    val automaticGainControlEnabled: Boolean?,
)

/**
 * Foreground-only Android Huddle audio engine.
 *
 * PCM never crosses Flutter. AudioRecord feeds Android's Opus MediaCodec and
 * compressed packets are delivered to Flutter; inbound compressed packets are
 * decoded with MediaCodec and written to per-peer AudioTracks, which Android's
 * communication mixer combines. Each peer gets a small bounded jitter queue.
 */
internal class HuddleAudioEngine(
    private val onLocalPacket: (HuddleLocalOpusPacket) -> Unit,
    private val onFailure: (code: String, message: String) -> Unit,
    private val onDiagnostics: (HuddleCaptureDiagnostics) -> Unit,
    private val diagnosticsEnabled: Boolean,
) {
    private val running = AtomicBoolean(false)
    private val failureReported = AtomicBoolean(false)
    private val playbackQueue = ArrayBlockingQueue<HuddlePlaybackCommand>(PLAYBACK_QUEUE_CAPACITY)
    private val interrupted = AtomicBoolean(false)
    private val interruptionEpoch = AtomicInteger(0)
    private val interruptionLock = Object()

    @Volatile
    private var muted = false

    @Volatile
    private var audioRecord: AudioRecord? = null

    @Volatile
    private var captureThread: Thread? = null

    @Volatile
    private var playbackThread: Thread? = null

    fun start() {
        check(!running.get()) { "Huddle audio is already running." }
        check(isSupported()) { "This device does not expose Android Opus encode/decode codecs." }

        val record = createAudioRecord()
        val encoder = try {
            createEncoder()
        } catch (error: Throwable) {
            runCatching { record.release() }
            throw error
        }
        try {
            encoder.start()
            record.startRecording()
            check(record.recordingState == AudioRecord.RECORDSTATE_RECORDING) {
                "Microphone recording did not start."
            }
        } catch (error: Throwable) {
            runCatching { encoder.release() }
            runCatching { record.release() }
            throw error
        }

        audioRecord = record
        muted = false
        failureReported.set(false)
        running.set(true)
        captureThread = thread(name = "buzz-huddle-capture", isDaemon = true) {
            runCapture(record, encoder)
        }
        playbackThread = thread(name = "buzz-huddle-playback", isDaemon = true) {
            runPlayback()
        }
    }

    fun setMuted(value: Boolean) {
        check(running.get()) { "Huddle audio is not running." }
        muted = value
    }

    fun setInterrupted(value: Boolean) {
        val record = audioRecord
        if (value) {
            interrupted.set(true)
            interruptionEpoch.incrementAndGet()
            // A focus loss must stop native capture, not merely discard frames
            // after AudioRecord has already read them from the microphone.
            if (record != null) runCatching { record.stop() }
            return
        }
        if (!running.get() || record == null) {
            interrupted.set(false)
            synchronized(interruptionLock) { interruptionLock.notifyAll() }
            return
        }
        try {
            record.startRecording()
            check(record.recordingState == AudioRecord.RECORDSTATE_RECORDING) {
                "Microphone recording did not resume."
            }
            interrupted.set(false)
            synchronized(interruptionLock) { interruptionLock.notifyAll() }
        } catch (error: Throwable) {
            reportFailure("audio_resume_failed", error)
        }
    }

    fun enqueueRemote(packet: HuddleRemoteOpusPacket) {
        check(running.get()) { "Huddle audio is not running." }
        offerPlayback(HuddlePlaybackCommand.Packet(packet))
    }

    fun removeRemotePeer(peerIndex: Int) {
        check(running.get()) { "Huddle audio is not running." }
        playbackQueue.removeIf { command ->
            command is HuddlePlaybackCommand.Packet && command.packet.peerIndex == peerIndex
        }
        offerPlayback(HuddlePlaybackCommand.RemovePeer(peerIndex))
    }

    private fun offerPlayback(command: HuddlePlaybackCommand) {
        if (playbackQueue.offer(command)) return

        // Packets are expendable realtime data; peer-removal commands are not.
        // Make room by evicting the oldest packet only, so an audio flood can
        // never resurrect stale decoder/track state after index reuse.
        val oldestPacket = playbackQueue.firstOrNull {
            it is HuddlePlaybackCommand.Packet
        }
        if (oldestPacket != null) playbackQueue.remove(oldestPacket)
        playbackQueue.offer(command)
    }

    fun stop() {
        running.set(false)
        synchronized(interruptionLock) { interruptionLock.notifyAll() }
        runCatching { audioRecord?.stop() }
        playbackThread?.interrupt()
        runCatching { captureThread?.join(STOP_JOIN_TIMEOUT_MS) }
        runCatching { playbackThread?.join(STOP_JOIN_TIMEOUT_MS) }
        captureThread = null
        playbackThread = null
        audioRecord = null
        playbackQueue.clear()
    }

    private fun runCapture(record: AudioRecord, encoder: MediaCodec) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
        val echoCanceler = if (AcousticEchoCanceler.isAvailable()) {
            AcousticEchoCanceler.create(record.audioSessionId)?.apply { enabled = true }
        } else {
            null
        }
        val noiseSuppressor = if (NoiseSuppressor.isAvailable()) {
            NoiseSuppressor.create(record.audioSessionId)?.apply { enabled = true }
        } else {
            null
        }
        // VOICE_COMMUNICATION may install AGC by default. Creating the control
        // lets debug diagnostics report the platform's actual state without
        // enabling or otherwise changing it.
        val automaticGainControl = if (AutomaticGainControl.isAvailable()) {
            AutomaticGainControl.create(record.audioSessionId)
        } else {
            null
        }

        val readBuffer = ShortArray(FRAME_SAMPLES)
        val frame = ShortArray(FRAME_SAMPLES)
        var filled = 0
        var frameIndex = 0L
        var sequence = 0
        val levelsByPresentationTime = ArrayDeque<Pair<Long, Int>>()
        var diagnosticFrameCount = 0L
        var diagnosticMaxPeakDbov = SILENT_LEVEL_DBOV
        val rmsDbovHistogram = mutableMapOf<Int, Long>()
        val peakDbovHistogram = mutableMapOf<Int, Long>()

        if (diagnosticsEnabled) {
            onDiagnostics(
                captureDiagnostics(
                    record = record,
                    echoCanceler = echoCanceler,
                    noiseSuppressor = noiseSuppressor,
                    automaticGainControl = automaticGainControl,
                    frameCount = diagnosticFrameCount,
                    rmsDbovHistogram = rmsDbovHistogram,
                    peakDbovHistogram = peakDbovHistogram,
                    maxPeakDbov = diagnosticMaxPeakDbov,
                ),
            )
        }

        try {
            while (running.get()) {
                // Snapshot before checking [interrupted]. If focus loss starts
                // after that check, its epoch still marks the stopped read as
                // expected; if it starts first, the loop waits below.
                val readEpoch = interruptionEpoch.get()
                if (interrupted.get()) {
                    synchronized(interruptionLock) {
                        while (running.get() && interrupted.get()) {
                            interruptionLock.wait(INTERRUPTION_WAIT_MS)
                        }
                    }
                    continue
                }
                val read = record.read(
                    readBuffer,
                    0,
                    readBuffer.size,
                    AudioRecord.READ_BLOCKING,
                )
                if (read < 0) {
                    // AudioRecord.stop() unblocks a pending read with an error.
                    // A focus interruption that began during this read owns that
                    // result even if focus was regained before we inspected it.
                    if (interruptionEpoch.get() != readEpoch) continue
                    throw IllegalStateException("AudioRecord read failed with code $read.")
                }
                var sourceOffset = 0
                while (sourceOffset < read && running.get()) {
                    val copied = minOf(FRAME_SAMPLES - filled, read - sourceOffset)
                    readBuffer.copyInto(
                        frame,
                        destinationOffset = filled,
                        startIndex = sourceOffset,
                        endIndex = sourceOffset + copied,
                    )
                    sourceOffset += copied
                    filled += copied
                    if (filled != FRAME_SAMPLES) continue

                    sequence = drainEncoder(
                        encoder = encoder,
                        levelsByPresentationTime = levelsByPresentationTime,
                        nextSequence = sequence,
                    )
                    if (!muted && !interrupted.get()) {
                        val presentationTimeUs = frameIndex * FRAME_DURATION_US
                        val level = audioLevelDbov(frame)
                        if (diagnosticsEnabled) {
                            val peak = audioPeakDbov(frame)
                            diagnosticFrameCount += 1
                            diagnosticMaxPeakDbov = max(diagnosticMaxPeakDbov, peak)
                            rmsDbovHistogram[level] = (rmsDbovHistogram[level] ?: 0L) + 1L
                            peakDbovHistogram[peak] = (peakDbovHistogram[peak] ?: 0L) + 1L
                            if (diagnosticFrameCount % DIAGNOSTIC_EMIT_FRAME_INTERVAL == 0L) {
                                onDiagnostics(
                                    captureDiagnostics(
                                        record = record,
                                        echoCanceler = echoCanceler,
                                        noiseSuppressor = noiseSuppressor,
                                        automaticGainControl = automaticGainControl,
                                        frameCount = diagnosticFrameCount,
                                        rmsDbovHistogram = rmsDbovHistogram,
                                        peakDbovHistogram = peakDbovHistogram,
                                        maxPeakDbov = diagnosticMaxPeakDbov,
                                    ),
                                )
                            }
                        }
                        queueEncoderInput(encoder, frame, presentationTimeUs)
                        levelsByPresentationTime.addLast(presentationTimeUs to level)
                        frameIndex += 1
                    }
                    filled = 0
                    sequence = drainEncoder(
                        encoder = encoder,
                        levelsByPresentationTime = levelsByPresentationTime,
                        nextSequence = sequence,
                    )
                }
            }
        } catch (error: Throwable) {
            if (running.get()) reportFailure("capture_failed", error)
        } finally {
            running.set(false)
            runCatching { record.stop() }
            runCatching { record.release() }
            runCatching { encoder.stop() }
            runCatching { encoder.release() }
            runCatching { echoCanceler?.release() }
            runCatching { noiseSuppressor?.release() }
            runCatching { automaticGainControl?.release() }
        }
    }

    private fun queueEncoderInput(
        encoder: MediaCodec,
        samples: ShortArray,
        presentationTimeUs: Long,
    ) {
        val inputIndex = encoder.dequeueInputBuffer(CODEC_DEQUEUE_TIMEOUT_US)
        check(inputIndex >= 0) { "Opus encoder did not provide an input buffer." }
        val input = checkNotNull(encoder.getInputBuffer(inputIndex)) {
            "Opus encoder input buffer was unavailable."
        }
        input.clear()
        input.order(ByteOrder.LITTLE_ENDIAN)
        for (sample in samples) input.putShort(sample)
        encoder.queueInputBuffer(
            inputIndex,
            0,
            FRAME_BYTES,
            presentationTimeUs,
            0,
        )
    }

    private fun drainEncoder(
        encoder: MediaCodec,
        levelsByPresentationTime: ArrayDeque<Pair<Long, Int>>,
        nextSequence: Int,
    ): Int {
        val info = MediaCodec.BufferInfo()
        var sequence = nextSequence
        while (true) {
            val outputIndex = encoder.dequeueOutputBuffer(info, 0)
            when {
                outputIndex == MediaCodec.INFO_TRY_AGAIN_LATER -> return sequence
                outputIndex == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> continue
                outputIndex >= 0 -> {
                    try {
                        if (info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
                            val output = checkNotNull(encoder.getOutputBuffer(outputIndex))
                            output.position(info.offset)
                            output.limit(info.offset + info.size)
                            val opus = ByteArray(info.size)
                            output.get(opus)
                            val level = removeLevel(
                                levelsByPresentationTime,
                                info.presentationTimeUs,
                            )
                            onLocalPacket(
                                HuddleLocalOpusPacket(
                                    sequence = sequence,
                                    timestamp48k = presentationTimeTo48k(info.presentationTimeUs),
                                    levelDbov = level,
                                    flags = 0,
                                    opus = opus,
                                ),
                            )
                            sequence = (sequence + 1) and 0xffff
                        }
                    } finally {
                        encoder.releaseOutputBuffer(outputIndex, false)
                    }
                }
            }
        }
    }

    private fun removeLevel(
        levels: ArrayDeque<Pair<Long, Int>>,
        presentationTimeUs: Long,
    ): Int {
        while (levels.isNotEmpty()) {
            val entry = levels.removeFirst()
            if (entry.first == presentationTimeUs) return entry.second
        }
        return SILENT_LEVEL_DBOV
    }

    private fun runPlayback() {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
        val playbacks = mutableMapOf<Int, PeerPlayback>()
        val activeTalkers = HuddleActiveTalkerSelector(MAX_REMOTE_PEERS)
        try {
            while (running.get()) {
                val command = try {
                    playbackQueue.poll(FRAME_DURATION_US, TimeUnit.MICROSECONDS)
                } catch (_: InterruptedException) {
                    break
                }
                when (command) {
                    is HuddlePlaybackCommand.Packet -> {
                        val packet = command.packet
                        val selection = activeTalkers.activate(
                            packet.peerIndex,
                            packet.levelDbov,
                        )
                        selection.evictedPeerIndex?.let {
                            playbacks.remove(it)?.release()
                        }
                        val playback = playbacks[packet.peerIndex]
                            ?: selection.allocateIfAccepted {
                                PeerPlayback(packet.peerIndex).also {
                                    playbacks[packet.peerIndex] = it
                                }
                            }
                        playback?.enqueue(packet)
                    }
                    is HuddlePlaybackCommand.RemovePeer -> {
                        activeTalkers.remove(command.peerIndex)
                        playbacks.remove(command.peerIndex)?.release()
                    }
                    null -> Unit
                }
                if (!interrupted.get()) {
                    for (playback in playbacks.values) playback.drainOne()
                }
            }
        } catch (error: Throwable) {
            if (running.get()) reportFailure("playback_failed", error)
        } finally {
            for (playback in playbacks.values) runCatching { playback.release() }
        }
    }

    private fun reportFailure(code: String, error: Throwable) {
        if (!failureReported.compareAndSet(false, true)) return
        running.set(false)
        onFailure(code, error.message ?: "Native Huddle audio failed.")
    }

    private class PeerPlayback private constructor(
        val peerIndex: Int,
        private val decoder: MediaCodec,
        private val track: AudioTrack,
    ) {
        private val jitterQueue = HuddlePacketJitterQueue(
            capacity = PER_PEER_JITTER_CAPACITY,
            startPackets = JITTER_START_PACKETS,
        )

        companion object {
            operator fun invoke(peerIndex: Int): PeerPlayback {
                val decoder = createDecoder()
                val track = try {
                    createAudioTrack()
                } catch (error: Throwable) {
                    runCatching { decoder.release() }
                    throw error
                }
                return PeerPlayback(peerIndex, decoder, track)
            }
        }

        init {
            try {
                decoder.start()
                track.play()
            } catch (error: Throwable) {
                runCatching { track.release() }
                runCatching { decoder.stop() }
                runCatching { decoder.release() }
                throw error
            }
        }

        fun enqueue(packet: HuddleRemoteOpusPacket) {
            jitterQueue.enqueue(packet)
        }

        fun drainOne() {
            jitterQueue.drainOne()?.let(::decode)
        }

        private fun decode(packet: HuddleRemoteOpusPacket) {
            val inputIndex = decoder.dequeueInputBuffer(CODEC_DEQUEUE_TIMEOUT_US)
            check(inputIndex >= 0) { "Opus decoder did not provide an input buffer." }
            val input = checkNotNull(decoder.getInputBuffer(inputIndex)) {
                "Opus decoder input buffer was unavailable."
            }
            input.clear()
            input.put(packet.opus)
            decoder.queueInputBuffer(
                inputIndex,
                0,
                packet.opus.size,
                timestamp48kToPresentationTime(packet.timestamp48k),
                0,
            )
            drainOutput()
        }

        private fun drainOutput() {
            val info = MediaCodec.BufferInfo()
            while (true) {
                val outputIndex = decoder.dequeueOutputBuffer(info, CODEC_DEQUEUE_TIMEOUT_US)
                when {
                    outputIndex == MediaCodec.INFO_TRY_AGAIN_LATER -> return
                    outputIndex == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> continue
                    outputIndex >= 0 -> {
                        try {
                            if (info.size <= 0) continue
                            val output = checkNotNull(decoder.getOutputBuffer(outputIndex))
                            output.position(info.offset)
                            output.limit(info.offset + info.size)
                            val pcm = ByteArray(info.size)
                            output.get(pcm)
                            var offset = 0
                            while (offset < pcm.size) {
                                val written = track.write(
                                    pcm,
                                    offset,
                                    pcm.size - offset,
                                    AudioTrack.WRITE_BLOCKING,
                                )
                                check(written > 0) { "AudioTrack write failed with code $written." }
                                offset += written
                            }
                        } finally {
                            decoder.releaseOutputBuffer(outputIndex, false)
                        }
                    }
                }
            }
        }

        fun release() {
            runCatching { track.pause() }
            runCatching { track.flush() }
            runCatching { track.release() }
            runCatching { decoder.stop() }
            runCatching { decoder.release() }
        }
    }

    internal companion object {
        private const val SAMPLE_RATE_HZ = 48_000
        private const val CHANNELS = 1
        private const val FRAME_SAMPLES = 960
        private const val BYTES_PER_SAMPLE = 2
        private const val FRAME_BYTES = FRAME_SAMPLES * BYTES_PER_SAMPLE
        private const val FRAME_DURATION_US = 20_000L
        private const val BIT_RATE = 32_000
        private const val CODEC_DEQUEUE_TIMEOUT_US = 10_000L
        private const val PLAYBACK_QUEUE_CAPACITY = 50
        private const val PER_PEER_JITTER_CAPACITY = 10
        private const val JITTER_START_PACKETS = 3
        private const val MAX_REMOTE_PEERS = 15
        private const val STOP_JOIN_TIMEOUT_MS = 750L
        private const val INTERRUPTION_WAIT_MS = 250L
        private const val SILENT_LEVEL_DBOV = -127
        private const val OPUS_SEEK_PREROLL_NS = 80_000_000L
        private const val DIAGNOSTIC_EMIT_FRAME_INTERVAL = 50L

        fun isSupported(): Boolean {
            val codecs = runCatching {
                MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.toList()
            }.getOrElse { return false }
            val hasEncoder = codecs.any { codec ->
                codec.isEncoder && codec.supportedTypes.any(MediaFormat.MIMETYPE_AUDIO_OPUS::equals)
            }
            val hasDecoder = codecs.any { codec ->
                !codec.isEncoder && codec.supportedTypes.any(MediaFormat.MIMETYPE_AUDIO_OPUS::equals)
            }
            return hasEncoder && hasDecoder
        }

        private fun createAudioRecord(): AudioRecord {
            val minimum = AudioRecord.getMinBufferSize(
                SAMPLE_RATE_HZ,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
            )
            check(minimum > 0) { "Android does not support 48 kHz mono microphone capture." }
            val record = AudioRecord.Builder()
                .setAudioSource(MediaRecorder.AudioSource.VOICE_COMMUNICATION)
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(SAMPLE_RATE_HZ)
                        .setChannelMask(AudioFormat.CHANNEL_IN_MONO)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build(),
                )
                .setBufferSizeInBytes(max(minimum, FRAME_BYTES * 8))
                .build()
            if (record.state != AudioRecord.STATE_INITIALIZED) {
                record.release()
                error("Android microphone capture could not be initialized.")
            }
            return record
        }

        private fun createEncoder(): MediaCodec {
            val format = MediaFormat.createAudioFormat(
                MediaFormat.MIMETYPE_AUDIO_OPUS,
                SAMPLE_RATE_HZ,
                CHANNELS,
            ).apply {
                setInteger(MediaFormat.KEY_BIT_RATE, BIT_RATE)
                setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, FRAME_BYTES)
                setInteger(MediaFormat.KEY_PCM_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
            }
            val encoder = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS)
            try {
                encoder.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                return encoder
            } catch (error: Throwable) {
                runCatching { encoder.release() }
                throw error
            }
        }

        private fun createDecoder(): MediaCodec {
            val format = MediaFormat.createAudioFormat(
                MediaFormat.MIMETYPE_AUDIO_OPUS,
                SAMPLE_RATE_HZ,
                CHANNELS,
            ).apply {
                setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 4096)
                setInteger(MediaFormat.KEY_PCM_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
                setByteBuffer("csd-0", ByteBuffer.wrap(opusIdentificationHeader()))
                setByteBuffer("csd-1", nativeLongBuffer(0))
                setByteBuffer("csd-2", nativeLongBuffer(OPUS_SEEK_PREROLL_NS))
            }
            val decoder = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS)
            try {
                decoder.configure(format, null, null, 0)
                return decoder
            } catch (error: Throwable) {
                runCatching { decoder.release() }
                throw error
            }
        }

        private fun createAudioTrack(): AudioTrack {
            val minimum = AudioTrack.getMinBufferSize(
                SAMPLE_RATE_HZ,
                AudioFormat.CHANNEL_OUT_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
            )
            check(minimum > 0) { "Android does not support 48 kHz mono audio output." }
            val track = AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(SAMPLE_RATE_HZ)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build(),
                )
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(max(minimum, FRAME_BYTES * 8))
                .build()
            if (track.state != AudioTrack.STATE_INITIALIZED) {
                track.release()
                error("Android Huddle audio output could not be initialized.")
            }
            return track
        }

        private fun opusIdentificationHeader(): ByteArray = byteArrayOf(
            'O'.code.toByte(),
            'p'.code.toByte(),
            'u'.code.toByte(),
            's'.code.toByte(),
            'H'.code.toByte(),
            'e'.code.toByte(),
            'a'.code.toByte(),
            'd'.code.toByte(),
            1,
            CHANNELS.toByte(),
            0,
            0,
            0x80.toByte(),
            0xbb.toByte(),
            0,
            0,
            0,
            0,
            0,
        )

        private fun nativeLongBuffer(value: Long): ByteBuffer =
            ByteBuffer.allocate(java.lang.Long.BYTES)
                .order(ByteOrder.nativeOrder())
                .putLong(value)
                .apply { flip() }

        private fun audioLevelDbov(samples: ShortArray): Int {
            var squareSum = 0.0
            for (sample in samples) {
                val normalized = sample.toDouble() / Short.MAX_VALUE.toDouble()
                squareSum += normalized * normalized
            }
            val meanSquare = squareSum / samples.size
            if (meanSquare <= 0.0) return SILENT_LEVEL_DBOV
            val dbov = 10.0 * log10(meanSquare)
            return dbov.roundToInt().coerceIn(SILENT_LEVEL_DBOV, 0)
        }

        private fun audioPeakDbov(samples: ShortArray): Int {
            var peak = 0
            for (sample in samples) {
                peak = max(peak, kotlin.math.abs(sample.toInt()))
            }
            if (peak == 0) return SILENT_LEVEL_DBOV
            val normalized = peak.toDouble() / Short.MAX_VALUE.toDouble()
            return (20.0 * log10(normalized))
                .roundToInt()
                .coerceIn(SILENT_LEVEL_DBOV, 0)
        }

        private fun captureDiagnostics(
            record: AudioRecord,
            echoCanceler: AcousticEchoCanceler?,
            noiseSuppressor: NoiseSuppressor?,
            automaticGainControl: AutomaticGainControl?,
            frameCount: Long,
            rmsDbovHistogram: Map<Int, Long>,
            peakDbovHistogram: Map<Int, Long>,
            maxPeakDbov: Int,
        ): HuddleCaptureDiagnostics {
            val activeConfiguration = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                runCatching { record.activeRecordingConfiguration }.getOrNull()
            } else {
                null
            }
            val device = activeConfiguration?.audioDevice
                ?: runCatching { record.routedDevice }.getOrNull()
            val source = activeConfiguration?.audioSource
                ?: MediaRecorder.AudioSource.VOICE_COMMUNICATION
            return HuddleCaptureDiagnostics(
                frameCount = frameCount,
                rmsDbovHistogram = numericHistogram(rmsDbovHistogram),
                peakDbovHistogram = numericHistogram(peakDbovHistogram),
                maxPeakDbov = maxPeakDbov,
                audioSource = source,
                audioSourceLabel = audioSourceLabel(source),
                deviceId = device?.id,
                deviceType = device?.type,
                deviceLabel = device?.productName?.toString()?.trim()?.takeIf { it.isNotEmpty() },
                acousticEchoCancelerAvailable = AcousticEchoCanceler.isAvailable(),
                acousticEchoCancelerEnabled = echoCanceler?.let {
                    runCatching { it.enabled }.getOrNull()
                },
                noiseSuppressorAvailable = NoiseSuppressor.isAvailable(),
                noiseSuppressorEnabled = noiseSuppressor?.let {
                    runCatching { it.enabled }.getOrNull()
                },
                automaticGainControlAvailable = AutomaticGainControl.isAvailable(),
                automaticGainControlEnabled = automaticGainControl?.let {
                    runCatching { it.enabled }.getOrNull()
                },
            )
        }

        private fun numericHistogram(values: Map<Int, Long>): Map<String, Long> =
            values.entries
                .sortedBy { it.key }
                .associate { (level, count) -> level.toString() to count }

        private fun audioSourceLabel(source: Int): String = when (source) {
            MediaRecorder.AudioSource.VOICE_COMMUNICATION -> "voice_communication"
            MediaRecorder.AudioSource.MIC -> "microphone"
            MediaRecorder.AudioSource.VOICE_RECOGNITION -> "voice_recognition"
            MediaRecorder.AudioSource.UNPROCESSED -> "unprocessed"
            MediaRecorder.AudioSource.VOICE_PERFORMANCE -> "voice_performance"
            else -> "source_$source"
        }

        private fun presentationTimeTo48k(presentationTimeUs: Long): Long =
            ((presentationTimeUs * SAMPLE_RATE_HZ) / 1_000_000L) and 0xffff_ffffL

        private fun timestamp48kToPresentationTime(timestamp48k: Long): Long =
            (timestamp48k * 1_000_000L) / SAMPLE_RATE_HZ
    }
}
