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
  private var apnsDeviceToken: Data?
  private lazy var endpointGrantStore = BuzzPushEndpointGrantKeychainStore(
    accessGroup: Bundle.main.object(forInfoDictionaryKey: "BuzzKeychainAccessGroup") as? String
  )
  #if DEBUG
    private var devEnrollmentTask: Task<Void, Never>?
  #endif
  private var appGroupIdentifier: String? {
    Bundle.main.object(forInfoDictionaryKey: "BuzzAppGroupIdentifier") as? String
  }
  private var qrScannerChannel: FlutterMethodChannel?
  private var inlinePhotoPickerSupportChannel: FlutterMethodChannel?
  private var nativeAttachmentPopoverCoordinator: NativeAttachmentPopoverCoordinator?

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
    let messenger = engineBridge.applicationRegistrar.messenger()
    mediaUploadChannel = FlutterMethodChannel(
      name: "buzz/media_upload",
      binaryMessenger: messenger
    )
    mediaUploadChannel?.setMethodCallHandler { [weak self] call, result in
      self?.handleMediaUploadMethodCall(call, result: result)
    }
    pushChannel = FlutterMethodChannel(
      name: "buzz/push",
      binaryMessenger: messenger
    )
    pushChannel?.setMethodCallHandler { [weak self] call, result in
      self?.handlePushMethodCall(call, result: result)
    }
    apnsRegistrationBuffer.attach { [weak self] update in
      self?.pushChannel?.invokeMethod(update.method, arguments: update.arguments)
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

  override func application(
    _ application: UIApplication,
    didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
  ) {
    super.application(application, didRegisterForRemoteNotificationsWithDeviceToken: deviceToken)
    apnsDeviceToken = deviceToken
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
    case "endpointGrants":
      do {
        result(try endpointGrantStore.records().map(\.flutterArguments))
      } catch {
        result(
          FlutterError(
            code: "endpoint_grant_read_failed",
            message: "Unable to read persisted push endpoint grants.",
            details: error.localizedDescription
          )
        )
      }
    #if DEBUG
      case "devEnrollPush":
        handleDevPushEnrollment(call, result: result)
      case "devMarkPushLeasePublished":
        handleDevPushLeasePublished(call, result: result)
    #endif
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  #if DEBUG
    private func handleDevPushEnrollment(
      _ call: FlutterMethodCall,
      result: @escaping FlutterResult
    ) {
      guard devEnrollmentTask == nil else {
        result(
          FlutterError(
            code: "enrollment_in_progress",
            message: "Development push enrollment is already running.",
            details: nil
          )
        )
        return
      }
      guard let deviceToken = apnsDeviceToken else {
        result(
          FlutterError(
            code: "missing_apns_token",
            message: "APNs has not supplied a device token.",
            details: nil
          )
        )
        return
      }
      guard !deviceToken.isEmpty else {
        preconditionFailure("APNs supplied an empty device token")
      }
      guard let arguments = call.arguments as? [String: Any],
        let relayText = arguments["relayUrl"] as? String,
        let relayURL = URL(string: relayText),
        let gatewayText = arguments["gatewayUrl"] as? String,
        let gatewayURL = URL(string: gatewayText)
      else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Development push enrollment requires relayUrl and gatewayUrl.",
            details: nil
          )
        )
        return
      }

      do {
        let driver = try BuzzDevPushEnrollmentDriver(
          gatewayBaseURL: gatewayURL,
          store: endpointGrantStore
        )
        devEnrollmentTask = Task { [weak self] in
          defer { self?.devEnrollmentTask = nil }
          do {
            let record = try await driver.enroll(deviceToken: deviceToken, relayURL: relayURL)
            await MainActor.run { result(record.flutterArguments) }
          } catch {
            await MainActor.run {
              result(
                FlutterError(
                  code: "dev_enrollment_failed",
                  message: "Development push enrollment failed.",
                  details: error.localizedDescription
                )
              )
            }
          }
        }
      } catch {
        result(
          FlutterError(
            code: "dev_enrollment_configuration_failed",
            message: "Development push enrollment is not configured.",
            details: error.localizedDescription
          )
        )
      }
    }

    private func handleDevPushLeasePublished(
      _ call: FlutterMethodCall,
      result: @escaping FlutterResult
    ) {
      guard let arguments = call.arguments as? [String: Any],
        let relayOrigin = arguments["relayOrigin"] as? String,
        let appProfile = arguments["appProfile"] as? String,
        let generation = (arguments["generation"] as? NSNumber)?.int64Value,
        !relayOrigin.isEmpty,
        !appProfile.isEmpty,
        generation > 0
      else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Published push lease state requires relayOrigin, appProfile, and generation.",
            details: nil
          )
        )
        return
      }
      do {
        try endpointGrantStore.markPublished(
          relayOrigin: relayOrigin,
          appProfile: appProfile,
          generation: generation
        )
        result(nil)
      } catch {
        result(
          FlutterError(
            code: "push_lease_state_write_failed",
            message: "Unable to persist the published push lease generation.",
            details: error.localizedDescription
          )
        )
      }
    }
  #endif

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
