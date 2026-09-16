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
  private static let shutterIconRatio: CGFloat = 80.0 / 115.0
  private static let shutterInsetRatio: CGFloat = 20.0 / 115.0
  private let containerView: UIView
  private let channel: FlutterMethodChannel
  private let button = NavigationGlassButton(type: .system)
  private let activityIndicator = UIActivityIndicatorView(style: .medium)
  private let swatchView = UIView()
  private var buttonLabel: String?
  private var buttonIconName = "chevron.backward"
  private var contentIcon = "back"
  private var buttonImage: UIImage?
  private var isBusy = false
  private var controlSize: CGFloat = 40

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
    let controlWidth =
      (arguments?["controlWidth"] as? NSNumber)?.doubleValue ?? 40
    controlSize =
      (arguments?["controlSize"] as? NSNumber)?.doubleValue ?? 40
    let fillWidth = arguments?["fillWidth"] as? Bool ?? false

    var configuration: UIButton.Configuration
    if #available(iOS 26.0, *) {
      configuration = .glass()
    } else {
      configuration = .gray()
      configuration.baseBackgroundColor = UIColor.secondarySystemBackground
    }
    configuration.cornerStyle = .capsule
    button.configuration = configuration
    applyContent(from: arguments)
    button.titleLabel?.numberOfLines = 1
    button.titleLabel?.lineBreakMode = .byClipping
    button.hitTargetInsets = UIEdgeInsets(
      top: max(0, (hitTargetHeight - controlSize) / 2),
      left: max(0, buttonCenterX - controlWidth / 2),
      bottom: max(0, (hitTargetHeight - controlSize) / 2),
      right: max(0, hitTargetWidth - buttonCenterX - controlWidth / 2)
    )
    button.accessibilityLabel = arguments?["accessibilityLabel"] as? String ?? "Back"
    button.translatesAutoresizingMaskIntoConstraints = false
    button.addAction(
      UIAction { [weak self] _ in
        self?.channel.invokeMethod("pressed", arguments: nil)
      },
      for: .touchUpInside
    )

    activityIndicator.translatesAutoresizingMaskIntoConstraints = false
    activityIndicator.hidesWhenStopped = true
    activityIndicator.isUserInteractionEnabled = false
    button.addSubview(activityIndicator)
    NSLayoutConstraint.activate([
      activityIndicator.centerXAnchor.constraint(equalTo: button.centerXAnchor),
      activityIndicator.centerYAnchor.constraint(equalTo: button.centerYAnchor),
      activityIndicator.widthAnchor.constraint(equalToConstant: 24),
      activityIndicator.heightAnchor.constraint(equalToConstant: 24),
    ])

    swatchView.translatesAutoresizingMaskIntoConstraints = false
    swatchView.isUserInteractionEnabled = false
    swatchView.isHidden = true
    swatchView.layer.cornerCurve = .continuous
    button.addSubview(swatchView)
    let swatchSize = max(0, controlSize - 8)
    NSLayoutConstraint.activate([
      swatchView.centerXAnchor.constraint(equalTo: button.centerXAnchor),
      swatchView.centerYAnchor.constraint(equalTo: button.centerYAnchor),
      swatchView.widthAnchor.constraint(equalToConstant: swatchSize),
      swatchView.heightAnchor.constraint(equalToConstant: swatchSize),
    ])
    swatchView.layer.cornerRadius = swatchSize / 2

    applyAppearance(from: args)
    channel.setMethodCallHandler { [weak self] call, result in
      if call.method == "setContent" {
        self?.setContent(from: call.arguments)
        result(nil)
        return
      }
      guard call.method == "setAppearance" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.applyAppearance(from: call.arguments)
      result(nil)
    }

    containerView.addSubview(button)
    NSLayoutConstraint.activate([
      button.centerYAnchor.constraint(equalTo: containerView.centerYAnchor),
      button.heightAnchor.constraint(equalToConstant: controlSize),
    ])
    if fillWidth {
      button.centerXAnchor.constraint(equalTo: containerView.centerXAnchor).isActive = true
      button.widthAnchor.constraint(equalTo: containerView.widthAnchor).isActive = true
    } else {
      button.centerXAnchor.constraint(
        equalTo: containerView.leadingAnchor,
        constant: buttonCenterX
      ).isActive = true
      button.widthAnchor.constraint(equalToConstant: controlWidth).isActive = true
    }
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
    let busy = arguments?["busy"] as? Bool ?? false
    let selected = arguments?["selected"] as? Bool ?? false
    let swatchColorValue = (arguments?["swatchColor"] as? NSNumber)?.uint32Value

    containerView.overrideUserInterfaceStyle = interfaceStyle
    button.overrideUserInterfaceStyle = interfaceStyle
    button.isEnabled = enabled
    button.isSelected = selected
    isBusy = busy
    if selected {
      button.accessibilityTraits.insert(.selected)
    } else {
      button.accessibilityTraits.remove(.selected)
    }
    button.configuration?.showsActivityIndicator = false
    if let foregroundColor {
      // Glass uses the view tint for its selected treatment. Keep it aligned
      // with the Buzz theme instead of falling back to the system blue tint.
      button.tintColor = foregroundColor
      button.configuration?.baseForegroundColor = foregroundColor
      activityIndicator.color = foregroundColor
    }
    if let swatchColorValue {
      swatchView.backgroundColor = Self.color(from: swatchColorValue)
    }
    if #unavailable(iOS 26.0) {
      button.configuration?.baseBackgroundColor = Self.fallbackBackgroundColor(
        foregroundColor: foregroundColor,
        interfaceStyle: interfaceStyle
      )
    }
    updateDisplayedContent()
    button.setNeedsUpdateConfiguration()
  }

  private func applyContent(from value: Any?) {
    let arguments = value as? [String: Any]
    let icon = arguments?["icon"] as? String
    contentIcon = icon ?? "back"
    buttonLabel = arguments?["label"] as? String
    if let accessibilityLabel = arguments?["accessibilityLabel"] as? String {
      button.accessibilityLabel = accessibilityLabel
    }
    switch icon {
    case "close": buttonIconName = "xmark"
    case "camera": buttonIconName = "camera"
    case "photoLibrary": buttonIconName = "photo.on.rectangle.angled"
    case "palette": buttonIconName = "paintpalette"
    case "droplet": buttonIconName = "drop.fill"
    case "emoji": buttonIconName = "face.smiling"
    case "person": buttonIconName = "person"
    case "frame": buttonIconName = "rectangle.stack"
    case "rotateCamera": buttonIconName = "arrow.triangle.2.circlepath.camera"
    case "shutter": buttonIconName = "circle.fill"
    case "sun": buttonIconName = "sun.max"
    case "moon": buttonIconName = "moon"
    case "systemAppearance": buttonIconName = "circle.lefthalf.filled"
    case "colorSwatch": buttonIconName = "circle.fill"
    default: buttonIconName = "chevron.backward"
    }
    if buttonLabel != nil {
      buttonImage = nil
      button.configuration?.contentInsets = NSDirectionalEdgeInsets(
        top: 8,
        leading: 8,
        bottom: 8,
        trailing: 8
      )
      button.configuration?.titleLineBreakMode = .byClipping
      button.configuration?.titleTextAttributesTransformer =
        UIConfigurationTextAttributesTransformer { incoming in
          var outgoing = incoming
          let preferred = UIFont.preferredFont(forTextStyle: .subheadline)
          outgoing.font = UIFont.systemFont(
            ofSize: preferred.pointSize,
            weight: .semibold
          )
          return outgoing
        }
    } else {
      button.configuration?.titleTextAttributesTransformer = nil
      let iconInset: CGFloat = icon == "shutter"
        ? controlSize * Self.shutterInsetRatio
        : 8
      button.configuration?.contentInsets = NSDirectionalEdgeInsets(
        top: iconInset,
        leading: iconInset,
        bottom: iconInset,
        trailing: iconInset
      )
      let pointSize: CGFloat = icon == "shutter"
        ? controlSize * Self.shutterIconRatio
        : 17
      buttonImage = UIImage(
        systemName: buttonIconName,
        withConfiguration: UIImage.SymbolConfiguration(
          pointSize: pointSize,
          weight: .semibold
        )
      )
    }
    updateDisplayedContent()
    button.setNeedsUpdateConfiguration()
  }

  private func setContent(from value: Any?) {
    let arguments = value as? [String: Any]
    let nextIcon = arguments?["icon"] as? String ?? "back"
    let nextLabel = arguments?["label"] as? String
    guard nextIcon != contentIcon || nextLabel != buttonLabel else {
      applyContent(from: value)
      return
    }
    let duration = UIAccessibility.isReduceMotionEnabled ? 0 : 0.12
    UIView.transition(
      with: button,
      duration: duration,
      options: [.transitionCrossDissolve, .beginFromCurrentState, .allowAnimatedContent],
      animations: { [weak self] in self?.applyContent(from: value) }
    )
  }

  private func updateDisplayedContent() {
    if isBusy {
      button.configuration?.title = nil
      button.configuration?.image = nil
      swatchView.isHidden = true
      activityIndicator.startAnimating()
    } else {
      let displaysSwatch = contentIcon == "colorSwatch"
      button.configuration?.title = displaysSwatch ? nil : buttonLabel
      button.configuration?.image = displaysSwatch || buttonLabel != nil ? nil : buttonImage
      swatchView.isHidden = !displaysSwatch
      activityIndicator.stopAnimating()
    }
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

final class NativeSegmentedControlFactory: NSObject, FlutterPlatformViewFactory {
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
    NativeSegmentedControlPlatformView(
      frame: frame,
      viewIdentifier: viewId,
      arguments: args,
      messenger: messenger
    )
  }
}

final class NativeSegmentedControlPlatformView: NSObject, FlutterPlatformView {
  private let containerView: UIView
  private let channel: FlutterMethodChannel
  private let segmentedControl: UISegmentedControl

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?,
    messenger: FlutterBinaryMessenger
  ) {
    let arguments = args as? [String: Any]
    let items = arguments?["items"] as? [String] ?? []
    containerView = UIView(frame: frame)
    channel = FlutterMethodChannel(
      name: "buzz/native_segmented_control/\(viewId)",
      binaryMessenger: messenger
    )
    segmentedControl = UISegmentedControl(items: items)
    super.init()

    containerView.backgroundColor = .clear
    containerView.isOpaque = false
    segmentedControl.translatesAutoresizingMaskIntoConstraints = false
    segmentedControl.addTarget(
      self,
      action: #selector(selectionChanged),
      for: .valueChanged
    )
    applyState(from: arguments)

    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "setState" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.applyState(from: call.arguments)
      result(nil)
    }

    containerView.addSubview(segmentedControl)
    NSLayoutConstraint.activate([
      segmentedControl.leadingAnchor.constraint(equalTo: containerView.leadingAnchor),
      segmentedControl.trailingAnchor.constraint(equalTo: containerView.trailingAnchor),
      segmentedControl.centerYAnchor.constraint(equalTo: containerView.centerYAnchor),
    ])
  }

  func view() -> UIView {
    containerView
  }

  @objc private func selectionChanged() {
    channel.invokeMethod("changed", arguments: segmentedControl.selectedSegmentIndex)
  }

  private func applyState(from value: Any?) {
    let arguments = value as? [String: Any]
    let selectedIndex = (arguments?["selectedIndex"] as? NSNumber)?.intValue ?? 0
    let brightness = arguments?["brightness"] as? String
    let enabled = arguments?["enabled"] as? Bool ?? true
    let interfaceStyle: UIUserInterfaceStyle = brightness == "dark" ? .dark : .light

    containerView.overrideUserInterfaceStyle = interfaceStyle
    segmentedControl.overrideUserInterfaceStyle = interfaceStyle
    segmentedControl.selectedSegmentIndex = selectedIndex
    segmentedControl.isEnabled = enabled
  }

  deinit {
    channel.setMethodCallHandler(nil)
  }
}
