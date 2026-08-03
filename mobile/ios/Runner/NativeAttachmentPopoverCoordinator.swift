import CoreText
import Flutter
import UIKit

enum NativeAttachmentExpandedSurfaceBehavior {
  @MainActor
  static func dismissKeyboard(in window: UIWindow?) {
    window?.endEditing(true)
  }

  static func keyboardOverlap(
    containerBounds: CGRect,
    keyboardLayoutFrame: CGRect
  ) -> CGFloat {
    guard
      !keyboardLayoutFrame.isNull,
      !keyboardLayoutFrame.isInfinite,
      keyboardLayoutFrame.minY < containerBounds.maxY
    else {
      return 0
    }
    return max(0, containerBounds.maxY - keyboardLayoutFrame.minY)
  }
}

enum NativeAttachmentPopoverAnchorLayout {
  static let expandedVerticalOffset: CGFloat = 40

  static func sourceRect(
    anchorBounds: CGRect,
    keyboardDismissalOffset: CGFloat,
    isExpanded: Bool
  ) -> CGRect {
    anchorBounds.offsetBy(
      dx: 0,
      dy: keyboardDismissalOffset
        + (isExpanded ? expandedVerticalOffset : 0)
    )
  }
}

enum NativeAttachmentPopoverPresentationLayout {
  static func keyboardDismissalOffset(
    sourceRect: CGRect,
    containerBounds: CGRect,
    safeAreaInsets: UIEdgeInsets,
    keyboardLayoutFrame: CGRect,
    menuHeight: CGFloat
  ) -> CGFloat {
    let keyboardOverlap =
      NativeAttachmentExpandedSurfaceBehavior.keyboardOverlap(
        containerBounds: containerBounds,
        keyboardLayoutFrame: keyboardLayoutFrame
      )
    guard keyboardOverlap > 0 else { return 0 }

    let availableHeight =
      sourceRect.minY - (containerBounds.minY + safeAreaInsets.top)
    return availableHeight >= menuHeight ? 0 : keyboardOverlap
  }

  static func sourceRect(
    _ sourceRect: CGRect,
    keyboardDismissalOffset: CGFloat
  ) -> CGRect {
    sourceRect.offsetBy(dx: 0, dy: keyboardDismissalOffset)
  }
}

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
    var convertedRect: CGRect
    if let window = sourceView?.window {
      convertedRect = sourceView?.convert(sourceRect, from: window) ?? sourceRect
    } else {
      convertedRect = sourceRect
    }

    if let sourceView {
      let keyboardDismissalOffset =
        NativeAttachmentPopoverPresentationLayout.keyboardDismissalOffset(
          sourceRect: convertedRect,
          containerBounds: sourceView.bounds,
          safeAreaInsets: sourceView.safeAreaInsets,
          keyboardLayoutFrame: sourceView.keyboardLayoutGuide.layoutFrame,
          menuHeight: NativeAttachmentMenuLayout.size(
            compatibleWith: sourceView.traitCollection
          ).height
        )
      if keyboardDismissalOffset > 0 {
        NativeAttachmentExpandedSurfaceBehavior.dismissKeyboard(
          in: sourceView.window
        )
        convertedRect =
          NativeAttachmentPopoverPresentationLayout.sourceRect(
            convertedRect,
            keyboardDismissalOffset: keyboardDismissalOffset
          )
      }
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

enum NativeAttachmentMenuLayout {
  static let itemCount: CGFloat = 4
  static let contentPadding: CGFloat = 16
  static let minimumItemHeight: CGFloat = 52
  static let itemSpacing: CGFloat = 8
  static let itemVerticalPadding: CGFloat = 8
  static let maximumHeight: CGFloat = 372
  static let width: CGFloat = 216
  static let labelTextStyle: UIFont.TextStyle = .title3

  static func itemHeight(
    compatibleWith traitCollection: UITraitCollection
  ) -> CGFloat {
    let labelHeight = NativeAttachmentMenuTypography.font(
      forTextStyle: labelTextStyle,
      compatibleWith: traitCollection
    ).lineHeight
    return max(
      minimumItemHeight,
      ceil(labelHeight + (itemVerticalPadding * 2))
    )
  }

  static func itemsHeight(
    compatibleWith traitCollection: UITraitCollection
  ) -> CGFloat {
    (itemHeight(compatibleWith: traitCollection) * itemCount)
      + (itemSpacing * (itemCount - 1))
  }

  static func contentHeight(
    compatibleWith traitCollection: UITraitCollection
  ) -> CGFloat {
    (contentPadding * 2) + itemsHeight(compatibleWith: traitCollection)
  }

  static func size(
    compatibleWith traitCollection: UITraitCollection,
    maximumHeight: CGFloat = NativeAttachmentMenuLayout.maximumHeight
  ) -> CGSize {
    CGSize(
      width: width,
      height: min(
        contentHeight(compatibleWith: traitCollection),
        maximumHeight
      )
    )
  }
}

enum NativeAttachmentMenuTypography {
  static let interPostScriptName = "InterVariable"

  private static let registeredInter: Bool = {
    let fontURL = Bundle.main.bundleURL
      .appendingPathComponent("Frameworks")
      .appendingPathComponent("App.framework")
      .appendingPathComponent("flutter_assets")
      .appendingPathComponent("assets")
      .appendingPathComponent("fonts")
      .appendingPathComponent("InterVariable.ttf")
    guard FileManager.default.fileExists(atPath: fontURL.path) else {
      return false
    }
    return CTFontManagerRegisterFontsForURL(
      fontURL as CFURL,
      .process,
      nil
    )
  }()

  static func font(
    forTextStyle textStyle: UIFont.TextStyle,
    compatibleWith traitCollection: UITraitCollection? = nil
  ) -> UIFont {
    _ = registeredInter
    let scaledPointSize = UIFontMetrics(forTextStyle: textStyle).scaledValue(
      for: 20,
      compatibleWith: traitCollection
    )
    let preferredFont = UIFont.preferredFont(
      forTextStyle: textStyle,
      compatibleWith: traitCollection
    )
    guard
      let interFont = UIFont(
        name: interPostScriptName,
        size: scaledPointSize
      )
    else {
      return preferredFont
    }
    return interFont
  }
}

enum NativeAttachmentPopoverStyle {
  static let cornerRadius: CGFloat = 20
  static let shadowOpacity: Float = 0.18
  static let shadowRadius: CGFloat = 12
  static let shadowOffset = CGSize(width: 0, height: 6)
  static let borderWidth: CGFloat = 1
}

func makeNativeAttachmentMenuButton(
  title: String,
  symbol: String,
  action: @escaping () -> Void
) -> UIButton {
  let button = UIButton(
    primaryAction: UIAction { _ in
      UISelectionFeedbackGenerator().selectionChanged()
      action()
    }
  )
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
  titleLabel.font = NativeAttachmentMenuTypography.font(
    forTextStyle: NativeAttachmentMenuLayout.labelTextStyle
  )
  titleLabel.adjustsFontForContentSizeCategory = true
  titleLabel.textAlignment = .left
  titleLabel.translatesAutoresizingMaskIntoConstraints = false

  button.addSubview(iconView)
  button.addSubview(titleLabel)
  NSLayoutConstraint.activate([
    iconView.leadingAnchor.constraint(
      equalTo: button.leadingAnchor,
      constant: 8
    ),
    iconView.centerYAnchor.constraint(equalTo: button.centerYAnchor),
    iconView.widthAnchor.constraint(equalToConstant: 26),
    titleLabel.leadingAnchor.constraint(
      equalTo: iconView.trailingAnchor,
      constant: 12
    ),
    titleLabel.trailingAnchor.constraint(
      equalTo: button.trailingAnchor,
      constant: -8
    ),
    titleLabel.centerYAnchor.constraint(equalTo: button.centerYAnchor),
  ])
  button.configurationUpdateHandler = { button in
    button.alpha = button.isHighlighted ? 0.62 : 1
  }
  return button
}
