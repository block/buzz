import AVFoundation
import Foundation

struct HuddleLocalOpusPacket {
  let sequence: Int
  let timestamp48k: Int
  let levelDbov: Int
  let flags: Int
  let opus: Data
}

struct HuddleRemoteOpusPacket {
  let peerIndex: Int
  let sequence: Int
  let timestamp48k: Int64
  let levelDbov: Int
  let opus: Data
}

struct HuddlePacketJitterQueue {
  private(set) var packets: [HuddleRemoteOpusPacket] = []
  private var lastDrainedSequence: Int?
  private var started = false
  let capacity: Int
  let startPackets: Int

  init(capacity: Int, startPackets: Int) {
    self.capacity = capacity
    self.startPackets = startPackets
  }

  mutating func enqueue(_ packet: HuddleRemoteOpusPacket) {
    if let last = lastDrainedSequence, !sequenceAfter(packet.sequence, last) {
      return
    }
    guard !packets.contains(where: { $0.sequence == packet.sequence }) else {
      return
    }
    let index = packets.firstIndex {
      sequenceBefore(packet.sequence, $0.sequence)
    } ?? packets.endIndex
    packets.insert(packet, at: index)
    if packets.count > capacity { packets.removeFirst() }
    if packets.count >= startPackets { started = true }
  }

  mutating func drainOne() -> HuddleRemoteOpusPacket? {
    guard started, !packets.isEmpty else { return nil }
    let packet = packets.removeFirst()
    lastDrainedSequence = packet.sequence
    return packet
  }

  private func sequenceAfter(_ sequence: Int, _ reference: Int) -> Bool {
    let distance = (sequence - reference) & 0xffff
    return distance > 0 && distance <= 0x7fff
  }

  private func sequenceBefore(_ left: Int, _ right: Int) -> Bool {
    sequenceAfter(right, left)
  }
}

struct HuddleCaptureDiagnostics {
  let frameCount: Int64
  let rmsDbovHistogram: [String: Int64]
  let peakDbovHistogram: [String: Int64]
  let maxPeakDbov: Int
  let deviceLabel: String?
  let voiceProcessingEnabled: Bool
}

enum HuddleNativeMediaError: LocalizedError {
  case invalidState(String)
  case unsupported(String)
  case conversion(String)
  case malformedPacket(String)

  var errorDescription: String? {
    switch self {
    case .invalidState(let message),
      .unsupported(let message),
      .conversion(let message),
      .malformedPacket(let message):
      message
    }
  }
}

enum HuddleAudioFormats {
  static let sampleRate = 48_000.0
  static let channels: AVAudioChannelCount = 1
  static let frameSamples = 960
  static let maximumOpusPacketBytes = 4_088

  static func makePCM() throws -> AVAudioFormat {
    guard
      let format = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: sampleRate,
        channels: channels,
        interleaved: false
      )
    else {
      throw HuddleNativeMediaError.unsupported(
        "iOS does not expose 48 kHz mono PCM audio."
      )
    }
    return format
  }

  static func makeOpus() throws -> AVAudioFormat {
    var description = AudioStreamBasicDescription(
      mSampleRate: sampleRate,
      mFormatID: kAudioFormatOpus,
      mFormatFlags: 0,
      mBytesPerPacket: 0,
      mFramesPerPacket: UInt32(frameSamples),
      mBytesPerFrame: 0,
      mChannelsPerFrame: channels,
      mBitsPerChannel: 0,
      mReserved: 0
    )
    guard let format = AVAudioFormat(streamDescription: &description) else {
      throw HuddleNativeMediaError.unsupported(
        "iOS does not expose the native Opus audio format."
      )
    }
    return format
  }
}

final class HuddleOpusEncoder {
  private let pcmFormat: AVAudioFormat
  private let opusFormat: AVAudioFormat
  private let converter: AVAudioConverter
  private let maximumPacketSize: Int

  init() throws {
    let pcmFormat = try HuddleAudioFormats.makePCM()
    let opusFormat = try HuddleAudioFormats.makeOpus()
    guard let converter = AVAudioConverter(from: pcmFormat, to: opusFormat) else {
      throw HuddleNativeMediaError.unsupported(
        "This iOS device does not expose an Opus encoder."
      )
    }
    converter.bitRate = 32_000
    converter.primeMethod = .none
    self.pcmFormat = pcmFormat
    self.opusFormat = opusFormat
    self.converter = converter
    maximumPacketSize = max(1_275, converter.maximumOutputPacketSize)
  }

  func encode(_ samples: [Float]) throws -> Data {
    guard samples.count == HuddleAudioFormats.frameSamples else {
      throw HuddleNativeMediaError.conversion(
        "Opus input must contain exactly 960 samples."
      )
    }
    guard
      let pcm = AVAudioPCMBuffer(
        pcmFormat: pcmFormat,
        frameCapacity: AVAudioFrameCount(HuddleAudioFormats.frameSamples)
      ),
      let channel = pcm.floatChannelData?.pointee
    else {
      throw HuddleNativeMediaError.conversion(
        "Unable to allocate the iOS Opus input buffer."
      )
    }
    pcm.frameLength = AVAudioFrameCount(HuddleAudioFormats.frameSamples)
    for index in samples.indices {
      channel[index] = samples[index]
    }

    let compressed = AVAudioCompressedBuffer(
      format: opusFormat,
      packetCapacity: 1,
      maximumPacketSize: maximumPacketSize
    )
    var suppliedInput = false
    var conversionError: NSError?
    let status = converter.convert(
      to: compressed,
      error: &conversionError
    ) { _, inputStatus in
      guard !suppliedInput else {
        inputStatus.pointee = .noDataNow
        return nil
      }
      suppliedInput = true
      inputStatus.pointee = .haveData
      return pcm
    }
    if status == .error || conversionError != nil {
      throw HuddleNativeMediaError.conversion(
        conversionError?.localizedDescription ?? "iOS Opus encoding failed."
      )
    }
    let byteCount = Int(compressed.byteLength)
    guard
      compressed.packetCount == 1,
      byteCount > 0,
      byteCount <= HuddleAudioFormats.maximumOpusPacketBytes
    else {
      throw HuddleNativeMediaError.conversion(
        "iOS produced an invalid Opus packet."
      )
    }
    return Data(bytes: compressed.data, count: byteCount)
  }
}

final class HuddleOpusDecoder {
  private let pcmFormat: AVAudioFormat
  private let opusFormat: AVAudioFormat
  private let converter: AVAudioConverter

  init() throws {
    let pcmFormat = try HuddleAudioFormats.makePCM()
    let opusFormat = try HuddleAudioFormats.makeOpus()
    guard let converter = AVAudioConverter(from: opusFormat, to: pcmFormat) else {
      throw HuddleNativeMediaError.unsupported(
        "This iOS device does not expose an Opus decoder."
      )
    }
    converter.primeMethod = .none
    self.pcmFormat = pcmFormat
    self.opusFormat = opusFormat
    self.converter = converter
  }

  func decode(_ packet: Data) throws -> AVAudioPCMBuffer {
    guard
      !packet.isEmpty,
      packet.count <= HuddleAudioFormats.maximumOpusPacketBytes
    else {
      throw HuddleNativeMediaError.malformedPacket(
        "Remote Huddle Opus packet has an invalid length."
      )
    }
    let compressed = AVAudioCompressedBuffer(
      format: opusFormat,
      packetCapacity: 1,
      maximumPacketSize: HuddleAudioFormats.maximumOpusPacketBytes
    )
    packet.withUnsafeBytes { bytes in
      guard let baseAddress = bytes.baseAddress else { return }
      compressed.data.copyMemory(from: baseAddress, byteCount: packet.count)
    }
    compressed.byteLength = UInt32(packet.count)
    compressed.packetCount = 1
    if let descriptions = compressed.packetDescriptions {
      descriptions.pointee = AudioStreamPacketDescription(
        mStartOffset: 0,
        mVariableFramesInPacket: UInt32(HuddleAudioFormats.frameSamples),
        mDataByteSize: UInt32(packet.count)
      )
    }

    guard
      let pcm = AVAudioPCMBuffer(
        pcmFormat: pcmFormat,
        frameCapacity: AVAudioFrameCount(HuddleAudioFormats.frameSamples)
      )
    else {
      throw HuddleNativeMediaError.conversion(
        "Unable to allocate the iOS Opus output buffer."
      )
    }
    var suppliedInput = false
    var conversionError: NSError?
    let status = converter.convert(
      to: pcm,
      error: &conversionError
    ) { _, inputStatus in
      guard !suppliedInput else {
        inputStatus.pointee = .noDataNow
        return nil
      }
      suppliedInput = true
      inputStatus.pointee = .haveData
      return compressed
    }
    if status == .error || conversionError != nil || pcm.frameLength == 0 {
      throw HuddleNativeMediaError.conversion(
        conversionError?.localizedDescription ?? "iOS Opus decoding failed."
      )
    }
    return pcm
  }
}

enum HuddleAudioLevels {
  static func rmsDbov(_ samples: [Float]) -> Int {
    guard !samples.isEmpty else { return -127 }
    let squareSum = samples.reduce(0.0) { partial, sample in
      partial + Double(sample * sample)
    }
    let meanSquare = squareSum / Double(samples.count)
    guard meanSquare > 0 else { return -127 }
    return min(0, max(-127, Int((10 * log10(meanSquare)).rounded())))
  }

  static func peakDbov(_ samples: [Float]) -> Int {
    let peak = samples.reduce(Float.zero) { max($0, abs($1)) }
    guard peak > 0 else { return -127 }
    return min(0, max(-127, Int((20 * log10(Double(peak))).rounded())))
  }
}

struct HuddleTalkerSelection: Equatable {
  let accepted: Bool
  let evictedPeerIndex: Int?

  func allocateIfAccepted<T>(_ allocate: () throws -> T) rethrows -> T? {
    guard accepted else { return nil }
    return try allocate()
  }
}

/// Fixed-capacity active-talker selector. The UI roster is independent; native
/// decoder/player resources exist only for peers that actually send packets.
struct HuddleActiveTalkerSelector {
  struct Activity {
    let peerIndex: Int
    let lastPacketOrdinal: UInt64
    let lastPacketAt: TimeInterval
    let levelDbov: Int
  }

  let capacity: Int
  let now: () -> TimeInterval
  let inactivityTimeout: TimeInterval
  private(set) var active: [Int: Activity] = [:]
  private var ordinal: UInt64 = 0

  init(
    capacity: Int,
    now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
    inactivityTimeout: TimeInterval = 1
  ) {
    self.capacity = capacity
    self.now = now
    self.inactivityTimeout = inactivityTimeout
  }

  mutating func activate(peerIndex: Int, levelDbov: Int) -> HuddleTalkerSelection {
    ordinal &+= 1
    let packetAt = now()
    if active[peerIndex] != nil {
      active[peerIndex] = Activity(
        peerIndex: peerIndex,
        lastPacketOrdinal: ordinal,
        lastPacketAt: packetAt,
        levelDbov: levelDbov
      )
      return HuddleTalkerSelection(accepted: true, evictedPeerIndex: nil)
    }
    let weakest = active.count >= capacity
      ? active.values.min { left, right in
        if left.levelDbov != right.levelDbov {
          return left.levelDbov < right.levelDbov
        }
        if left.lastPacketOrdinal != right.lastPacketOrdinal {
          return left.lastPacketOrdinal < right.lastPacketOrdinal
        }
        return left.peerIndex < right.peerIndex
      }
      : nil
    let expired = active.values
      .filter { packetAt - $0.lastPacketAt >= inactivityTimeout }
      .min { left, right in
        if left.lastPacketAt != right.lastPacketAt {
          return left.lastPacketAt < right.lastPacketAt
        }
        return left.peerIndex < right.peerIndex
      }
    // Equal-level packets (especially continuous -127 dBov silence) must not
    // churn decoder/jitter state. Admit a new peer when any selected slot is
    // inactive or the new packet is louder than the quietest selected peer.
    if let weakest, expired == nil, levelDbov <= weakest.levelDbov {
      return HuddleTalkerSelection(accepted: false, evictedPeerIndex: nil)
    }
    let evicted = expired?.peerIndex ?? weakest?.peerIndex
    if let evicted { active.removeValue(forKey: evicted) }
    active[peerIndex] = Activity(
      peerIndex: peerIndex,
      lastPacketOrdinal: ordinal,
      lastPacketAt: packetAt,
      levelDbov: levelDbov
    )
    return HuddleTalkerSelection(accepted: true, evictedPeerIndex: evicted)
  }

  mutating func remove(_ peerIndex: Int) {
    active.removeValue(forKey: peerIndex)
  }

  mutating func removeAll() {
    active.removeAll()
    ordinal = 0
  }
}

/// Foreground-only iOS realtime media engine for the fixed Huddle Opus v2 path.
///
/// AVAudioEngine owns voice-processed capture and per-peer mixed playout.
/// AVAudioConverter keeps PCM native and moves only Opus packets across Flutter.
final class HuddleAudioEngine {
  private let onLocalPacket: (HuddleLocalOpusPacket) -> Void
  private let onFailure: (String, String) -> Void
  private let onDiagnostics: (HuddleCaptureDiagnostics) -> Void
  private let diagnosticsEnabled: Bool
  private let audioEngine = AVAudioEngine()
  private let processingQueue = DispatchQueue(
    label: "xyz.block.buzz.huddle.ios-media",
    qos: .userInteractive
  )
  private let stateLock = NSLock()
  private let pcmFormat: AVAudioFormat
  private let encoder: HuddleOpusEncoder

  private var running = false
  private var muted = false
  private var interrupted = false
  private var failureReported = false
  private var pendingCaptureBuffers = 0
  private var pendingRemotePackets = 0
  private var captureConverter: AVAudioConverter?
  private var captureSourceFormat: AVAudioFormat?
  private var captureTapInstalled = false
  private var captureSamples: [Float] = []
  private var captureSampleOffset = 0
  private var sequence = 0
  private var frameIndex: Int64 = 0
  private var diagnosticFrameCount: Int64 = 0
  private var diagnosticMaxPeakDbov = -127
  private var rmsDbovHistogram: [Int: Int64] = [:]
  private var peakDbovHistogram: [Int: Int64] = [:]
  private var peerPlaybacks: [Int: HuddlePeerPlayback] = [:]
  private var activeTalkers = HuddleActiveTalkerSelector(capacity: 15)
  private var playbackTimer: DispatchSourceTimer?
  private var configurationObserver: NSObjectProtocol?

  init(
    onLocalPacket: @escaping (HuddleLocalOpusPacket) -> Void,
    onFailure: @escaping (String, String) -> Void,
    onDiagnostics: @escaping (HuddleCaptureDiagnostics) -> Void,
    diagnosticsEnabled: Bool
  ) throws {
    pcmFormat = try HuddleAudioFormats.makePCM()
    encoder = try HuddleOpusEncoder()
    self.onLocalPacket = onLocalPacket
    self.onFailure = onFailure
    self.onDiagnostics = onDiagnostics
    self.diagnosticsEnabled = diagnosticsEnabled
    configurationObserver = NotificationCenter.default.addObserver(
      forName: .AVAudioEngineConfigurationChange,
      object: audioEngine,
      queue: nil
    ) { [weak self] _ in
      self?.processingQueue.async {
        self?.restartAfterConfigurationChange()
      }
    }
  }

  deinit {
    stop()
    if let configurationObserver {
      NotificationCenter.default.removeObserver(configurationObserver)
    }
  }

  static func isSupported() -> Bool {
    do {
      _ = try HuddleOpusEncoder()
      _ = try HuddleOpusDecoder()
      return true
    } catch {
      return false
    }
  }

  func start() throws {
    var startError: Error?
    processingQueue.sync {
      do {
        try startOnQueue()
      } catch {
        startError = error
      }
    }
    if let startError { throw startError }
  }

  func setMuted(_ value: Bool) throws {
    stateLock.lock()
    defer { stateLock.unlock() }
    guard running else {
      throw HuddleNativeMediaError.invalidState(
        "Huddle audio is not running."
      )
    }
    muted = value
  }

  func setInterrupted(_ value: Bool) {
    stateLock.lock()
    let shouldUpdate = running
    interrupted = value
    stateLock.unlock()
    guard shouldUpdate else { return }
    processingQueue.async { [weak self] in
      guard let self else { return }
      if value {
        for playback in peerPlaybacks.values {
          playback.pause()
        }
        return
      }
      do {
        if !audioEngine.isRunning {
          try audioEngine.start()
        }
        for playback in peerPlaybacks.values {
          playback.resume()
        }
      } catch {
        reportFailure(code: "audio_resume_failed", error: error)
      }
    }
  }

  func enqueueRemote(_ packet: HuddleRemoteOpusPacket) throws {
    stateLock.lock()
    guard running else {
      stateLock.unlock()
      throw HuddleNativeMediaError.invalidState(
        "Huddle audio is not running."
      )
    }
    guard pendingRemotePackets < 50 else {
      stateLock.unlock()
      return
    }
    pendingRemotePackets += 1
    stateLock.unlock()
    processingQueue.async { [weak self] in
      guard let self else { return }
      defer { completeRemotePacket() }
      do {
        let selection = activeTalkers.activate(
          peerIndex: packet.peerIndex,
          levelDbov: packet.levelDbov
        )
        if let evictedIndex = selection.evictedPeerIndex,
          let evicted = peerPlaybacks.removeValue(forKey: evictedIndex)
        {
          evicted.release(from: audioEngine)
        }
        let playback: HuddlePeerPlayback
        if let existing = peerPlaybacks[packet.peerIndex] {
          playback = existing
        } else if let allocated = try selection.allocateIfAccepted({
          try HuddlePeerPlayback(
            engine: self.audioEngine,
            pcmFormat: self.pcmFormat
          )
        }) {
          playback = allocated
          peerPlaybacks[packet.peerIndex] = allocated
        } else {
          return
        }
        playback.enqueue(packet)
      } catch {
        reportFailure(code: "playback_failed", error: error)
      }
    }
  }

  private func completeRemotePacket() {
    stateLock.lock()
    pendingRemotePackets = max(0, pendingRemotePackets - 1)
    stateLock.unlock()
  }

  func removeRemotePeer(_ peerIndex: Int) {
    processingQueue.async { [weak self] in
      guard let self else { return }
      activeTalkers.remove(peerIndex)
      guard let playback = peerPlaybacks.removeValue(forKey: peerIndex)
      else { return }
      playback.release(from: audioEngine)
    }
  }

  func stop() {
    processingQueue.sync {
      stopOnQueue()
    }
  }

  private func startOnQueue() throws {
    stateLock.lock()
    let wasRunning = running
    if !wasRunning {
      running = true
      muted = false
      interrupted = false
      failureReported = false
    }
    stateLock.unlock()
    guard !wasRunning else {
      throw HuddleNativeMediaError.invalidState(
        "Huddle audio is already running."
      )
    }

    do {
      let input = audioEngine.inputNode
      try input.setVoiceProcessingEnabled(true)
      let inputFormat = input.outputFormat(forBus: 0)
      guard inputFormat.sampleRate > 0, inputFormat.channelCount > 0 else {
        throw HuddleNativeMediaError.unsupported(
          "The iOS microphone does not expose an input format."
        )
      }
      input.installTap(
        onBus: 0,
        bufferSize: AVAudioFrameCount(HuddleAudioFormats.frameSamples),
        format: inputFormat
      ) { [weak self] buffer, _ in
        self?.acceptCapturedBuffer(buffer)
      }
      captureTapInstalled = true
      audioEngine.prepare()
      try audioEngine.start()
      startPlaybackTimer()
      if diagnosticsEnabled {
        onDiagnostics(makeDiagnostics())
      }
    } catch {
      stopOnQueue()
      throw error
    }
  }

  private func stopOnQueue() {
    stateLock.lock()
    running = false
    muted = false
    interrupted = false
    pendingCaptureBuffers = 0
    pendingRemotePackets = 0
    stateLock.unlock()

    playbackTimer?.cancel()
    playbackTimer = nil
    if captureTapInstalled {
      audioEngine.inputNode.removeTap(onBus: 0)
      captureTapInstalled = false
    }
    for playback in peerPlaybacks.values {
      playback.release(from: audioEngine)
    }
    peerPlaybacks.removeAll()
    activeTalkers.removeAll()
    audioEngine.stop()
    audioEngine.reset()
    try? audioEngine.inputNode.setVoiceProcessingEnabled(false)
    captureConverter = nil
    captureSourceFormat = nil
    captureSamples.removeAll(keepingCapacity: false)
    captureSampleOffset = 0
    sequence = 0
    frameIndex = 0
    diagnosticFrameCount = 0
    diagnosticMaxPeakDbov = -127
    rmsDbovHistogram.removeAll()
    peakDbovHistogram.removeAll()
  }

  private func acceptCapturedBuffer(_ buffer: AVAudioPCMBuffer) {
    stateLock.lock()
    guard running, pendingCaptureBuffers < 8 else {
      stateLock.unlock()
      return
    }
    pendingCaptureBuffers += 1
    stateLock.unlock()

    guard let copied = copyPCMBuffer(buffer) else {
      completeCaptureBuffer()
      return
    }
    processingQueue.async { [weak self] in
      guard let self else { return }
      defer { completeCaptureBuffer() }
      do {
        try processCapturedBuffer(copied)
      } catch {
        reportFailure(code: "capture_failed", error: error)
      }
    }
  }

  private func completeCaptureBuffer() {
    stateLock.lock()
    pendingCaptureBuffers = max(0, pendingCaptureBuffers - 1)
    stateLock.unlock()
  }

  private func processCapturedBuffer(_ input: AVAudioPCMBuffer) throws {
    stateLock.lock()
    let isRunning = running
    stateLock.unlock()
    guard isRunning else { return }

    let normalized = try normalizeCapturedBuffer(input)
    guard let channel = normalized.floatChannelData?.pointee else {
      throw HuddleNativeMediaError.conversion(
        "iOS microphone PCM data is unavailable."
      )
    }
    captureSamples.append(
      contentsOf: UnsafeBufferPointer(
        start: channel,
        count: Int(normalized.frameLength)
      )
    )

    while captureSamples.count - captureSampleOffset >= HuddleAudioFormats.frameSamples {
      let end = captureSampleOffset + HuddleAudioFormats.frameSamples
      let frame = Array(captureSamples[captureSampleOffset..<end])
      captureSampleOffset = end
      try processCaptureFrame(frame)
    }
    if captureSampleOffset >= HuddleAudioFormats.frameSamples * 4 {
      captureSamples.removeFirst(captureSampleOffset)
      captureSampleOffset = 0
    }
  }

  private func normalizeCapturedBuffer(
    _ input: AVAudioPCMBuffer
  ) throws -> AVAudioPCMBuffer {
    if matchesTargetPCM(input.format) {
      return input
    }
    if captureSourceFormat?.isEqual(input.format) != true {
      guard let converter = AVAudioConverter(from: input.format, to: pcmFormat) else {
        throw HuddleNativeMediaError.unsupported(
          "iOS cannot convert the active microphone route to Huddle audio."
        )
      }
      converter.primeMethod = .none
      captureConverter = converter
      captureSourceFormat = input.format
    }
    guard let converter = captureConverter else {
      throw HuddleNativeMediaError.conversion(
        "The iOS microphone converter is unavailable."
      )
    }
    let ratio = HuddleAudioFormats.sampleRate / input.format.sampleRate
    let capacity = max(
      HuddleAudioFormats.frameSamples,
      Int(ceil(Double(input.frameLength) * ratio)) + 64
    )
    guard
      let output = AVAudioPCMBuffer(
        pcmFormat: pcmFormat,
        frameCapacity: AVAudioFrameCount(capacity)
      )
    else {
      throw HuddleNativeMediaError.conversion(
        "Unable to allocate converted iOS microphone audio."
      )
    }
    var suppliedInput = false
    var conversionError: NSError?
    let status = converter.convert(
      to: output,
      error: &conversionError
    ) { _, inputStatus in
      guard !suppliedInput else {
        inputStatus.pointee = .noDataNow
        return nil
      }
      suppliedInput = true
      inputStatus.pointee = .haveData
      return input
    }
    if status == .error || conversionError != nil {
      throw HuddleNativeMediaError.conversion(
        conversionError?.localizedDescription
          ?? "iOS microphone conversion failed."
      )
    }
    return output
  }

  private func processCaptureFrame(_ frame: [Float]) throws {
    stateLock.lock()
    let shouldSend = running && !muted && !interrupted
    stateLock.unlock()
    guard shouldSend else { return }

    let level = HuddleAudioLevels.rmsDbov(frame)
    if diagnosticsEnabled {
      let peak = HuddleAudioLevels.peakDbov(frame)
      diagnosticFrameCount += 1
      diagnosticMaxPeakDbov = max(diagnosticMaxPeakDbov, peak)
      rmsDbovHistogram[level, default: 0] += 1
      peakDbovHistogram[peak, default: 0] += 1
      if diagnosticFrameCount % 50 == 0 {
        onDiagnostics(makeDiagnostics())
      }
    }

    let opus = try encoder.encode(frame)
    let packet = HuddleLocalOpusPacket(
      sequence: sequence,
      timestamp48k: Int(
        UInt32(truncatingIfNeeded: frameIndex * Int64(HuddleAudioFormats.frameSamples))
      ),
      levelDbov: level,
      flags: 0,
      opus: opus
    )
    sequence = (sequence + 1) & 0xffff
    frameIndex += 1
    onLocalPacket(packet)
  }

  private func startPlaybackTimer() {
    let timer = DispatchSource.makeTimerSource(queue: processingQueue)
    timer.schedule(
      deadline: .now() + .milliseconds(20),
      repeating: .milliseconds(20),
      leeway: .milliseconds(2)
    )
    timer.setEventHandler { [weak self] in
      self?.drainPlaybackTick()
    }
    playbackTimer = timer
    timer.resume()
  }

  private func drainPlaybackTick() {
    stateLock.lock()
    let shouldPlay = running && !interrupted
    stateLock.unlock()
    guard shouldPlay else { return }
    do {
      for playback in peerPlaybacks.values {
        try playback.drainOne()
      }
    } catch {
      reportFailure(code: "playback_failed", error: error)
    }
  }

  private func restartAfterConfigurationChange() {
    stateLock.lock()
    let shouldRestart = running && !interrupted
    stateLock.unlock()
    guard shouldRestart else { return }
    do {
      if !audioEngine.isRunning {
        try audioEngine.start()
      }
      for playback in peerPlaybacks.values {
        playback.resume()
      }
    } catch {
      reportFailure(code: "audio_route_failed", error: error)
    }
  }

  private func makeDiagnostics() -> HuddleCaptureDiagnostics {
    let route = AVAudioSession.sharedInstance().currentRoute
    return HuddleCaptureDiagnostics(
      frameCount: diagnosticFrameCount,
      rmsDbovHistogram: numericHistogram(rmsDbovHistogram),
      peakDbovHistogram: numericHistogram(peakDbovHistogram),
      maxPeakDbov: diagnosticMaxPeakDbov,
      deviceLabel: route.inputs.first?.portName,
      voiceProcessingEnabled: audioEngine.inputNode.isVoiceProcessingEnabled
    )
  }

  private func reportFailure(code: String, error: Error) {
    stateLock.lock()
    guard running, !failureReported else {
      stateLock.unlock()
      return
    }
    let message =
      (error as? LocalizedError)?.errorDescription
      ?? error.localizedDescription
    failureReported = true
    stateLock.unlock()
    // Fail closed on the native queue. Flutter will perform the durable session
    // teardown too, but microphone and playout lifetime must not depend on a
    // platform-channel callback making a successful round trip.
    stopOnQueue()
    onFailure(code, message)
  }

  private func matchesTargetPCM(_ format: AVAudioFormat) -> Bool {
    format.commonFormat == .pcmFormatFloat32
      && format.sampleRate == HuddleAudioFormats.sampleRate
      && format.channelCount == HuddleAudioFormats.channels
      && !format.isInterleaved
  }

  private func numericHistogram(_ values: [Int: Int64]) -> [String: Int64] {
    Dictionary(uniqueKeysWithValues: values.map { (String($0.key), $0.value) })
  }

  private func copyPCMBuffer(_ source: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
    guard
      let copy = AVAudioPCMBuffer(
        pcmFormat: source.format,
        frameCapacity: source.frameLength
      )
    else { return nil }
    copy.frameLength = source.frameLength
    let sourceBuffers = UnsafeMutableAudioBufferListPointer(
      source.mutableAudioBufferList
    )
    let destinationBuffers = UnsafeMutableAudioBufferListPointer(
      copy.mutableAudioBufferList
    )
    guard sourceBuffers.count == destinationBuffers.count else { return nil }
    for index in sourceBuffers.indices {
      let sourceBuffer = sourceBuffers[index]
      let destinationBuffer = destinationBuffers[index]
      guard
        let sourceData = sourceBuffer.mData,
        let destinationData = destinationBuffer.mData
      else { return nil }
      let byteCount = Int(sourceBuffer.mDataByteSize)
      memcpy(destinationData, sourceData, byteCount)
    }
    return copy
  }
}

private final class HuddlePeerPlayback {
  private let player: AVAudioPlayerNode
  private let decoder: HuddleOpusDecoder
  private var jitterQueue = HuddlePacketJitterQueue(
    capacity: 10,
    startPackets: 3
  )

  init(
    engine: AVAudioEngine,
    pcmFormat: AVAudioFormat
  ) throws {
    decoder = try HuddleOpusDecoder()
    let player = AVAudioPlayerNode()
    self.player = player
    engine.attach(player)
    do {
      engine.connect(player, to: engine.mainMixerNode, format: pcmFormat)
      player.play()
    } catch {
      player.stop()
      engine.disconnectNodeOutput(player)
      engine.detach(player)
      throw error
    }
  }

  func enqueue(_ packet: HuddleRemoteOpusPacket) {
    jitterQueue.enqueue(packet)
  }

  func drainOne() throws {
    guard let packet = jitterQueue.drainOne() else { return }
    let decoded = try decoder.decode(packet.opus)
    player.scheduleBuffer(decoded)
    if !player.isPlaying {
      player.play()
    }
  }

  func pause() {
    if player.isPlaying {
      player.pause()
    }
  }

  func resume() {
    if !player.isPlaying {
      player.play()
    }
  }

  func release(from engine: AVAudioEngine) {
    player.stop()
    engine.disconnectNodeOutput(player)
    engine.detach(player)
    jitterQueue = HuddlePacketJitterQueue(capacity: 10, startPackets: 3)
  }
}
