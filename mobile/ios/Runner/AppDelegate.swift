import AVFoundation
import BuzzPushKit
import Flutter
import UIKit
import UserNotifications

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  private var mediaUploadChannel: FlutterMethodChannel?
  private var pushChannel: FlutterMethodChannel?
  private let apnsRegistrationBuffer = APNsRegistrationBuffer()
  private var appGroupIdentifier: String? {
    Bundle.main.object(forInfoDictionaryKey: "BuzzAppGroupIdentifier") as? String
  }

  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound]) {
      granted, _ in
      if granted {
        DispatchQueue.main.async {
          application.registerForRemoteNotifications()
        }
      }
    }
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    mediaUploadChannel = FlutterMethodChannel(
      name: "buzz/media_upload",
      binaryMessenger: engineBridge.applicationRegistrar.messenger()
    )
    mediaUploadChannel?.setMethodCallHandler { [weak self] call, result in
      self?.handleMediaUploadMethodCall(call, result: result)
    }

    pushChannel = FlutterMethodChannel(
      name: "buzz/push",
      binaryMessenger: engineBridge.applicationRegistrar.messenger()
    )
    pushChannel?.setMethodCallHandler { [weak self] call, result in
      self?.handlePushMethodCall(call, result: result)
    }
    apnsRegistrationBuffer.attach { [weak self] update in
      self?.pushChannel?.invokeMethod(update.method, arguments: update.arguments)
    }
  }

  override func application(
    _ application: UIApplication,
    didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
  ) {
    super.application(application, didRegisterForRemoteNotificationsWithDeviceToken: deviceToken)
    apnsRegistrationBuffer.recordToken(deviceToken)
  }

  override func application(
    _ application: UIApplication,
    didFailToRegisterForRemoteNotificationsWithError error: Error
  ) {
    super.application(application, didFailToRegisterForRemoteNotificationsWithError: error)
    apnsRegistrationBuffer.recordError(error.localizedDescription)
  }

  private func handlePushMethodCall(
    _ call: FlutterMethodCall,
    result: @escaping FlutterResult
  ) {
    switch call.method {
    case "saveCommunitySnapshot":
      guard let arguments = call.arguments as? [String: Any],
        let communities = arguments["communities"] as? [[String: Any]],
        let signingKeys = arguments["signingKeys"] as? [String: String]
      else {
        result(
          FlutterError(
            code: "invalid_arguments", message: "Expected communities array.", details: nil))
        return
      }
      do {
        try savePushCommunitySnapshot(communities)
        try BuzzPushKeychain.replace(
          signingKeys: signingKeys,
          accessGroup: Bundle.main.object(forInfoDictionaryKey: "BuzzKeychainAccessGroup")
            as? String
        )
        result(nil)
      } catch {
        result(
          FlutterError(
            code: "save_failed", message: "Unable to save push community credentials.",
            details: error.localizedDescription))
      }
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  private func savePushCommunitySnapshot(_ communities: [[String: Any]]) throws {
    guard let appGroupIdentifier else {
      throw NSError(
        domain: "BuzzPush", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "Missing BuzzAppGroupIdentifier"])
    }
    guard
      let container = FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: appGroupIdentifier)
    else {
      throw NSError(
        domain: "BuzzPush", code: 2,
        userInfo: [NSLocalizedDescriptionKey: "Missing App Group container"])
    }
    let data = try JSONSerialization.data(
      withJSONObject: ["communities": communities], options: [.sortedKeys])
    let destination = container.appendingPathComponent("push-communities.json")
    try data.write(to: destination, options: [.atomic])
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

    guard
      let exportSession = AVAssetExportSession(
        asset: asset,
        presetName: AVAssetExportPresetPassthrough
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
    exportSession.metadataItemFilter = AVMetadataItemFilter.forSharing()

    exportSession.exportAsynchronously {
      switch exportSession.status {
      case .completed:
        result(outputURL.path)
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
}
