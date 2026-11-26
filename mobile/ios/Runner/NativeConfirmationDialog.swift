import Flutter
import UIKit

/// Owns a UIKit confirmation or choice alert until dismissal and replies
/// exactly once.
final class NativeConfirmationDialogCoordinator: NSObject {
  private let channel: FlutterMethodChannel
  private weak var parent: UIViewController?
  private weak var alert: UIAlertController?
  private var pendingResult: FlutterResult?
  private var isFinishing = false
  /// What dismissing answers: false for a confirmation, nothing for a choice.
  private var cancelValue: Any? = false

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
    guard call.method == "present" || call.method == "choose" else {
      result(FlutterMethodNotImplemented)
      return
    }
    let choosing = call.method == "choose"
    guard let data = call.arguments as? [String: Any],
      let title = data["title"] as? String,
      let message = data["message"] as? String,
      let cancelLabel = data["cancelLabel"] as? String else {
      result(FlutterError(code: "invalid_arguments", message: "Expected confirmation labels.", details: nil))
      return
    }
    // A confirmation has one action; a choice has its options,
    // the last one preferred, and answers with the chosen index.
    let labels: [String]
    if choosing {
      guard let options = data["options"] as? [String], !options.isEmpty else {
        result(FlutterError(code: "invalid_arguments", message: "Expected choice options.", details: nil))
        return
      }
      labels = options
    } else {
      guard let confirmLabel = data["confirmLabel"] as? String else {
        result(FlutterError(code: "invalid_arguments", message: "Expected confirmation labels.", details: nil))
        return
      }
      labels = [confirmLabel]
    }
    // Repeated taps must not create stacked alerts or multiple actions.
    guard pendingResult == nil else { result(choosing ? nil : false); return }
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
    cancelValue = choosing ? nil : false
    alert.addAction(UIAlertAction(title: cancelLabel, style: .cancel) { [weak self] _ in
      self?.cancel()
    })
    let destructive = data["destructive"] as? Bool ?? true
    for (index, label) in labels.enumerated() {
      let style: UIAlertAction.Style = choosing || !destructive ? .default : .destructive
      let action = UIAlertAction(title: label, style: style) { [weak self] _ in
        self?.finish(choosing ? index : true)
      }
      alert.addAction(action)
      if choosing && index == labels.count - 1 { alert.preferredAction = action }
    }
    pendingResult = result
    self.alert = alert
    presenter.present(alert, animated: !UIAccessibility.isReduceMotionEnabled)
  }

  /// Dismisses an outstanding alert without authorizing any action.
  func cancel() { finish(cancelValue) }

  private func finish(_ value: Any?) {
    guard pendingResult != nil, !isFinishing else { return }
    isFinishing = true
    guard let alert else { resolve(value); return }
    // Finish native dismissal before Flutter removes the underlying route.
    alert.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in
      self?.resolve(value)
    }
  }

  private func resolve(_ value: Any?) {
    let result = pendingResult
    pendingResult = nil
    isFinishing = false
    alert = nil
    result?(value)
  }
}
