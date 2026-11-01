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
  var pendingMenuAction: (() -> Void)?
  var menuVisibilityChanged: ((Bool) -> Void)?
  var animateMenu: ((Bool, UIContextMenuInteractionAnimating?) -> Void)?
  weak var glassPreview: UIView?

  override func contextMenuInteraction(
    _ interaction: UIContextMenuInteraction,
    willDisplayMenuFor configuration: UIContextMenuConfiguration,
    animator: UIContextMenuInteractionAnimating?
  ) {
    super.contextMenuInteraction(interaction, willDisplayMenuFor: configuration, animator: animator)
    animateMenu?(true, animator)
    menuVisibilityChanged?(true)
    UIImpactFeedbackGenerator(style: .light).impactOccurred()
  }

  override func contextMenuInteraction(
    _ interaction: UIContextMenuInteraction,
    willEndFor configuration: UIContextMenuConfiguration,
    animator: UIContextMenuInteractionAnimating?
  ) {
    super.contextMenuInteraction(interaction, willEndFor: configuration, animator: animator)
    animateMenu?(false, animator)
    menuVisibilityChanged?(false)
    let action = pendingMenuAction
    pendingMenuAction = nil
    // Flutter routes must start after UIKit removes the glass menu and its
    // source-button preview from the window's overlay hierarchy.
    if let animator {
      animator.addCompletion { action?() }
    } else {
      action?()
    }
  }

  override func contextMenuInteraction(
    _ interaction: UIContextMenuInteraction,
    previewForHighlightingMenuWithConfiguration configuration: UIContextMenuConfiguration
  ) -> UITargetedPreview? {
    guard let glassPreview else {
      return super.contextMenuInteraction(interaction, previewForHighlightingMenuWithConfiguration: configuration)
    }
    return UITargetedPreview(view: glassPreview)
  }

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
    configuration.image = NativeTabler.image("arrow.down", size: 16)
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
  private var navigationGlass: MergingNavigationGlass?
  private let activityIndicator = UIActivityIndicatorView(style: .medium)
  private let swatchView = UIView()
  private var labelForegroundColor: UIColor?
  private var showIconWithLabel = false
  private var labelFontSize: CGFloat?
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

    button.menuVisibilityChanged = { [weak self] visible in
      self?.channel.invokeMethod("menuVisibilityChanged", arguments: visible)
    }
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
    if arguments?["plain"] as? Bool == true || arguments?["mergingNavigation"] as? Bool == true { configuration = .plain() }
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

    if arguments?["mergingNavigation"] as? Bool == true {
      let glass = MergingNavigationGlass(frame: containerView.bounds)
      glass.autoresizingMask = [.flexibleWidth, .flexibleHeight]
      containerView.addSubview(glass)
      navigationGlass = glass
      glass.centerSurface.contentView.addSubview(button)
      button.glassPreview = glass.centerSurface
      button.animateMenu = { [weak glass] visible, animator in
        glass?.setMenuOpen(visible, animator: animator)
      }
      NSLayoutConstraint.activate([
        button.leadingAnchor.constraint(equalTo: glass.centerSurface.contentView.leadingAnchor),
        button.trailingAnchor.constraint(equalTo: glass.centerSurface.contentView.trailingAnchor),
        button.topAnchor.constraint(equalTo: glass.centerSurface.contentView.topAnchor),
        button.bottomAnchor.constraint(equalTo: glass.centerSurface.contentView.bottomAnchor),
      ])
      return
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
    // Resolve label contrast explicitly on the first selected frame as well as
    // later selections; UIKit may retain the initial glass title foreground.
    labelForegroundColor = selected
      ? (interfaceStyle == .dark ? .black : .white)
      : foregroundColor
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
    labelFontSize = (arguments?["labelFontSize"] as? NSNumber).map { CGFloat(truncating: $0) }
    showIconWithLabel = arguments?["showIconWithLabel"] as? Bool == true
    button.configuration?.imagePadding = showIconWithLabel ? 8 : 0
    if let accessibilityLabel = arguments?["accessibilityLabel"] as? String {
      button.accessibilityLabel = accessibilityLabel
    }
    if let menu = arguments?["menu"] as? [[String: Any]] {
      // Keep titled sections in reading order even above a bottom-bar trigger.
      button.preferredMenuElementOrder = menu.contains { $0["sectionTitle"] is String } ? .fixed : .automatic
      var groups: [[UIMenuElement]] = [[]]
      var groupTitles = [""]
      for item in menu {
        if item["dividerBefore"] as? Bool == true || item["sectionTitle"] is String,
           !groups[groups.count - 1].isEmpty {
          groups.append([])
          groupTitles.append("")
        }
        if let title = item["sectionTitle"] as? String {
          groupTitles[groupTitles.count - 1] = title
        }
        let symbol = (item["symbol"] as? String).flatMap {
          // A transparent image keeps the native menu's icon column aligned.
          $0.isEmpty
            ? UIGraphicsImageRenderer(size: CGSize(width: 24, height: 24)).image { _ in }
            : NativeTabler.image($0, size: 24)
        }
        let action = UIAction(title: item["label"] as? String ?? "", image: symbol,
                        attributes: item["enabled"] as? Bool == true ? [] : [.disabled],
                        state: item["checked"] as? Bool == true ? .on : .off) { [weak self] _ in
          let select = { [weak self] in
            self?.channel.invokeMethod("menuAction", arguments: item["id"])
          }
          if arguments?["deferMenuActions"] as? Bool == false {
            // Value changes do not navigate. Deliver them directly: UIKit may
            // begin dismissing the menu before invoking its selected UIAction.
            select()
          } else {
            self?.button.pendingMenuAction = { select() }
          }
        }
        groups[groups.count - 1].append(action)
      }
      button.menu = UIMenu(children: groups.enumerated().map { index, children in
        UIMenu(title: groupTitles[index], options: .displayInline, children: children)
      })
      button.showsMenuAsPrimaryAction = !menu.isEmpty
    }
    switch icon {
    case "plus": buttonIconName = "plus"
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
    case "trash": buttonIconName = "trash"
    case "more": buttonIconName = "ellipsis"
    case "reply": buttonIconName = "arrowshape.turn.up.left"
    case "systemAppearance": buttonIconName = "circle.lefthalf.filled"
    case "colorSwatch": buttonIconName = "circle.fill"
    default: buttonIconName = "chevron.backward"
    }
    if buttonLabel != nil {
      buttonImage = showIconWithLabel ? NativeTabler.image(buttonIconName, size: 24) : nil
      button.configuration?.contentInsets = NSDirectionalEdgeInsets(
        top: 8,
        leading: 8,
        bottom: 8,
        trailing: 8
      )
      button.configuration?.titleLineBreakMode = .byClipping
      button.configuration?.titleTextAttributesTransformer =
        UIConfigurationTextAttributesTransformer { [weak self] incoming in
          var outgoing = incoming
          if let foreground = self?.labelForegroundColor {
            outgoing.foregroundColor = foreground
          }
          let preferred = UIFont.preferredFont(forTextStyle: .subheadline)
          outgoing.font = UIFont.systemFont(
            ofSize: self?.labelFontSize.map { UIFontMetrics(forTextStyle: .subheadline).scaledValue(for: $0) } ?? preferred.pointSize,
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
        : 24
      if icon == "shutter" {
        // The shutter is a solid disc, not Tabler's outlined circle glyph.
        buttonImage = UIGraphicsImageRenderer(size: CGSize(width: pointSize, height: pointSize)).image { _ in
          UIColor.white.setFill()
          UIBezierPath(ovalIn: CGRect(x: 0, y: 0, width: pointSize, height: pointSize)).fill()
        }.withRenderingMode(.alwaysTemplate)
      } else {
        buttonImage = NativeTabler.image(buttonIconName, size: pointSize)
      }
    }
    if let bytes = arguments?["imageBytes"] as? FlutterStandardTypedData,
       let image = UIImage(data: bytes.data) {
      let size = CGSize(width: 36, height: 36)
      buttonImage = UIGraphicsImageRenderer(size: size).image { _ in
        image.draw(in: CGRect(origin: .zero, size: size))
      }.withRenderingMode(.alwaysOriginal)
      button.configuration?.contentInsets = .zero
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
      let image = button.isSelected
        ? buttonImage?.withTintColor(labelForegroundColor ?? .label, renderingMode: .alwaysOriginal)
        : buttonImage
      button.configuration?.image = displaysSwatch || (buttonLabel != nil && !showIconWithLabel) ? nil : image
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

/// Three native glass elements share a compositor so their surfaces merge.
final class MergingNavigationGlass: UIView {
  let leftSurface: UIVisualEffectView
  let centerSurface: UIVisualEffectView
  let rightSurface: UIVisualEffectView
  private let group: UIVisualEffectView
  private var movementAnimator: UIViewPropertyAnimator?

  override init(frame: CGRect) {
    if #available(iOS 26.0, *) {
      let container = UIGlassContainerEffect()
      container.spacing = 12
      group = UIVisualEffectView(effect: container)
      leftSurface = UIVisualEffectView(effect: UIGlassEffect(style: .regular))
      centerSurface = UIVisualEffectView(effect: UIGlassEffect(style: .regular))
      rightSurface = UIVisualEffectView(effect: UIGlassEffect(style: .regular))
    } else {
      group = UIVisualEffectView(effect: nil)
      leftSurface = UIVisualEffectView(effect: UIBlurEffect(style: .systemMaterial))
      centerSurface = UIVisualEffectView(effect: UIBlurEffect(style: .systemMaterial))
      rightSurface = UIVisualEffectView(effect: UIBlurEffect(style: .systemMaterial))
    }
    super.init(frame: frame)
    group.frame = bounds
    group.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    addSubview(group)
    for surface in [leftSurface, rightSurface, centerSurface] {
      if #available(iOS 26.0, *) {
        surface.cornerConfiguration = .capsule()
      } else {
        surface.layer.cornerRadius = 28
        surface.clipsToBounds = true
      }
      group.contentView.addSubview(surface)
    }
    leftSurface.isUserInteractionEnabled = false
    rightSurface.isUserInteractionEnabled = false
  }

  required init?(coder: NSCoder) { nil }

  override func layoutSubviews() {
    super.layoutSubviews()
    // Use bounds/center rather than frame: side views may be transformed.
    leftSurface.bounds = CGRect(x: 0, y: 0, width: 56, height: 56)
    rightSurface.bounds = leftSurface.bounds
    centerSurface.bounds = CGRect(x: 0, y: 0, width: 112, height: 56)
    leftSurface.center = CGPoint(x: 28, y: bounds.midY)
    rightSurface.center = CGPoint(x: bounds.width - 28, y: bounds.midY)
    centerSurface.center = CGPoint(x: bounds.midX, y: bounds.midY)
  }

  func setMenuOpen(_ open: Bool, animator: UIContextMenuInteractionAnimating?) {
    layoutIfNeeded()
    let movement = bounds.midX - 28
    let changes = { [weak self] in
      guard let self else { return }
      self.leftSurface.transform = open
        ? CGAffineTransform(translationX: movement, y: 0).scaledBy(x: 0.9, y: 0.9) : .identity
      self.rightSurface.transform = open
        ? CGAffineTransform(translationX: -movement, y: 0).scaledBy(x: 0.9, y: 0.9) : .identity
    }
    let opacity = { [weak self] in
      self?.leftSurface.alpha = open ? 0 : 1
      self?.rightSurface.alpha = open ? 0 : 1
    }
    // The live Flutter artwork uses this same 180 ms easeOutCubic curve.
    // UIKit's menu animator has different timing, which makes the glass outrun
    // its artwork. Start both from the visibility callback instead.
    if let movementAnimator, movementAnimator.state == .active {
      movementAnimator.stopAnimation(false)
      movementAnimator.finishAnimation(at: .current)
    }
    movementAnimator = nil
    if UIAccessibility.isReduceMotionEnabled {
      UIView.performWithoutAnimation {
        changes()
        opacity()
      }
    } else {
      let timing = UICubicTimingParameters(
        controlPoint1: CGPoint(x: 0.215, y: 0.61),
        controlPoint2: CGPoint(x: 0.355, y: 1)
      )
      let movement = UIViewPropertyAnimator(duration: 0.18, timingParameters: timing)
      movement.addAnimations(changes)
      movement.addAnimations(opacity, delayFactor: open ? 0.5 : 0)
      movementAnimator = movement
      movement.startAnimation()
    }
  }
}
