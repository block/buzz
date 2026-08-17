import Flutter
import SwiftUI
import UIKit

private struct NativeEmojiMediaHeaderError: Error {}

final class NativeEmojiPickerCoordinator: NSObject,
  UIAdaptivePresentationControllerDelegate
{
  private let channel: FlutterMethodChannel
  private static weak var activeCoordinator: NativeEmojiPickerCoordinator?
  private weak var parentViewController: UIViewController?
  private weak var presentedController: UIViewController?
  private var didNotifyDismissal = false
  private var isDismissing = false

  init(
    messenger: FlutterBinaryMessenger,
    parentViewController: UIViewController?
  ) {
    channel = FlutterMethodChannel(
      name: "buzz/native_emoji_picker",
      binaryMessenger: messenger
    )
    self.parentViewController = parentViewController
    super.init()
    Self.activeCoordinator = self
    channel.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result)
    }
  }

  static func mediaHeaders(for url: URL) async throws -> [String: String] {
    guard let channel = activeCoordinator?.channel else { return [:] }
    return try await withCheckedThrowingContinuation { continuation in
      channel.invokeMethod("mediaHeaders", arguments: url.absoluteString) { result in
        if result is FlutterError {
          continuation.resume(throwing: NativeEmojiMediaHeaderError())
          return
        }
        continuation.resume(returning: result as? [String: String] ?? [:])
      }
    }
  }

  private func handle(
    _ call: FlutterMethodCall,
    result: @escaping FlutterResult
  ) {
    guard call.method == "present" else {
      result(FlutterMethodNotImplemented)
      return
    }
    guard let arguments = call.arguments as? [String: Any] else {
      result(
        FlutterError(
          code: "invalid_arguments",
          message: "Expected emoji-picker configuration.",
          details: nil
        )
      )
      return
    }

    DispatchQueue.main.async { [weak self] in
      result(self?.present(arguments: arguments) ?? false)
    }
  }

  @MainActor
  private func present(arguments: [String: Any]) -> Bool {
    // A sheet is already owned by an earlier caller; report busy rather than a
    // successful presentation so the caller does not treat this as its own.
    guard presentedController == nil else { return false }
    guard
      let data = NativeEmojiPickerDataLoader.load(arguments: arguments),
      let presenter = topViewController(
        from: parentViewController ?? activeWindowRootViewController()
      )
    else {
      return false
    }

    didNotifyDismissal = false
    isDismissing = false
    let appearance = NativeEmojiPickerAppearance(arguments: arguments)
    let content = NativeEmojiPickerView(
      data: data,
      appearance: appearance,
      initialSkinTone: (arguments["skinTone"] as? NSNumber)?.intValue ?? 0,
      onSelect: { [weak self] emoji in self?.select(emoji) },
      onSkinToneChanged: { [weak self] value in
        self?.channel.invokeMethod("skinToneChanged", arguments: value)
      },
      onClose: { [weak self] in self?.dismiss() }
    )
    let controller = UIHostingController(rootView: content)
    controller.view.backgroundColor = appearance.surface
    controller.modalPresentationStyle = .pageSheet
    controller.overrideUserInterfaceStyle = appearance.isDark ? .dark : .light

    if let sheet = controller.sheetPresentationController {
      let compactID = UISheetPresentationController.Detent.Identifier(
        "buzz.emoji.compact"
      )
      let mediumID = UISheetPresentationController.Detent.Identifier(
        "buzz.emoji.medium"
      )
      sheet.detents = [
        .custom(identifier: compactID) { context in
          context.maximumDetentValue * 0.34
        },
        .custom(identifier: mediumID) { context in
          context.maximumDetentValue * 0.67
        },
        .large(),
      ]
      sheet.selectedDetentIdentifier = mediumID
      sheet.prefersGrabberVisible = true
      sheet.prefersScrollingExpandsWhenScrolledToEdge = false
      sheet.prefersEdgeAttachedInCompactHeight = false
      sheet.widthFollowsPreferredContentSizeWhenEdgeAttached = true
    }

    presentedController = controller
    presenter.present(controller, animated: true) { [weak self, weak controller] in
      controller?.presentationController?.delegate = self
    }
    return true
  }

  @MainActor
  private func select(_ emoji: String) {
    // A single presentation returns at most one selection. The sheet stays
    // live through its dismissal animation, so ignore extra taps that arrive
    // before dismissal completes to avoid emitting duplicate selections.
    guard !isDismissing else { return }
    channel.invokeMethod("selected", arguments: emoji)
    dismiss()
  }

  @MainActor
  private func dismiss() {
    isDismissing = true
    guard let controller = presentedController else {
      notifyDismissalIfNeeded()
      return
    }
    controller.dismiss(animated: true) { [weak self] in
      self?.notifyDismissalIfNeeded()
    }
  }

  func presentationControllerDidDismiss(
    _ presentationController: UIPresentationController
  ) {
    notifyDismissalIfNeeded()
  }

  @MainActor
  private func notifyDismissalIfNeeded() {
    guard !didNotifyDismissal else { return }
    didNotifyDismissal = true
    presentedController = nil
    channel.invokeMethod("dismissed", arguments: nil)
  }

  @MainActor
  private func activeWindowRootViewController() -> UIViewController? {
    UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap(\.windows)
      .first(where: \.isKeyWindow)?
      .rootViewController
  }

  @MainActor
  private func topViewController(
    from viewController: UIViewController?
  ) -> UIViewController? {
    if let presented = viewController?.presentedViewController {
      return topViewController(from: presented)
    }
    if let navigation = viewController as? UINavigationController {
      return topViewController(from: navigation.visibleViewController)
    }
    if let tab = viewController as? UITabBarController {
      return topViewController(from: tab.selectedViewController)
    }
    return viewController
  }
}
