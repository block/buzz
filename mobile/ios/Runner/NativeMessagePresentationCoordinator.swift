import Flutter
import UIKit

/// Owns one UIKit message surface and completes its Flutter request on dismissal.
final class NativeMessagePresentationCoordinator: NSObject, UIAdaptivePresentationControllerDelegate {
  private let channel: FlutterMethodChannel
  private weak var parent: UIViewController?
  private weak var presented: UIViewController?
  private var pendingResult: FlutterResult?
  private var requestID: String?

  init(messenger: FlutterBinaryMessenger, parentViewController: UIViewController?) {
    channel = FlutterMethodChannel(name: "buzz/native_message_presentation", binaryMessenger: messenger)
    parent = parentViewController
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { result(nil); return }
      self.handle(call, result: result)
    }
  }

  private func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    guard let data = call.arguments as? [String: Any] else { result(nil); return }
    if call.method == "supportsMessage" {
      result(["supported": true])
      return
    }
    if call.method == "updateProfiles" {
      if data["requestId"] as? String == requestID,
        let sheet = presented as? NativeReactionDetailsViewController,
        let profiles = data["profiles"] as? [String: [String: Any]] {
        sheet.updateProfiles(profiles)
      }
      result([:])
      return
    }
    guard call.method == "message" || call.method == "reactions" else {
      result(FlutterMethodNotImplemented)
      return
    }
    // Ignore a second long press while presentation/dismissal is in flight.
    guard pendingResult == nil else { result(["busy": true]); return }
    let root = parent ?? UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController
    guard let parent = root, parent.view.window != nil else { result(nil); return }
    guard parent.presentedViewController == nil else { result(["busy": true]); return }
    let controller: UIViewController
    if call.method == "message" {
      guard let rect = NativeMessageMenuViewController.sourceRect(data),
        let bytes = data["previewBytes"] as? FlutterStandardTypedData,
        let image = UIImage(data: bytes.data) else { result(nil); return }
      // Capture the Flutter message boundary, never the screen behind it.
      // Its transparent pixels and exclusion of attached reactions are preserved.
      let snapshot = UIImageView(image: image)
      snapshot.contentMode = .scaleAspectFit
      let menu = NativeMessageMenuViewController(data: data, sourceRect: rect, preview: snapshot)
      menu.onPreviewReady = { [weak self] ready in
        self?.channel.invokeMethod("messagePresented", arguments: data["requestId"]) { _ in ready() }
      }
      menu.onSelect = { [weak self] value in self?.dismiss(value) }
      menu.modalPresentationStyle = .overFullScreen
      controller = menu
    } else {
      let sheet = NativeReactionDetailsViewController(data: data)
      sheet.onClose = { [weak self] in self?.dismiss([:]) }
      sheet.modalPresentationStyle = .pageSheet
      if let presentation = sheet.sheetPresentationController {
        presentation.overrideTraitCollection = UITraitCollection(userInterfaceStyle: sheet.overrideUserInterfaceStyle)
        presentation.detents = [.medium(), .large()]
        presentation.prefersGrabberVisible = true
        presentation.prefersScrollingExpandsWhenScrolledToEdge = true
      }
      controller = sheet
    }
    controller.overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
    pendingResult = result
    requestID = data["requestId"] as? String
    presented = controller
    controller.presentationController?.delegate = self
    UIImpactFeedbackGenerator(style: .medium).impactOccurred()
    parent.present(controller, animated: call.method == "reactions" && !UIAccessibility.isReduceMotionEnabled)
  }

  private func dismiss(_ value: [String: String]) {
    guard let presented else { complete(value); return }
    let completion: () -> Void = { [weak self] in self?.complete(value) }
    if let menu = presented as? NativeMessageMenuViewController {
      menu.animateOut {
        menu.dismiss(animated: false, completion: completion)
      }
    } else {
      presented.dismiss(animated: !UIAccessibility.isReduceMotionEnabled, completion: completion)
    }
  }

  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    complete([:])
  }

  private func complete(_ value: [String: String]) {
    let result = pendingResult
    pendingResult = nil
    presented = nil
    requestID = nil
    result?(value)
  }
}
