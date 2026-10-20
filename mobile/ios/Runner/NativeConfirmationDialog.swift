import Flutter
import UIKit

/// Owns a destructive UIKit alert until dismissal and replies exactly once.
final class NativeConfirmationDialogCoordinator: NSObject {
  private let channel: FlutterMethodChannel
  private weak var parent: UIViewController?
  private weak var alert: UIAlertController?
  private var pendingResult: FlutterResult?
  private var isFinishing = false

  init(messenger: FlutterBinaryMessenger, parentViewController: UIViewController?) {
    channel = FlutterMethodChannel(name: "buzz/confirmation_dialog", binaryMessenger: messenger)
    parent = parentViewController
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { result(false); return }
      self.handle(call, result: result)
    }
  }

  private func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    guard call.method == "present" else { result(FlutterMethodNotImplemented); return }
    guard let data = call.arguments as? [String: Any],
      let title = data["title"] as? String,
      let message = data["message"] as? String,
      let confirmLabel = data["confirmLabel"] as? String,
      let cancelLabel = data["cancelLabel"] as? String else {
      result(FlutterError(code: "invalid_arguments", message: "Expected confirmation labels.", details: nil))
      return
    }
    // Repeated taps must not create stacked alerts or multiple removals.
    guard pendingResult == nil else { result(false); return }
    var presenter = parent ?? UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController
    while let presented = presenter?.presentedViewController { presenter = presented }
    guard let presenter, presenter.view.window != nil,
      !presenter.isBeingDismissed, !(presenter is UIAlertController) else {
      result(FlutterError(code: "presentation_failed", message: "Cannot present confirmation right now.", details: nil))
      return
    }
    let alert = UIAlertController(title: title, message: message, preferredStyle: .alert)
    alert.overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
    alert.addAction(UIAlertAction(title: cancelLabel, style: .cancel) { [weak self] _ in
      self?.cancel()
    })
    alert.addAction(UIAlertAction(title: confirmLabel, style: .destructive) { [weak self] _ in
      self?.finish(true)
    })
    pendingResult = result
    self.alert = alert
    presenter.present(alert, animated: !UIAccessibility.isReduceMotionEnabled)
  }

  /// Dismisses an outstanding confirmation without authorizing the action.
  func cancel() { finish(false) }

  private func finish(_ confirmed: Bool) {
    guard pendingResult != nil, !isFinishing else { return }
    isFinishing = true
    guard let alert else { resolve(confirmed); return }
    // Finish native dismissal before Flutter removes the underlying route.
    alert.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in
      self?.resolve(confirmed)
    }
  }

  private func resolve(_ confirmed: Bool) {
    let result = pendingResult
    pendingResult = nil
    isFinishing = false
    alert = nil
    result?(confirmed)
  }
}
