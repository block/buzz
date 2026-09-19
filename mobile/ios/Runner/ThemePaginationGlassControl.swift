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
  private static let maximumVisibleDots = 7
  private static let fullDotSize: CGFloat = 6
  private static let selectedDotSize: CGFloat = 10
  private static let dotSpacing: CGFloat = 6
  private let glassView: UIVisualEffectView
  private let dotsContainer = UIView()
  private var dots: [UIView] = []
  private var totalCount = 1
  private var selectedIndex = 0
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

    NSLayoutConstraint.activate([
      glassView.leadingAnchor.constraint(equalTo: leadingAnchor),
      glassView.trailingAnchor.constraint(equalTo: trailingAnchor),
      glassView.centerYAnchor.constraint(equalTo: centerYAnchor),
      glassView.heightAnchor.constraint(equalToConstant: 30),
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

  @objc private func handleGesture(_ recognizer: UIGestureRecognizer) {
    guard totalCount > 0 else { return }
    let location = recognizer.location(in: glassView)
    let progress = max(0, min(0.999_999, location.x / max(1, glassView.bounds.width)))
    select(index: Int(progress * CGFloat(totalCount)))
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
    let visibleCount = min(totalCount, Self.maximumVisibleDots)
    let maximumStart = max(0, totalCount - visibleCount)
    let centerSlot = visibleCount / 2
    let windowStart = min(max(0, selectedIndex - centerSlot), maximumStart)
    let windowEnd = windowStart + visibleCount - 1
    let hasEarlierDots = windowStart > 0
    let hasLaterDots = windowEnd < totalCount - 1
    let pitch = Self.fullDotSize + Self.dotSpacing
    let trackWidth = CGFloat(visibleCount) * Self.fullDotSize
      + CGFloat(max(0, visibleCount - 1)) * Self.dotSpacing
    let trackOrigin = (dotsContainer.bounds.width - trackWidth) / 2
    let changes = {
      for (page, dot) in self.dots.enumerated() {
        let slot = page - windowStart
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
          x: trackOrigin + Self.fullDotSize / 2 + CGFloat(slot) * pitch,
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
