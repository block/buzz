import AVFoundation
import Flutter
import Speech

/// File-based on-device recognition. Capture and waveform remain owned by Flutter.
@MainActor
final class DeviceTranscriptionPlugin {
  private let channel: FlutterMethodChannel
  private var operation: Task<Void, Never>?
  private var owner: String?
  private var cancelAnalysis: (() -> Void)?

  init(messenger: FlutterBinaryMessenger) {
    channel = FlutterMethodChannel(name: "buzz/transcription", binaryMessenger: messenger)
    channel.setMethodCallHandler { [weak self] call, reply in
      guard let self, let args = call.arguments as? [String: Any],
        let id = args["id"] as? String else {
        reply(FlutterError(code: "arguments", message: "Invalid transcription request.", details: nil))
        return
      }
      if call.method == "cancel" {
        if owner == id { operation?.cancel(); cancelAnalysis?(); owner = nil }
        reply(nil)
        return
      }
      guard call.method == "prepare" || call.method == "transcribe" else {
        reply(FlutterMethodNotImplemented); return
      }
      guard #available(iOS 26.0, *) else {
        reply(FlutterError(code: "unavailable", message: "On-device transcription requires iOS 26.", details: nil)); return
      }
      guard owner == nil || owner == id else {
        reply(FlutterError(code: "busy", message: "Another transcription is active.", details: nil)); return
      }
      owner = id
      operation = Task { [weak self] in
        guard let self else { return }
        do {
          guard SpeechTranscriber.isAvailable,
            let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale.current)
          else { throw TranscriptionFailure.unavailable }
          let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
          if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
            try await request.downloadAndInstall()
          }
          try Task.checkCancellation()
          if call.method == "prepare" {
            if owner == id { owner = nil }
            reply(nil)
            return
          }
          guard let path = args["path"] as? String else { throw TranscriptionFailure.invalidFile }
          let file = try AVAudioFile(forReading: URL(fileURLWithPath: path))
          let analyzer = SpeechAnalyzer(modules: [transcriber])
          cancelAnalysis = { Task { await analyzer.cancelAndFinishNow() } }
          let results = Task {
            var text = ""
            for try await result in transcriber.results {
              if result.isFinal { text += String(result.text.characters) }
            }
            return text.trimmingCharacters(in: .whitespacesAndNewlines)
          }
          do {
            try await analyzer.start(inputAudioFile: file, finishAfterFile: true)
            let text = try await results.value
            try Task.checkCancellation()
            reply(text)
          } catch {
            results.cancel()
            await analyzer.cancelAndFinishNow()
            throw error
          }
        } catch {
          reply(FlutterError(code: "transcription_failed", message: error.localizedDescription, details: nil))
        }
        if owner == id { owner = nil; cancelAnalysis = nil; operation = nil }
      }
    }
  }
}

private enum TranscriptionFailure: LocalizedError {
  case unavailable, invalidFile
  var errorDescription: String? {
    switch self {
    case .unavailable: return "On-device transcription is unavailable for this device or language."
    case .invalidFile: return "The saved recording could not be opened."
    }
  }
}
