import Flutter
import UIKit

final class NativeAttachmentPopoverCoordinator: NSObject {
  private let channel: FlutterMethodChannel
  private weak var parentViewController: UIViewController?
  private weak var presentedController: UIViewController?
  private weak var sourceAnchorView: UIView?

  init(
    messenger: FlutterBinaryMessenger,
    parentViewController: UIViewController?
  ) {
    channel = FlutterMethodChannel(
      name: "buzz/native_attachment_popover",
      binaryMessenger: messenger
    )
    self.parentViewController = parentViewController
    super.init()

    channel.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result)
    }
  }

  private func handle(
    _ call: FlutterMethodCall,
    result: @escaping FlutterResult
  ) {
    switch call.method {
    case "isSupported":
      if #available(iOS 26.0, *) {
        result(true)
      } else {
        result(false)
      }
    case "present":
      guard
        let arguments = call.arguments as? [String: Any],
        let x = arguments["x"] as? NSNumber,
        let y = arguments["y"] as? NSNumber,
        let width = arguments["width"] as? NSNumber,
        let height = arguments["height"] as? NSNumber
      else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "Expected the attachment trigger bounds.",
            details: nil
          )
        )
        return
      }

      let sourceRect = CGRect(
        x: CGFloat(truncating: x),
        y: CGFloat(truncating: y),
        width: CGFloat(truncating: width),
        height: CGFloat(truncating: height)
      )
      DispatchQueue.main.async { [weak self] in
        result(self?.presentPopover(sourceRect: sourceRect) ?? false)
      }
    case "dismiss":
      DispatchQueue.main.async { [weak self] in
        if #available(iOS 26.0, *),
          let controller =
            self?.presentedController
            as? NativeAttachmentPopoverViewController
        {
          controller.dismissAndNotify()
        } else {
          self?.presentedController?.dismiss(animated: true)
        }
        result(nil)
      }
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  @MainActor
  private func presentPopover(sourceRect: CGRect) -> Bool {
    guard #available(iOS 26.0, *) else { return false }
    guard presentedController == nil else { return true }
    let rootViewController =
      parentViewController ?? activeWindowRootViewController()
    guard let presenter = topViewController(from: rootViewController) else {
      return false
    }

    let sourceView = presenter.view
    let convertedRect: CGRect
    if let window = sourceView?.window {
      convertedRect = sourceView?.convert(sourceRect, from: window) ?? sourceRect
    } else {
      convertedRect = sourceRect
    }

    let anchorView = makeSourceAnchor(frame: convertedRect)
    sourceView?.addSubview(anchorView)
    sourceAnchorView = anchorView

    let availableWidth = max(
      320,
      min(
        (sourceView?.bounds.width ?? UIScreen.main.bounds.width) - 24,
        430
      )
    )
    let controller = NativeAttachmentPopoverViewController(
      channel: channel,
      expandedWidth: availableWidth
    )
    controller.modalPresentationStyle = .popover
    controller.preferredTransition = .zoom { [weak anchorView] _ in
      anchorView
    }
    controller.onDismiss = { [weak self] in
      self?.presentedController = nil
      self?.sourceAnchorView?.removeFromSuperview()
      self?.channel.invokeMethod("dismissed", arguments: nil)
    }

    guard let popover = controller.popoverPresentationController else {
      anchorView.removeFromSuperview()
      return false
    }
    popover.sourceView = anchorView
    popover.sourceRect = anchorView.bounds
    popover.permittedArrowDirections = [.down]
    popover.backgroundColor = .clear
    popover.delegate = controller

    presentedController = controller
    presenter.present(controller, animated: true)
    return true
  }

  @MainActor
  private func makeSourceAnchor(frame: CGRect) -> UIView {
    let anchor = UIView(frame: frame)
    anchor.isUserInteractionEnabled = false
    anchor.accessibilityElementsHidden = true
    anchor.backgroundColor = .clear
    anchor.layer.cornerRadius = min(frame.width, frame.height) / 2
    anchor.layer.cornerCurve = .continuous
    return anchor
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

func makeNativeAttachmentMenuButton(
  title: String,
  symbol: String,
  action: UIAction
) -> UIButton {
  let button = UIButton(primaryAction: action)
  button.accessibilityLabel = title

  let symbolConfiguration = UIImage.SymbolConfiguration(
    pointSize: 18,
    weight: .regular
  )
  let iconView = UIImageView(
    image: UIImage(
      systemName: symbol,
      withConfiguration: symbolConfiguration
    )
  )
  iconView.tintColor = .label
  iconView.contentMode = .center
  iconView.translatesAutoresizingMaskIntoConstraints = false

  let titleLabel = UILabel()
  titleLabel.text = title
  titleLabel.textColor = .label
  titleLabel.font = .preferredFont(forTextStyle: .body)
  titleLabel.adjustsFontForContentSizeCategory = true
  titleLabel.textAlignment = .left
  titleLabel.translatesAutoresizingMaskIntoConstraints = false

  button.addSubview(iconView)
  button.addSubview(titleLabel)
  NSLayoutConstraint.activate([
    iconView.leadingAnchor.constraint(equalTo: button.leadingAnchor, constant: 14),
    iconView.centerYAnchor.constraint(equalTo: button.centerYAnchor),
    iconView.widthAnchor.constraint(equalToConstant: 26),
    titleLabel.leadingAnchor.constraint(
      equalTo: iconView.trailingAnchor,
      constant: 12
    ),
    titleLabel.trailingAnchor.constraint(
      equalTo: button.trailingAnchor,
      constant: -14
    ),
    titleLabel.centerYAnchor.constraint(equalTo: button.centerYAnchor),
  ])
  button.configurationUpdateHandler = { button in
    button.alpha = button.isHighlighted ? 0.62 : 1
  }
  return button
}
