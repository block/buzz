import Flutter
import UIKit

final class ThemePaginationGlassControlFactory: NSObject, FlutterPlatformViewFactory {
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
    ThemePaginationGlassControlPlatformView(
      frame: frame,
      viewIdentifier: viewId,
      arguments: args,
      messenger: messenger
    )
  }
}

private final class ThemePaginationControl: UIControl {
  private static let fullDotSize = ThemePaginationGeometry.dotSize
  private static let selectedDotSize: CGFloat = 10
  private let glassView: UIVisualEffectView
  private let dotsContainer = UIView()
  private var glassHeightConstraint: NSLayoutConstraint?
  private var dots: [UIView] = []
  private var totalCount = 1
  private var selectedIndex = 0
  private var scrub = ThemePaginationScrub()
  private var activeColor = UIColor.label
  private var inactiveColor = UIColor.secondaryLabel.withAlphaComponent(0.32)
  var onSelectionChanged: ((Int) -> Void)?

  override init(frame: CGRect) {
    if #available(iOS 26.0, *) {
      let effect = UIGlassEffect(style: .regular)
      effect.isInteractive = true
      glassView = UIVisualEffectView(effect: effect)
    } else {
      glassView = UIVisualEffectView(effect: UIBlurEffect(style: .systemMaterial))
    }
    super.init(frame: frame)

    backgroundColor = .clear
    isOpaque = false
    isAccessibilityElement = true
    accessibilityTraits = [.adjustable]
    accessibilityLabel = "Theme"

    glassView.translatesAutoresizingMaskIntoConstraints = false
    glassView.clipsToBounds = true
    glassView.layer.cornerCurve = .continuous
    glassView.isUserInteractionEnabled = false
    addSubview(glassView)

    dotsContainer.translatesAutoresizingMaskIntoConstraints = false
    dotsContainer.clipsToBounds = true
    dotsContainer.isUserInteractionEnabled = false
    glassView.contentView.addSubview(dotsContainer)

    let glassHeightConstraint = glassView.heightAnchor.constraint(equalToConstant: 30)
    self.glassHeightConstraint = glassHeightConstraint
    NSLayoutConstraint.activate([
      glassView.leadingAnchor.constraint(equalTo: leadingAnchor),
      glassView.trailingAnchor.constraint(equalTo: trailingAnchor),
      glassView.centerYAnchor.constraint(equalTo: centerYAnchor),
      glassHeightConstraint,
      dotsContainer.leadingAnchor.constraint(equalTo: glassView.contentView.leadingAnchor, constant: 12),
      dotsContainer.trailingAnchor.constraint(equalTo: glassView.contentView.trailingAnchor, constant: -12),
      dotsContainer.topAnchor.constraint(equalTo: glassView.contentView.topAnchor),
      dotsContainer.bottomAnchor.constraint(equalTo: glassView.contentView.bottomAnchor),
    ])

    addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(handleGesture(_:))))
    addGestureRecognizer(UIPanGestureRecognizer(target: self, action: #selector(handleGesture(_:))))
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    nil
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    glassView.layer.cornerRadius = glassView.bounds.height / 2
    glassView.layoutIfNeeded()
    dotsContainer.layoutIfNeeded()
    updateDots(animated: false)
  }

  func apply(arguments: [String: Any], animated: Bool) {
    let count = max(1, (arguments["count"] as? NSNumber)?.intValue ?? totalCount)
    let selected = min(
      max(0, (arguments["selected"] as? NSNumber)?.intValue ?? selectedIndex),
      count - 1
    )
    if let value = arguments["activeColor"] as? NSNumber {
      activeColor = Self.color(fromARGB: value.uint32Value)
    }
    if let value = arguments["inactiveColor"] as? NSNumber {
      inactiveColor = Self.color(fromARGB: value.uint32Value)
    }

    accessibilityLabel = arguments["accessibilityLabel"] as? String ?? accessibilityLabel
    if let height = arguments["containerHeight"] as? NSNumber {
      glassHeightConstraint?.constant = max(10, CGFloat(height.doubleValue))
      setNeedsLayout()
    }
    if let isRTL = arguments["isRTL"] as? Bool {
      semanticContentAttribute = isRTL ? .forceRightToLeft : .forceLeftToRight
    }
    if count != totalCount { scrub.end() }
    totalCount = count
    if count != dots.count {
      rebuildDots(count: count)
    }
    selectedIndex = selected
    accessibilityValue = "\(selected + 1) of \(count)"
    let animateChanges = (arguments["animateChanges"] as? Bool) ?? true
    updateDots(animated: animated && animateChanges)
  }

  override func accessibilityIncrement() {
    select(index: min(selectedIndex + 1, totalCount - 1))
  }

  override func accessibilityDecrement() {
    select(index: max(selectedIndex - 1, 0))
  }

  private var geometry: ThemePaginationGeometry {
    ThemePaginationGeometry(
      count: totalCount, selected: selectedIndex, width: dotsContainer.bounds.width,
      isRTL: effectiveUserInterfaceLayoutDirection == .rightToLeft
    )
  }

  @objc private func handleGesture(_ recognizer: UIGestureRecognizer) {
    guard dotsContainer.bounds.width > 0 else { return }
    let location = recognizer.location(in: dotsContainer)
    if recognizer is UIPanGestureRecognizer {
      switch recognizer.state {
      case .began:
        scrub.begin(geometry)
      case .changed:
        guard scrub.geometry != nil else { return }
      case .ended:
        let page = scrub.page(at: location.x, current: geometry)
        scrub.end()
        select(index: page)
        updateDots(animated: true)
        return
      case .cancelled, .failed:
        scrub.end()
        updateDots(animated: true)
        return
      default:
        return
      }
      select(index: scrub.page(at: location.x, current: geometry))
    } else if recognizer.state == .ended {
      select(index: geometry.page(at: location.x))
    }
  }

  private func select(index: Int) {
    guard index != selectedIndex, (0..<totalCount).contains(index) else { return }
    selectedIndex = index
    accessibilityValue = "\(index + 1) of \(totalCount)"
    updateDots(animated: true)
    onSelectionChanged?(index)
  }

  private func rebuildDots(count: Int) {
    dots.forEach { dot in
      dot.removeFromSuperview()
    }
    dots = (0..<count).map { _ in
      let dot = UIView()
      dot.bounds = CGRect(
        x: 0,
        y: 0,
        width: Self.fullDotSize,
        height: Self.fullDotSize
      )
      dot.layer.cornerRadius = Self.fullDotSize / 2
      dot.layer.cornerCurve = .continuous
      dot.isUserInteractionEnabled = false
      dotsContainer.addSubview(dot)
      return dot
    }
  }

  private func updateDots(animated: Bool) {
    guard !dots.isEmpty, dotsContainer.bounds.width > 0 else { return }
    let geometry = scrub.geometry ?? self.geometry
    let visibleCount = geometry.visibleCount
    let hasEarlierDots = geometry.isRTL ? geometry.hasLaterDots : geometry.hasEarlierDots
    let hasLaterDots = geometry.isRTL ? geometry.hasEarlierDots : geometry.hasLaterDots
    let changes = {
      for (page, dot) in self.dots.enumerated() {
        let slot = geometry.slot(for: page)
        let isVisible = (0..<visibleCount).contains(slot)
        var diameter = Self.fullDotSize
        if page == self.selectedIndex {
          diameter = Self.selectedDotSize
        } else if (hasEarlierDots && slot == 0) ||
          (hasLaterDots && slot == visibleCount - 1)
        {
          diameter = 2
        } else if (hasEarlierDots && slot == 1) ||
          (hasLaterDots && slot == visibleCount - 2)
        {
          diameter = 4
        }
        dot.center = CGPoint(
          x: geometry.centerX(for: page),
          y: self.dotsContainer.bounds.midY
        )
        dot.alpha = isVisible ? 1 : 0
        dot.backgroundColor = page == self.selectedIndex
          ? self.activeColor
          : self.inactiveColor
        dot.transform = CGAffineTransform(
          scaleX: diameter / Self.fullDotSize,
          y: diameter / Self.fullDotSize
        )
        dot.layer.zPosition = page == self.selectedIndex ? 1 : 0
      }
    }
    guard animated, !UIAccessibility.isReduceMotionEnabled else {
      changes()
      return
    }
    UIView.animate(
      withDuration: 0.15,
      delay: 0,
      options: [.beginFromCurrentState, .allowUserInteraction, .curveEaseInOut],
      animations: changes
    )
  }

  private static func color(fromARGB value: UInt32) -> UIColor {
    UIColor(
      red: CGFloat((value >> 16) & 0xFF) / 255,
      green: CGFloat((value >> 8) & 0xFF) / 255,
      blue: CGFloat(value & 0xFF) / 255,
      alpha: CGFloat((value >> 24) & 0xFF) / 255
    )
  }
}

final class ThemePaginationGlassControlPlatformView: NSObject, FlutterPlatformView {
  private let control: ThemePaginationControl
  private let channel: FlutterMethodChannel

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?,
    messenger: FlutterBinaryMessenger
  ) {
    control = ThemePaginationControl(frame: frame)
    channel = FlutterMethodChannel(
      name: "buzz/theme_pagination_glass/\(viewId)",
      binaryMessenger: messenger
    )
    super.init()

    let arguments = args as? [String: Any] ?? [:]
    applyBrightness(from: arguments["brightness"])
    control.apply(arguments: arguments, animated: false)
    control.onSelectionChanged = { [weak self] index in
      self?.channel.invokeMethod("selected", arguments: index)
    }

    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "setState", let arguments = call.arguments as? [String: Any] else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.applyBrightness(from: arguments["brightness"])
      self?.control.apply(arguments: arguments, animated: true)
      result(nil)
    }
  }

  func view() -> UIView {
    control
  }

  private func applyBrightness(from value: Any?) {
    let style: UIUserInterfaceStyle = value as? String == "dark" ? .dark : .light
    control.overrideUserInterfaceStyle = style
  }

  deinit {
    channel.setMethodCallHandler(nil)
  }
}
