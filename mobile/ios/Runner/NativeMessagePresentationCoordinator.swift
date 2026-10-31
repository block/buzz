import Flutter
import UIKit

/// Owns one UIKit reaction-details surface and completes its Flutter request on dismissal.
final class NativeMessagePresentationCoordinator: NSObject, UIAdaptivePresentationControllerDelegate {
  private let channel: FlutterMethodChannel
  private weak var parent: UIViewController?
  private weak var presented: UIViewController?
  private var pendingResult: FlutterResult?
  private var requestID: String?
  private var terminalValue: [String: String]?

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
    if call.method == "updateReactions" {
      if data["requestId"] as? String == requestID,
        let sheet = presented as? NativeReactionDetailsViewController,
        let reactions = data["reactions"] as? [[String: Any]] {
        sheet.updateReactions(reactions)
      }
      result([:])
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
    guard call.method == "reactions" else {
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
    let controller = NativeReactionDetailsViewController(data: data)
    controller.onClose = { [weak self] in self?.dismiss([:]) }
    controller.onAddReaction = { [weak self] in
      self?.dismiss(["action": "addReaction"])
    }
    controller.onToggle = { [weak self] emoji in
      self?.channel.invokeMethod("toggleReaction", arguments: [
        "requestId": data["requestId"] as? String ?? "", "emoji": emoji,
      ])
    }
    controller.modalPresentationStyle = .pageSheet
    if let presentation = controller.sheetPresentationController {
      presentation.overrideTraitCollection = UITraitCollection(userInterfaceStyle: controller.overrideUserInterfaceStyle)
      presentation.detents = [.medium(), .large()]
      presentation.prefersGrabberVisible = true
      presentation.prefersScrollingExpandsWhenScrolledToEdge = true
    }
    controller.overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
    pendingResult = result
    requestID = data["requestId"] as? String
    presented = controller
    controller.presentationController?.delegate = self
    UIImpactFeedbackGenerator(style: .medium).impactOccurred()
    parent.present(controller, animated: !UIAccessibility.isReduceMotionEnabled)
  }

  private func dismiss(_ value: [String: String]) {
    guard pendingResult != nil, terminalValue == nil else { return }
    terminalValue = value
    guard let presented else { complete(value); return }
    let completion: () -> Void = { [weak self] in
      guard let self, self.presented === presented else { return }
      self.complete(value)
    }
    presented.dismiss(animated: !UIAccessibility.isReduceMotionEnabled, completion: completion)
  }

  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    guard presented === presentationController.presentedViewController else { return }
    complete([:])
  }

  private func complete(_ value: [String: String]) {
    let result = pendingResult
    let terminal = terminalValue ?? value
    terminalValue = nil
    pendingResult = nil
    presented = nil
    requestID = nil
    result?(terminal)
  }
}
