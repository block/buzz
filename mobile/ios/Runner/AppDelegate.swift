import AVFoundation
import Flutter
import UIKit
import UserNotifications

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  private var mediaUploadChannel: FlutterMethodChannel?
  private var qrScannerChannel: FlutterMethodChannel?
  private var inlinePhotoPickerSupportChannel: FlutterMethodChannel?
  private var concentricSheetSurfaceChannel: FlutterMethodChannel?
  private var nativeAttachmentPopoverCoordinator: NativeAttachmentPopoverCoordinator?

  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    UNUserNotificationCenter.current().requestAuthorization(options: [.badge]) { _, _ in }
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    let messenger = engineBridge.applicationRegistrar.messenger()
    mediaUploadChannel = FlutterMethodChannel(
      name: "buzz/media_upload",
      binaryMessenger: messenger
    )
    mediaUploadChannel?.setMethodCallHandler { [weak self] call, result in
      self?.handleMediaUploadMethodCall(call, result: result)
    }
    qrScannerChannel = FlutterMethodChannel(
      name: "buzz/qr_scanner",
      binaryMessenger: messenger
    )
    qrScannerChannel?.setMethodCallHandler { call, result in
      Self.handleQrScannerMethodCall(call, result: result)
    }
    inlinePhotoPickerSupportChannel = FlutterMethodChannel(
      name: "buzz/inline_photo_picker",
      binaryMessenger: messenger
    )
    inlinePhotoPickerSupportChannel?.setMethodCallHandler { call, result in
      guard call.method == "isSupported" else {
        result(FlutterMethodNotImplemented)
        return
      }
      if #available(iOS 17.0, *) {
        result(true)
      } else {
        result(false)
      }
    }

    if let inlinePhotoPickerRegistrar = engineBridge.pluginRegistry.registrar(
      forPlugin: "BuzzInlinePhotoPicker"
    ) {
      inlinePhotoPickerRegistrar.register(
        InlinePhotoPickerFactory(
          messenger: messenger,
          parentViewController: inlinePhotoPickerRegistrar.viewController
        ),
        withId: "buzz/inline_photo_picker"
      )
    }

    if let concentricSheetRegistrar = engineBridge.pluginRegistry.registrar(
      forPlugin: "BuzzConcentricSheetSurface"
    ) {
      concentricSheetRegistrar.register(
        ConcentricSheetSurfaceFactory(),
        withId: "buzz/concentric_sheet_surface"
      )
      concentricSheetSurfaceChannel = FlutterMethodChannel(
        name: "buzz/concentric_sheet_surface",
        binaryMessenger: messenger
      )
      concentricSheetSurfaceChannel?.setMethodCallHandler { call, result in
        guard call.method == "isSupported" else {
          result(FlutterMethodNotImplemented)
          return
        }
        if #available(iOS 26.0, *) {
          result(true)
        } else {
          result(false)
        }
      }
    }

    let nativeAttachmentRegistrar = engineBridge.pluginRegistry.registrar(
      forPlugin: "BuzzNativeAttachmentPopover"
    )
    nativeAttachmentPopoverCoordinator = NativeAttachmentPopoverCoordinator(
      messenger: messenger,
      parentViewController: nativeAttachmentRegistrar?.viewController
    )
  }

  private static func handleQrScannerMethodCall(
    _ call: FlutterMethodCall,
    result: @escaping FlutterResult
  ) {
    switch call.method {
    case "usesDynamicIslandQrScannerPortal":
      result(
        UIDevice.current.userInterfaceIdiom == .phone
          && usesDynamicIslandQrScannerPortal(
            safeAreaTopInset: activeWindowSafeAreaTopInset()
          )
      )
    case "setDynamicIslandScannerStatusBarHidden":
      guard let hidden = call.arguments as? Bool else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Expected a Bool status-bar visibility value.",
            details: nil
          )
        )
        return
      }
      UIApplication.shared.setStatusBarHidden(hidden, with: .fade)
      result(nil)
    case "performDynamicIslandQrScanSuccessHaptic":
      let generator = UINotificationFeedbackGenerator()
      generator.prepare()
      generator.notificationOccurred(.success)
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  static func usesDynamicIslandQrScannerPortal(
    safeAreaTopInset: CGFloat
  ) -> Bool {
    safeAreaTopInset > 50
  }

  private static func activeWindowSafeAreaTopInset() -> CGFloat {
    UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap(\.windows)
      .first(where: \.isKeyWindow)?
      .safeAreaInsets.top ?? 0
  }

  private func handleMediaUploadMethodCall(
    _ call: FlutterMethodCall,
    result: @escaping FlutterResult
  ) {
    switch call.method {
    case "sanitizeImageForUpload":
      guard
        let arguments = call.arguments as? [String: Any],
        let typedData = arguments["bytes"] as? FlutterStandardTypedData,
        let mimeType = arguments["mimeType"] as? String
      else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Expected image bytes and mime type.",
            details: nil
          )
        )
        return
      }

      guard let image = UIImage(data: typedData.data) else {
        result(
          FlutterError(
            code: "sanitize_failed",
            message: "Unable to decode picked image.",
            details: nil
          )
        )
        return
      }

      do {
        guard let sanitizedData = try MediaSanitizer.sanitizeImage(image, mimeType: mimeType) else {
          result(
            FlutterError(
              code: "sanitize_failed",
              message: "Unable to sanitize picked image.",
              details: mimeType
            )
          )
          return
        }
        result(FlutterStandardTypedData(bytes: sanitizedData))
      } catch {
        result(
          FlutterError(
            code: "sanitize_failed",
            message: "Unable to sanitize picked image.",
            details: mimeType
          )
        )
      }
    case "transcodeImageToJpeg":
      guard let typedData = call.arguments as? FlutterStandardTypedData else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Expected raw image bytes.",
            details: nil
          )
        )
        return
      }

      guard let image = UIImage(data: typedData.data) else {
        result(
          FlutterError(
            code: "transcode_failed",
            message: "Unable to convert picked image to JPEG.",
            details: nil
          )
        )
        return
      }

      do {
        guard let jpegData = try MediaSanitizer.encodeJpeg(image) else {
          result(
            FlutterError(
              code: "transcode_failed",
              message: "Unable to convert picked image to JPEG.",
              details: nil
            )
          )
          return
        }
        result(FlutterStandardTypedData(bytes: jpegData))
      } catch {
        result(
          FlutterError(
            code: "transcode_failed",
            message: "Unable to convert picked image to JPEG.",
            details: nil
          )
        )
      }
    case "transcodeVideoToMp4":
      guard let sourcePath = call.arguments as? String else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Expected source file path as String.",
            details: nil
          )
        )
        return
      }
      transcodeVideoToMp4(sourcePath: sourcePath, result: result)
    case "generateVideoPoster":
      guard let sourcePath = call.arguments as? String else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Expected source file path as String.",
            details: nil
          )
        )
        return
      }
      generateVideoPoster(sourcePath: sourcePath, result: result)
    case "clipboardHasImage":
      result(UIPasteboard.general.hasImages)
    case "readClipboardImage":
      guard let imageData = Self.clipboardImageData(from: UIPasteboard.general) else {
        result(nil)
        return
      }
      result(FlutterStandardTypedData(bytes: imageData))
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  static func clipboardImageData(from pasteboard: UIPasteboard) -> Data? {
    if let pngData = pasteboard.data(forPasteboardType: "public.png") {
      return pngData
    }
    if let jpegData = pasteboard.data(forPasteboardType: "public.jpeg") {
      return jpegData
    }
    for imageType in ["public.heic", "public.heif", "org.webmproject.webp", "com.compuserve.gif"] {
      if let imageData = pasteboard.data(forPasteboardType: imageType) {
        return imageData
      }
    }
    guard let image = pasteboard.image else {
      return nil
    }
    return image.pngData()
  }

  private func transcodeVideoToMp4(
    sourcePath: String,
    result: @escaping FlutterResult
  ) {
    let sourceURL = URL(fileURLWithPath: sourcePath)
    let asset = AVURLAsset(url: sourceURL)

    // Do not export the source asset directly. An iPhone video can carry GPS,
    // spatial-video, and other data tracks even when its user-visible metadata
    // is cleared. A fresh composition copies only one video and one audio
    // track, so those private channels cannot reach the relay.
    let composition = AVMutableComposition()
    guard
      let sourceVideo = asset.tracks(withMediaType: .video).first,
      let destinationVideo = composition.addMutableTrack(
        withMediaType: .video,
        preferredTrackID: kCMPersistentTrackID_Invalid
      )
    else {
      result(
        FlutterError(
          code: "transcode_failed",
          message: "The selected file does not contain a video track.",
          details: nil
        )
      )
      return
    }

    do {
      let sourceAudio = asset.tracks(withMediaType: .audio).first
      let insertionTimes = Self.relativeTrackInsertionTimes(
        videoStart: sourceVideo.timeRange.start,
        audioStart: sourceAudio?.timeRange.start
      )
      try destinationVideo.insertTimeRange(
        sourceVideo.timeRange,
        of: sourceVideo,
        at: insertionTimes.video
      )
      destinationVideo.preferredTransform = sourceVideo.preferredTransform

      if
        let sourceAudio,
        let destinationAudio = composition.addMutableTrack(
          withMediaType: .audio,
          preferredTrackID: kCMPersistentTrackID_Invalid
        )
      {
        try destinationAudio.insertTimeRange(
          sourceAudio.timeRange,
          of: sourceAudio,
          at: insertionTimes.audio ?? .zero
        )
      }
    } catch {
      result(
        FlutterError(
          code: "transcode_failed",
          message: error.localizedDescription,
          details: nil
        )
      )
      return
    }

    guard
      let exportSession = AVAssetExportSession(
        asset: composition,
        // Passthrough preserves the source's HEVC codec and container
        // metadata. Buzz accepts only canonical H.264/AAC MP4s with no
        // metadata channels, so re-encode instead of copying the movie.
        presetName: AVAssetExportPresetMediumQuality
      )
    else {
      result(
        FlutterError(
          code: "transcode_failed",
          message: "Unable to create export session.",
          details: nil
        )
      )
      return
    }

    let outputURL = FileManager.default.temporaryDirectory
      .appendingPathComponent(UUID().uuidString)
      .appendingPathExtension("mp4")

    exportSession.outputURL = outputURL
    exportSession.outputFileType = .mp4
    exportSession.shouldOptimizeForNetworkUse = true
    // `forSharing()` intentionally retains playback metadata. The relay
    // rejects every descriptive metadata channel to avoid leaking location or
    // other private information, so write no source metadata at all.
    exportSession.metadata = []
    exportSession.metadataItemFilter = nil

    exportSession.exportAsynchronously {
      switch exportSession.status {
      case .completed:
        do {
          // AVFoundation writes a standard sample-dependency table (`sdtp`).
          // Older Buzz relays mistook that playback-only box for metadata. Keep
          // its size and payload in a `free` box so chunk offsets stay valid and
          // uploads work before those relays receive the validator fix.
          try Self.neutralizeSampleDependencyBoxes(at: outputURL)
          result(outputURL.path)
        } catch {
          try? FileManager.default.removeItem(at: outputURL)
          result(
            FlutterError(
              code: "transcode_failed",
              message: "Unable to canonicalize transcoded video.",
              details: error.localizedDescription
            )
          )
        }
      default:
        let errorMessage =
          exportSession.error?.localizedDescription
          ?? "Video transcoding failed with status \(exportSession.status.rawValue)."
        result(
          FlutterError(
            code: "transcode_failed",
            message: errorMessage,
            details: nil
          )
        )
        // Clean up partial output on failure.
        try? FileManager.default.removeItem(at: outputURL)
      }
    }
  }

  static func relativeTrackInsertionTimes(
    videoStart: CMTime,
    audioStart: CMTime?
  ) -> (video: CMTime, audio: CMTime?) {
    guard let audioStart else {
      return (video: .zero, audio: nil)
    }

    let timelineStart =
      CMTimeCompare(audioStart, videoStart) < 0 ? audioStart : videoStart
    return (
      video: CMTimeSubtract(videoStart, timelineStart),
      audio: CMTimeSubtract(audioStart, timelineStart)
    )
  }

  private func generateVideoPoster(
    sourcePath: String,
    result: @escaping FlutterResult
  ) {
    DispatchQueue.global(qos: .userInitiated).async {
      let asset = AVURLAsset(url: URL(fileURLWithPath: sourcePath))
      let generator = AVAssetImageGenerator(asset: asset)
      generator.appliesPreferredTrackTransform = true
      generator.maximumSize = CGSize(width: 720, height: 720)
      generator.requestedTimeToleranceBefore = .positiveInfinity
      generator.requestedTimeToleranceAfter = .positiveInfinity

      do {
        let durationSeconds = CMTimeGetSeconds(asset.duration)
        let middleTime = durationSeconds.isFinite && durationSeconds > 0
          ? min(durationSeconds / 2, 1)
          : 0
        let candidateTimes = [0, 0.1, middleTime]
        var posterImage: CGImage?
        var lastError: Error?

        for seconds in candidateTimes {
          do {
            posterImage = try generator.copyCGImage(
              at: CMTime(seconds: seconds, preferredTimescale: 600),
              actualTime: nil
            )
            if posterImage != nil { break }
          } catch {
            lastError = error
          }
        }

        guard let posterImage else {
          throw lastError ?? NSError(
            domain: "BuzzVideoPoster",
            code: 1,
            userInfo: [NSLocalizedDescriptionKey: "Unable to decode a video frame."]
          )
        }
        guard let jpegData = try MediaSanitizer.encodeJpeg(UIImage(cgImage: posterImage)) else {
          throw NSError(
            domain: "BuzzVideoPoster",
            code: 2,
            userInfo: [NSLocalizedDescriptionKey: "Unable to encode video poster."]
          )
        }
        DispatchQueue.main.async {
          result(FlutterStandardTypedData(bytes: jpegData))
        }
      } catch {
        DispatchQueue.main.async {
          result(
            FlutterError(
              code: "poster_failed",
              message: "Unable to create a video preview.",
              details: error.localizedDescription
            )
          )
        }
      }
    }
  }

  private static func neutralizeSampleDependencyBoxes(at url: URL) throws {
    var data = try Data(contentsOf: url)
    try neutralizeSampleDependencyBoxes(in: &data, start: 0, end: data.count)
    try data.write(to: url, options: .atomic)
  }

  private static func neutralizeSampleDependencyBoxes(
    in data: inout Data,
    start: Int,
    end: Int
  ) throws {
    let containers: Set<[UInt8]> = [
      Array("moov".utf8), Array("trak".utf8), Array("mdia".utf8),
      Array("minf".utf8), Array("stbl".utf8), Array("edts".utf8),
      Array("dinf".utf8), Array("sinf".utf8), Array("schi".utf8),
    ]
    let sampleDependencyType = Array("sdtp".utf8)
    let freeType = Array("free".utf8)
    var offset = start

    while offset < end {
      guard end - offset >= 8 else { throw invalidMp4BoxError() }
      let compactSize = Int(readBigEndianUInt32(data, at: offset))
      var headerSize = 8
      let boxSize: Int
      if compactSize == 1 {
        guard end - offset >= 16 else { throw invalidMp4BoxError() }
        let extendedSize = readBigEndianUInt64(data, at: offset + 8)
        guard extendedSize <= UInt64(Int.max) else { throw invalidMp4BoxError() }
        boxSize = Int(extendedSize)
        headerSize = 16
      } else if compactSize == 0 {
        boxSize = end - offset
      } else {
        boxSize = compactSize
      }

      guard boxSize >= headerSize, offset + boxSize <= end else {
        throw invalidMp4BoxError()
      }
      let type = Array(data[(offset + 4)..<(offset + 8)])
      if type == sampleDependencyType {
        data.replaceSubrange((offset + 4)..<(offset + 8), with: freeType)
      } else if containers.contains(type) {
        try neutralizeSampleDependencyBoxes(
          in: &data,
          start: offset + headerSize,
          end: offset + boxSize
        )
      }
      offset += boxSize
    }
  }

  private static func readBigEndianUInt32(_ data: Data, at offset: Int) -> UInt32 {
    data[offset..<(offset + 4)].reduce(0) { ($0 << 8) | UInt32($1) }
  }

  private static func readBigEndianUInt64(_ data: Data, at offset: Int) -> UInt64 {
    data[offset..<(offset + 8)].reduce(0) { ($0 << 8) | UInt64($1) }
  }

  private static func invalidMp4BoxError() -> NSError {
    NSError(
      domain: "BuzzVideoTranscode",
      code: 1,
      userInfo: [NSLocalizedDescriptionKey: "Invalid MP4 box structure."]
    )
  }
}
