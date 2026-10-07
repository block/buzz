import Flutter
import UIKit

final class JumpToLatestGlassButtonFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger

  init(messenger: FlutterBinaryMessenger) {
    self.messenger = messenger
    super.init()
  }

  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }

  func create(
    withFrame frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?
  ) -> FlutterPlatformView {
    JumpToLatestGlassButtonPlatformView(
      frame: frame,
      viewIdentifier: viewId,
      arguments: args,
      messenger: messenger
    )
  }
}

private final class JumpToLatestGlassButton: UIButton {
  private static let hitTargetExpansion: CGFloat = 4

  override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
    bounds
      .insetBy(
        dx: -Self.hitTargetExpansion,
        dy: -Self.hitTargetExpansion
      )
      .contains(point)
  }
}

final class NavigationGlassButton: UIButton {
  var hitTargetInsets = UIEdgeInsets.zero

  override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
    guard isEnabled, isUserInteractionEnabled, !isHidden, alpha > 0.01 else {
      return false
    }
    return bounds
      .inset(by: UIEdgeInsets(
        top: -hitTargetInsets.top,
        left: -hitTargetInsets.left,
        bottom: -hitTargetInsets.bottom,
        right: -hitTargetInsets.right
      ))
      .contains(point)
  }
}

final class JumpToLatestGlassButtonPlatformView: NSObject, FlutterPlatformView {
  private let containerView: UIView
  private let channel: FlutterMethodChannel
  private let button = JumpToLatestGlassButton(type: .system)

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?,
    messenger: FlutterBinaryMessenger
  ) {
    containerView = UIView(frame: frame)
    channel = FlutterMethodChannel(
      name: "buzz/jump_to_latest_glass/\(viewId)",
      binaryMessenger: messenger
    )
    super.init()

    containerView.backgroundColor = .clear
    containerView.isOpaque = false
    applyBrightness(from: args)

    var configuration: UIButton.Configuration
    if #available(iOS 26.0, *) {
      configuration = .glass()
    } else {
      configuration = .gray()
      configuration.baseBackgroundColor = UIColor.secondarySystemBackground
    }
    configuration.cornerStyle = .capsule
    configuration.baseForegroundColor = .label
    configuration.image = UIImage(
      systemName: "arrow.down",
      withConfiguration: UIImage.SymbolConfiguration(
        pointSize: 16,
        weight: .semibold
      )
    )
    button.configuration = configuration
    button.accessibilityLabel = "Jump to latest message"
    button.translatesAutoresizingMaskIntoConstraints = false
    button.addAction(
      UIAction { [weak self] _ in
        self?.channel.invokeMethod("pressed", arguments: nil)
      },
      for: .touchUpInside
    )

    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "setBrightness" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.applyBrightness(from: call.arguments)
      result(nil)
    }

    containerView.addSubview(button)
    NSLayoutConstraint.activate([
      button.centerXAnchor.constraint(equalTo: containerView.centerXAnchor),
      button.bottomAnchor.constraint(equalTo: containerView.bottomAnchor),
      button.widthAnchor.constraint(equalToConstant: 40),
      button.heightAnchor.constraint(equalToConstant: 40),
    ])
  }

  func view() -> UIView {
    containerView
  }

  private func applyBrightness(from value: Any?) {
    let brightness = (value as? [String: Any])?["brightness"] as? String
      ?? value as? String
    let interfaceStyle: UIUserInterfaceStyle = brightness == "dark" ? .dark : .light
    containerView.overrideUserInterfaceStyle = interfaceStyle
    button.overrideUserInterfaceStyle = interfaceStyle
    button.setNeedsUpdateConfiguration()
  }

  deinit {
    channel.setMethodCallHandler(nil)
  }
}

final class NavigationGlassButtonFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger

  init(messenger: FlutterBinaryMessenger) {
    self.messenger = messenger
    super.init()
  }

  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }

  func create(
    withFrame frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?
  ) -> FlutterPlatformView {
    NavigationGlassButtonPlatformView(
      frame: frame,
      viewIdentifier: viewId,
      arguments: args,
      messenger: messenger
    )
  }
}

final class NavigationGlassButtonPlatformView: NSObject, FlutterPlatformView {
  private let containerView: UIView
  private let channel: FlutterMethodChannel
  private let button = NavigationGlassButton(type: .system)

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?,
    messenger: FlutterBinaryMessenger
  ) {
    containerView = UIView(frame: frame)
    channel = FlutterMethodChannel(
      name: "buzz/navigation_glass/\(viewId)",
      binaryMessenger: messenger
    )
    super.init()

    containerView.backgroundColor = .clear
    containerView.isOpaque = false
    let arguments = args as? [String: Any]
    let buttonCenterX =
      (arguments?["buttonCenterX"] as? NSNumber)?.doubleValue ?? 24
    let hitTargetWidth =
      (arguments?["hitTargetWidth"] as? NSNumber)?.doubleValue ?? 48
    let hitTargetHeight =
      (arguments?["hitTargetHeight"] as? NSNumber)?.doubleValue ?? 48
    let icon = arguments?["icon"] as? String
    let symbolName = icon == "close" ? "xmark" : "chevron.backward"

    var configuration: UIButton.Configuration
    if #available(iOS 26.0, *) {
      configuration = .glass()
    } else {
      configuration = .gray()
      configuration.baseBackgroundColor = UIColor.secondarySystemBackground
    }
    configuration.cornerStyle = .capsule
    configuration.image = UIImage(
      systemName: symbolName,
      withConfiguration: UIImage.SymbolConfiguration(
        pointSize: 17,
        weight: .semibold
      )
    )
    button.configuration = configuration
    button.hitTargetInsets = UIEdgeInsets(
      top: max(0, (hitTargetHeight - 40) / 2),
      left: max(0, buttonCenterX - 20),
      bottom: max(0, (hitTargetHeight - 40) / 2),
      right: max(0, hitTargetWidth - buttonCenterX - 20)
    )
    button.accessibilityLabel = arguments?["accessibilityLabel"] as? String ?? "Back"
    button.translatesAutoresizingMaskIntoConstraints = false
    button.addAction(
      UIAction { [weak self] _ in
        self?.channel.invokeMethod("pressed", arguments: nil)
      },
      for: .touchUpInside
    )

    applyAppearance(from: args)
    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "setAppearance" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.applyAppearance(from: call.arguments)
      result(nil)
    }

    containerView.addSubview(button)
    NSLayoutConstraint.activate([
      button.centerXAnchor.constraint(
        equalTo: containerView.leadingAnchor,
        constant: buttonCenterX
      ),
      button.centerYAnchor.constraint(equalTo: containerView.centerYAnchor),
      button.widthAnchor.constraint(equalToConstant: 40),
      button.heightAnchor.constraint(equalToConstant: 40),
    ])
  }

  func view() -> UIView {
    containerView
  }

  /// Returns the pre-iOS-26 surface that maximizes contrast with the requested
  /// foreground while preserving the system surface when it already passes.
  static func fallbackBackgroundColor(
    foregroundColor: UIColor?,
    interfaceStyle: UIUserInterfaceStyle
  ) -> UIColor {
    let traits = UITraitCollection(userInterfaceStyle: interfaceStyle)
    let backgroundColor = UIColor.secondarySystemBackground.resolvedColor(with: traits)
    guard interfaceStyle != .dark, let foregroundColor else { return backgroundColor }
    let resolvedForeground = foregroundColor.resolvedColor(with: traits)
    if contrastRatio(foregroundColor: resolvedForeground, backgroundColor: backgroundColor) >= 3 {
      return backgroundColor
    }
    return .black
  }

  static func contrastRatio(
    foregroundColor: UIColor,
    backgroundColor: UIColor
  ) -> CGFloat {
    guard
      let foregroundLuminance = relativeLuminance(of: foregroundColor),
      let backgroundLuminance = relativeLuminance(of: backgroundColor)
    else { return 1 }
    let lighter = max(foregroundLuminance, backgroundLuminance)
    let darker = min(foregroundLuminance, backgroundLuminance)
    return (lighter + 0.05) / (darker + 0.05)
  }

  private static func relativeLuminance(of color: UIColor) -> CGFloat? {
    var red: CGFloat = 0
    var green: CGFloat = 0
    var blue: CGFloat = 0
    var alpha: CGFloat = 0
    guard color.getRed(&red, green: &green, blue: &blue, alpha: &alpha) else {
      return nil
    }

    func linearize(_ component: CGFloat) -> CGFloat {
      component <= 0.04045
        ? component / 12.92
        : pow((component + 0.055) / 1.055, 2.4)
    }

    return 0.2126 * linearize(red) +
      0.7152 * linearize(green) +
      0.0722 * linearize(blue)
  }

  private func applyAppearance(from value: Any?) {
    let arguments = value as? [String: Any]
    let brightness = arguments?["brightness"] as? String
    let interfaceStyle: UIUserInterfaceStyle = brightness == "dark" ? .dark : .light
    let colorValue = (arguments?["foregroundColor"] as? NSNumber)?.uint32Value
    let foregroundColor = colorValue.map(Self.color(from:))
    let enabled = arguments?["enabled"] as? Bool ?? true

    containerView.overrideUserInterfaceStyle = interfaceStyle
    button.overrideUserInterfaceStyle = interfaceStyle
    button.isEnabled = enabled
    if let foregroundColor {
      button.configuration?.baseForegroundColor = foregroundColor
    }
    if #unavailable(iOS 26.0) {
      button.configuration?.baseBackgroundColor = Self.fallbackBackgroundColor(
        foregroundColor: foregroundColor,
        interfaceStyle: interfaceStyle
      )
    }
    button.setNeedsUpdateConfiguration()
  }

  private static func color(from value: UInt32) -> UIColor {
    let alpha = CGFloat((value >> 24) & 0xFF) / 255
    let red = CGFloat((value >> 16) & 0xFF) / 255
    let green = CGFloat((value >> 8) & 0xFF) / 255
    let blue = CGFloat(value & 0xFF) / 255
    return UIColor(red: red, green: green, blue: blue, alpha: alpha)
  }

  deinit {
    channel.setMethodCallHandler(nil)
  }
}
