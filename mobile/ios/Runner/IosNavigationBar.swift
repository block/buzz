import Flutter
import UIKit

// UIKit 9127.0.85 is the iOS 27.1 SDK with Duo APIs. SDK checks must
// surround runtime availability checks: older SDKs cannot resolve these symbols.

/// Each Flutter route owns a UIKit navigation controller. Its private scroll
/// view mirrors the Flutter viewport so UIKit performs the large-title layout.
final class IosNavigationBarFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger
  private weak var parent: UIViewController?

  init(messenger: FlutterBinaryMessenger, parent: UIViewController?) {
    self.messenger = messenger
    self.parent = parent
    super.init()
  }

  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }

  func create(withFrame frame: CGRect, viewIdentifier viewId: Int64, arguments args: Any?) -> FlutterPlatformView {
    IosNavigationBarView(frame: frame, id: viewId, args: args, messenger: messenger, parent: parent)
  }
}

/// A thin bar along the bottom inner edge of the title capsule while the page
/// reconnects or catches up. Its duration is unknown, so it sweeps in and then
/// pulses until the refresh ends, then fades. The navigation view owns it so
/// it survives title rebuilds.
final class NavigationRefreshBar: UIView {
  private static let pulseKey = "refresh-pulse"
  private let fill = UIView()
  private(set) var isRefreshing = false

  override init(frame: CGRect) {
    super.init(frame: frame)
    isUserInteractionEnabled = false
    isAccessibilityElement = false
    accessibilityIdentifier = "channel-navigation-refresh"
    fill.layer.anchorPoint = CGPoint(x: 0, y: 0.5)
    addSubview(fill)
    alpha = 0
  }

  required init?(coder: NSCoder) { return nil }

  var color: UIColor? {
    get { fill.backgroundColor }
    set { fill.backgroundColor = newValue }
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    let transform = fill.transform
    fill.transform = .identity
    fill.frame = bounds
    fill.transform = transform
  }

  // Moving between title views drops layer animations; restart the pulse.
  override func didMoveToWindow() {
    super.didMoveToWindow()
    if window != nil, isRefreshing { startPulse() }
  }

  func setRefreshing(_ refreshing: Bool) {
    guard refreshing != isRefreshing else { return }
    isRefreshing = refreshing
    let reduceMotion = UIAccessibility.isReduceMotionEnabled
    layer.removeAllAnimations()
    fill.layer.removeAllAnimations()
    if refreshing {
      alpha = 1
      guard !reduceMotion else {
        fill.transform = .identity
        return
      }
      fill.transform = CGAffineTransform(scaleX: 0.001, y: 1)
      UIView.animate(withDuration: 0.35, delay: 0, options: [.curveEaseOut]) {
        self.fill.transform = .identity
      }
      startPulse()
    } else {
      fill.transform = .identity
      UIView.animate(withDuration: reduceMotion ? 0 : 0.25, delay: 0, options: [.beginFromCurrentState]) {
        self.alpha = 0
      }
    }
  }

  private func startPulse() {
    guard !UIAccessibility.isReduceMotionEnabled,
          fill.layer.animation(forKey: Self.pulseKey) == nil else { return }
    let pulse = CABasicAnimation(keyPath: "opacity")
    pulse.fromValue = 1
    pulse.toValue = 0.3
    pulse.duration = 0.75
    pulse.autoreverses = true
    pulse.repeatCount = .infinity
    pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
    pulse.beginTime = CACurrentMediaTime() + 0.35
    pulse.fillMode = .backwards
    fill.layer.add(pulse, forKey: Self.pulseKey)
  }
}

final class NavigationTitleView: UIVisualEffectView, UIGestureRecognizerDelegate {
  var onActivate: (() -> Void)?
  private let titleLabel = UILabel()
  private let subtitleLabel = UILabel()
  private let contentStack = UIStackView()
  private let textStack = UIStackView()
  private let subtitleStack = UIStackView()
  private let maximumWidth: CGFloat?

  init(title: String?, subtitle: String, color: UIColor, maximumWidth: CGFloat? = nil) {
    self.maximumWidth = maximumWidth
    if #available(iOS 26.0, *) {
      super.init(effect: UIGlassEffect(style: .regular))
    } else {
      super.init(effect: UIBlurEffect(style: .systemMaterial))
    }
    clipsToBounds = true
    layer.cornerCurve = .continuous
    titleLabel.text = title
    titleLabel.textColor = color
    subtitleStack.isHidden = subtitle.isEmpty
    subtitleLabel.text = subtitle
    subtitleLabel.textColor = .secondaryLabel
    for label in [titleLabel, subtitleLabel] {
      label.textAlignment = .center
      label.lineBreakMode = .byTruncatingTail
      label.adjustsFontForContentSizeCategory = false
      label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    }
    subtitleStack.axis = .horizontal
    subtitleStack.alignment = .center
    subtitleStack.spacing = 6
    subtitleStack.addArrangedSubview(subtitleLabel)
    textStack.axis = .vertical
    textStack.alignment = .center
    textStack.addArrangedSubview(titleLabel)
    textStack.addArrangedSubview(subtitleStack)
    contentStack.axis = .horizontal
    contentStack.alignment = .center
    contentStack.spacing = 8
    contentStack.addArrangedSubview(textStack)
    contentStack.translatesAutoresizingMaskIntoConstraints = false
    contentView.addSubview(contentStack)
    NSLayoutConstraint.activate([
      contentStack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 12),
      contentStack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -12),
      contentStack.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
      titleLabel.widthAnchor.constraint(lessThanOrEqualTo: contentView.widthAnchor, constant: -24),
      subtitleStack.widthAnchor.constraint(lessThanOrEqualTo: contentView.widthAnchor, constant: -24),
    ])
    translatesAutoresizingMaskIntoConstraints = false
    if let maximumWidth {
      widthAnchor.constraint(lessThanOrEqualToConstant: max(0, maximumWidth)).isActive = true
    }
    updateFonts()
    // Flutter creates platform views with a zero frame. Keep a nonzero
    // intrinsic width and let UINavigationBar compress it between its items.
    setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    isAccessibilityElement = true
    accessibilityTraits = .button
    let tap = UITapGestureRecognizer(target: self, action: #selector(activate))
    tap.delegate = self
    addGestureRecognizer(tap)
  }

  func setAvatar(_ image: UIImage?, presence: UIColor?) {
    // Bar-item images carry alignment insets; the title owns its 32pt layout.
    let avatar = UIImageView(image: image?.withAlignmentRectInsets(.zero))
    avatar.accessibilityIdentifier = "dm-navigation-avatar"
    avatar.contentMode = .scaleAspectFit
    let avatarContainer = UIView()
    avatar.translatesAutoresizingMaskIntoConstraints = false
    avatarContainer.addSubview(avatar)
    contentStack.insertArrangedSubview(avatarContainer, at: 0)
    NSLayoutConstraint.activate([
      avatarContainer.widthAnchor.constraint(equalToConstant: 32),
      avatarContainer.heightAnchor.constraint(equalToConstant: 32),
      avatar.leadingAnchor.constraint(equalTo: avatarContainer.leadingAnchor),
      avatar.trailingAnchor.constraint(equalTo: avatarContainer.trailingAnchor),
      avatar.topAnchor.constraint(equalTo: avatarContainer.topAnchor),
      avatar.bottomAnchor.constraint(equalTo: avatarContainer.bottomAnchor),
    ])
    textStack.alignment = .leading
    for label in [titleLabel, subtitleLabel] { label.textAlignment = .natural }
    if let presence {
      let badge = UIView()
      badge.backgroundColor = presence
      badge.layer.cornerRadius = 4
      badge.layer.borderWidth = 1.5
      badge.layer.borderColor = UIColor.systemBackground.cgColor
      badge.accessibilityIdentifier = "dm-navigation-presence"
      badge.translatesAutoresizingMaskIntoConstraints = false
      avatarContainer.addSubview(badge)
      NSLayoutConstraint.activate([
        badge.widthAnchor.constraint(equalToConstant: 8),
        badge.heightAnchor.constraint(equalToConstant: 8),
        badge.trailingAnchor.constraint(equalTo: avatarContainer.trailingAnchor),
        badge.bottomAnchor.constraint(equalTo: avatarContainer.bottomAnchor),
      ])
    }
    invalidateIntrinsicContentSize()
  }

  func setSubtitlePresence(_ color: UIColor) {
    let dot = UIView()
    dot.backgroundColor = color
    dot.layer.cornerRadius = 3
    dot.isAccessibilityElement = false
    dot.accessibilityIdentifier = "dm-navigation-status-dot"
    subtitleStack.insertArrangedSubview(dot, at: 0)
    NSLayoutConstraint.activate([
      dot.widthAnchor.constraint(equalToConstant: 6),
      dot.heightAnchor.constraint(equalToConstant: 6),
    ])
    invalidateIntrinsicContentSize()
  }

  func setEphemeralStatus(_ label: String) {
    // The subtitle carries the visible temporary/expiry text. Keep the full
    // retention explanation on the title's single VoiceOver element too.
    accessibilityLabel = [accessibilityLabel, label].compactMap { $0 }.joined(separator: ", ")
  }

  func attachRefreshBar(_ bar: NavigationRefreshBar) {
    bar.removeFromSuperview()
    bar.translatesAutoresizingMaskIntoConstraints = false
    contentView.addSubview(bar)
    NSLayoutConstraint.activate([
      bar.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
      bar.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
      bar.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
      bar.heightAnchor.constraint(equalToConstant: 2),
    ])
  }

  func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
    acceptsTitleTouch(in: touch.view)
  }

  // UIKit can wrap titleView in a UIControl. Only controls inside our title
  // should consume the title tap.
  func acceptsTitleTouch(in touchedView: UIView?) -> Bool {
    var touched = touchedView
    while let view = touched, view !== self {
      if view is UIControl { return false }
      touched = view.superview
    }
    return true
  }

  required init?(coder: NSCoder) { return nil }

  @objc private func activate() { onActivate?() }

  override func accessibilityActivate() -> Bool {
    guard isUserInteractionEnabled, let onActivate else { return false }
    onActivate()
    return true
  }

  override var intrinsicContentSize: CGSize {
    let contentSize = contentStack.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize)
    return CGSize(width: min(contentSize.width + 24, maximumWidth ?? .greatestFiniteMagnitude),
                  height: max(44, contentSize.height))
  }

  override func traitCollectionDidChange(_ previousTraitCollection: UITraitCollection?) {
    super.traitCollectionDidChange(previousTraitCollection)
    if previousTraitCollection?.preferredContentSizeCategory != traitCollection.preferredContentSizeCategory {
      updateFonts()
      invalidateIntrinsicContentSize()
      setNeedsLayout()
    }
  }

  override func sizeThatFits(_ size: CGSize) -> CGSize {
    let desired = intrinsicContentSize
    return CGSize(width: min(size.width, desired.width), height: desired.height)
  }

  private func updateFonts() {
    // Compact navigation remains a 44pt toolbar. Scale within that budget;
    // the complete title/subtitle remains available as one VoiceOver label.
    titleLabel.font = UIFontMetrics(forTextStyle: .headline).scaledFont(
      for: .systemFont(ofSize: 17, weight: .semibold), maximumPointSize: 20,
      compatibleWith: traitCollection)
    subtitleLabel.font = UIFontMetrics(forTextStyle: .caption1).scaledFont(
      for: .systemFont(ofSize: 12), maximumPointSize: 14,
      compatibleWith: traitCollection)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    layer.cornerRadius = bounds.height / 2
  }
}

private final class NavigationClipView: UIView {
  var onLayout: (() -> Void)?
  override func safeAreaInsetsDidChange() {
    super.safeAreaInsetsDidChange()
    setNeedsLayout()
  }
  override func layoutSubviews() {
    super.layoutSubviews()
    onLayout?()
  }
}

private final class NavigationContentController: UIViewController {
  let scrollView = UIScrollView()
  override func loadView() {
    view = scrollView
    scrollView.contentSize = CGSize(width: 1, height: 10000)
    scrollView.isUserInteractionEnabled = false
    scrollView.scrollsToTop = false
    scrollView.backgroundColor = .clear
    if #available(iOS 26.0, *) {
      // Configure the native effect before UIKit lays out the bar; the
      // platform-view configuration selects it or the material fallback.
      scrollView.topEdgeEffect.style = .soft
      scrollView.bottomEdgeEffect.isHidden = true
    }
  }
}

private final class IosNavigationBarView: NSObject, FlutterPlatformView {
  /// Far beyond any title collapse range, well within the mirror's content.
  private static let maxTrackedDepth: CGFloat = 1000
  private let container: NavigationClipView
  private let material = UIVisualEffectView(effect: UIBlurEffect(style: .systemUltraThinMaterial))
  private let materialFade = CAGradientLayer()
  private let content = NavigationContentController()
  private let navigation: UINavigationController
  private let channel: FlutterMethodChannel
  private var offset: CGFloat = 0
  private var controlBackground = UIColor.secondarySystemBackground
  private var foldRefresh: DispatchWorkItem?
  private var standaloneControl = false
  private var standaloneLargeTitle = false
  private var standaloneSurface: UIView?
  private var horizontalControls = false
  private var headerLeading: UIView?
  private var headerTrailing: UIView?
  private var paneTitle: UIView?
  private var titleInsetLeft: CGFloat = 0
  private var titleInsetRight: CGFloat = 0
  private var alignCollapsedTitleToLeading = false
  private var alwaysFrosted = false
  private var backdropOnly = false
  private var usesSystemScrollEdge = false
  private var expandedBarHeight: CGFloat = 0
  private var titleIsCollapsed: Bool?
  private let compactTitle = UILabel()
  private let compactTitleContainer = UIStackView()
  private let compactTitleMask = CALayer()
  private var titleColor = UIColor.label
  private var largeTitleOpacity: CGFloat = -1
  private var measuredWidth: CGFloat = 0
  private var measuredSafeTop: CGFloat = -1
  private var measuredCategory: UIContentSizeCategory?
  private var measuring = false
  private var metrics: [String: Any]?
  private weak var trackedAvatar: UIButton?
  private var reportedAvatarFrame: CGRect?
  private var flutterSafeTop: CGFloat = 0
  private let refreshBar = NavigationRefreshBar()
  private var floatingTitle: NavigationTitleView?
  private var titleMaxWidth: CGFloat?
  private var floatingTitleLift: CGFloat = 0
  private var portraitTopPadding: CGFloat = 0
  private var centerTitleInPane = false

  init(frame: CGRect, id: Int64, args: Any?, messenger: FlutterBinaryMessenger, parent: UIViewController?) {
    container = NavigationClipView(frame: frame)
    navigation = UINavigationController(rootViewController: content)
    channel = FlutterMethodChannel(name: "buzz/ios_navigation_bar/\(id)", binaryMessenger: messenger)
    super.init()
    container.clipsToBounds = true
    container.backgroundColor = .clear
    material.isUserInteractionEnabled = false
    material.accessibilityIdentifier = "navigation-scroll-material"
    material.alpha = 0
    // Keep the scroll-edge backdrop light and let it fade into the page
    // instead of drawing a uniformly frosted rectangular toolbar.
    materialFade.colors = [UIColor.black.cgColor, UIColor.black.cgColor, UIColor.clear.cgColor]
    materialFade.locations = [0, 0.55, 1]
    material.layer.mask = materialFade
    container.addSubview(material)
    navigation.view.backgroundColor = .clear
    navigation.navigationBar.isTranslucent = true
    parent?.addChild(navigation)
    container.addSubview(navigation.view)
    navigation.didMove(toParent: parent)
    container.onLayout = { [weak self] in self?.layout() }
    channel.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "configure": self?.configure(call.arguments as? [String: Any] ?? [:])
      case "prepareForReveal":
        UIView.performWithoutAnimation {
          self?.navigation.view.setNeedsLayout()
          self?.navigation.navigationBar.setNeedsLayout()
          self?.container.setNeedsLayout()
          self?.container.layoutIfNeeded()
        }
      case "scroll":
        self?.setScrollOffset((call.arguments as? NSNumber)?.doubleValue ?? 0)
      default: result(FlutterMethodNotImplemented); return
      }
      result(nil)
    }
    configure(args as? [String: Any] ?? [:])
    #if canImport(UIKit, _version: 9127.0.85)
    if #available(iOS 27.1, *), standaloneControl {
      container.addInteraction(UIHingeInteraction { [weak self] _, _ in
        guard let self else { return }
        self.container.setNeedsLayout()
        // Reserved divisions settle shortly after the physical hinge does.
        self.foldRefresh?.cancel()
        let refresh = DispatchWorkItem { [weak self] in self?.container.setNeedsLayout() }
        self.foldRefresh = refresh
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.2, execute: refresh)
      })
    }
    #endif
  }

  func view() -> UIView { container }

  private func layout() {
    if standaloneLargeTitle, let scroll = standaloneSurface as? UIScrollView,
       let label = scroll.subviews.compactMap({ $0 as? UILabel }).first {
      scroll.frame = container.bounds
      let width = max(0, scroll.bounds.width - 32)
      let size = label.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))
      // Match the pane's 44-point navigation row plus its 24-point top spacing.
      label.frame = CGRect(x: 16, y: flutterSafeTop + 68, width: width, height: size.height)
      scroll.contentSize = CGSize(width: scroll.bounds.width, height: label.frame.maxY + 24)
      return
    }
    if standaloneControl {
      reportRailGeometry()
      if let surface = standaloneSurface {
        let size = surface.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize)
        surface.frame = CGRect(x: (container.bounds.width - size.width) / 2,
                               y: (container.bounds.height - size.height) / 2,
                               width: size.width, height: size.height)
        surface.layoutIfNeeded()
        reportAvatarBounds()
      }
      return
    }
    // Only the bar is exposed by the platform-view clip. A full viewport is
    // needed for UIKit's scroll-edge and large-title calculations.
    guard !measuring else { return }
    let viewportHeight = container.window?.bounds.height ?? container.bounds.height
    navigation.view.frame = CGRect(x: 0, y: 0, width: container.bounds.width, height: viewportHeight)
    // Flutter's route can begin offscreen, where UIKit infers a zero safe area.
    // Keep the toolbar below the status region throughout that transition.
    // Read the unmodified host inset, never the inset we are adjusting.
    // During a slide UIKit may not yet have applied the previous adjustment.
    let automaticTop = container.safeAreaInsets.top
    // An empty native bar supplies the same edge geometry beneath a custom
    // Flutter header. Place its lower edge at the end of that header so the
    // system fade protects the filters as well as the status area. Duo may
    // collapse the native row to zero; do not subtract a phone-sized row.
    let protectedTop = backdropOnly
      ? max(flutterSafeTop, container.bounds.height - navigation.navigationBar.bounds.height)
      : flutterSafeTop
    // Duo can collapse the native horizontal control row to zero because its
    // controls live in the side rail. Flutter still reserves a 44-point row
    // plus 24 points of breathing room. Give expanded titles that same space.
    let compact = (metrics?["compactHeight"] as? CGFloat) ?? 44
    let paneTitleClearance = alignCollapsedTitleToLeading
      ? max(44, compact) + 24 - compact : 0
    let requiredTop = max(0, protectedTop + paneTitleClearance - automaticTop)
    if abs(navigation.additionalSafeAreaInsets.top - requiredTop) > 0.5 {
      navigation.additionalSafeAreaInsets.top = requiredTop
    }
    navigation.view.setNeedsLayout()
    navigation.view.layoutIfNeeded()
    measureIfNeeded()
    applyScroll()
    // Glass-backed conversations need only a short status-area fade.
    // Recompute the material bounds after rotation and inset changes.
    let statusHeight = navigation.view.safeAreaInsets.top
    // Before iOS 26 the actions have no glass, and subtitle-less thread
    // titles remain plain UIKit labels even on iOS 26. They need material
    // beneath the entire control area, not just the status indicators.
    let glassBackedControls: Bool
    if #available(iOS 26.0, *) {
      glassBackedControls = content.navigationItem.titleView is NavigationTitleView || paneTitle is NavigationTitleView || floatingTitle != nil
    } else {
      glassBackedControls = false
    }
    let statusOnly = alwaysFrosted && !centerTitleInPane && paneTitle == nil && !horizontalControls
      && !navigation.navigationBar.prefersLargeTitles && glassBackedControls
    let height = statusOnly
      ? min(container.bounds.height, statusHeight > 0 ? statusHeight + 6 : 0)
      : container.bounds.height
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    material.frame = CGRect(x: 0, y: 0, width: container.bounds.width, height: height)
    materialFade.frame = material.bounds
    materialFade.locations = statusOnly
      ? [0, NSNumber(value: Double(max(0, height - 12) / max(1, height))), 1]
      : [0, 0.55, 1]
    CATransaction.commit()
    if let title = paneTitle {
      let titleWidth = max(0, container.bounds.width - titleInsetLeft - titleInsetRight)
      let center = titleInsetLeft + titleWidth / 2
      var availableWidth = max(0, titleWidth - 32)
      if horizontalControls && !alignCollapsedTitleToLeading {
        let leadingWidth = headerLeading?.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize).width ?? 0
        let trailingWidth = headerTrailing?.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize).width ?? 0
        let leftEdge = titleInsetLeft + 16 + leadingWidth
        let rightEdge = container.bounds.width - titleInsetRight - 20 - trailingWidth
        availableWidth = max(0, min(center - leftEdge, rightEdge - center) * 2 - 16)
      }
      let size = title.sizeThatFits(CGSize(width: availableWidth, height: 44))
      title.frame = CGRect(x: alignCollapsedTitleToLeading
                           ? navigation.navigationBar.layoutMargins.left
                           : titleInsetLeft + (titleWidth - size.width) / 2,
                           y: alignCollapsedTitleToLeading
                             ? flutterSafeTop + 24 + (44 - size.height) / 2
                             : (container.bounds.height - size.height) / 2,
                           width: size.width, height: size.height)
      title.layoutIfNeeded()
    }
    if horizontalControls {
      // These native glass groups belong to the Flutter pane, whose bounds
      // already exclude the rail. Avoid UINavigationBar reserving it again.
      for (surface, trailing) in [(headerLeading, false), (headerTrailing, true)] {
        guard let surface else { continue }
        let size = surface.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize)
        surface.frame = CGRect(x: trailing ? container.bounds.width - titleInsetRight - 20 - size.width : titleInsetLeft + 16,
                               y: navigation.navigationBar.prefersLargeTitles
                                 ? max(12, flutterSafeTop + 24 + (44 - size.height) / 2)
                                 : max(12, (container.bounds.height - size.height) / 2),
                               width: size.width, height: size.height)
        surface.layoutIfNeeded()
      }
    }
    if let title = floatingTitle {
      let available = max(0, container.bounds.width - 48)
      let width = min(title.intrinsicContentSize.width, min(available, titleMaxWidth ?? available))
      title.frame = CGRect(x: (container.bounds.width - width) / 2,
                           y: flutterSafeTop + portraitTopPadding + 56 - floatingTitleLift, width: width, height: 44)
      // This overlay is outside UINavigationBar, which normally lays out the
      // effect content after constraining its title. Apply that pass here too.
      title.layoutIfNeeded()

    }
    reportAvatarBounds()
  }

  private func reportRailGeometry() {
    #if canImport(UIKit, _version: 9127.0.85)
    guard #available(iOS 27.1, *), let window = container.window else { return }
    let inset = window.safeAreaInsets
    let right = inset.right > 0 && inset.left == 0
    let left = inset.left > 0 && inset.right == 0
    guard inset.top == 0, right || left else { return }
    let strip = CGRect(x: right ? window.bounds.width - inset.right : 0,
                       y: 0, width: right ? inset.right : inset.left,
                       height: window.bounds.height)
    // Reserved-region frames include interactive margins. Remove those
    // before finding the hardware axis; the safe-area strip is asymmetric.
    let regions = window.reservedRegions(kind: .occlusion)
      .filter { $0.isActive && $0.frame.intersects(strip) && $0.frame.midY < window.bounds.midY }
      .sorted { $0.frame.minY < $1.frame.minY }
    // Some hosts don't expose occlusions. The full safe-area inset includes
    // content clearance; use UIKit's outer layout margin for the control,
    // rather than centering it in that asymmetric inset.
    guard let root = window.rootViewController else { return }
    // Read the unobstructed edge: the strip-side margin has already been
    // expanded to the safe area, while Flutter's controller reports a zero
    // systemMinimumLayoutMargins despite its view's actual UIKit margins.
    let margins = root.view.layoutMargins
    let outerMargin = right ? margins.left : margins.right
    let marginCenter = right ? window.bounds.width - outerMargin - 28 : outerMargin + 28
    let center = regions.first.map { $0.frame.inset(by: $0.margins).midX } ?? marginCenter
    guard strip.contains(CGPoint(x: center, y: strip.midY)) else { return }
    let division = window.reservedRegions(kind: .division)
      .first { $0.isActive && $0.frame.height >= window.bounds.height - 1
        && $0.frame.minX > 0 && $0.frame.maxX < window.bounds.width }
    let fold: Any = division.map { region -> Any in
      [region.frame.minX, region.frame.minY, region.frame.width, region.frame.height]
    } ?? NSNull()
    let values: [String: Any] = ["bookDivision": fold, "centerX": center,
                               "topClearance": regions.map { $0.frame.maxY }.max() as Any? ?? NSNull(),
                               "windowWidth": window.bounds.width,
                               "windowHeight": window.bounds.height]
    DispatchQueue.main.async { [weak self] in
      self?.channel.invokeMethod("railGeometry", arguments: values)
    }
    #endif
  }

  private func reportAvatarBounds() {
    guard let avatar = trackedAvatar, avatar.window != nil, avatar.bounds.width > 0 else { return }
    avatar.layoutIfNeeded()
    guard let image = avatar.imageView, image.bounds.width > 0 else { return }
    let frame = image.convert(image.bounds, to: container)
    guard frame != reportedAvatarFrame else { return }
    reportedAvatarFrame = frame
    DispatchQueue.main.async { [weak self] in
      self?.channel.invokeMethod("avatarBounds", arguments: [
        "id": "leading", "x": frame.minX, "y": frame.minY,
        "width": frame.width, "height": frame.height
      ])
    }
  }

  private func configure(_ args: [String: Any]) {
    titleInsetLeft = max(0, (args["titleInsetLeft"] as? NSNumber)?.doubleValue ?? 0)
    titleInsetRight = max(0, (args["titleInsetRight"] as? NSNumber)?.doubleValue ?? 0)
    controlBackground = Self.color(args["controlBackground"])
    flutterSafeTop = (args["safeTop"] as? NSNumber)?.doubleValue ?? 0
    floatingTitleLift = args["titleHasAnimatedPortrait"] as? Bool == true ? 14 : 0
    titleMaxWidth = (args["titleMaxWidth"] as? NSNumber).map { max(0, CGFloat($0.doubleValue)) }
    navigation.overrideUserInterfaceStyle = args["dark"] as? Bool == true ? .dark : .light
    container.overrideUserInterfaceStyle = navigation.overrideUserInterfaceStyle
    material.overrideUserInterfaceStyle = navigation.overrideUserInterfaceStyle
    // Let the ultra-thin system material provide its own adaptive tint.
    material.contentView.backgroundColor = .clear
    standaloneControl = args["standaloneControl"] as? Bool == true
    standaloneLargeTitle = args["standaloneLargeTitle"] as? Bool == true
    standaloneSurface?.removeFromSuperview()
    standaloneSurface = nil
    horizontalControls = args["horizontalControls"] as? Bool == true
    centerTitleInPane = args["centerTitleInPane"] as? Bool == true
    portraitTopPadding = CGFloat((args["portraitTopPadding"] as? NSNumber)?.doubleValue ?? 0)
    paneTitle?.removeFromSuperview()
    paneTitle = nil
    alignCollapsedTitleToLeading = args["alignCollapsedTitleToLeading"] as? Bool == true
    headerLeading?.removeFromSuperview()
    headerTrailing?.removeFromSuperview()
    headerLeading = nil
    headerTrailing = nil
    navigation.view.isHidden = standaloneControl || standaloneLargeTitle
      || (horizontalControls && args["largeTitle"] as? Bool != true)
    if standaloneLargeTitle {
      container.overrideUserInterfaceStyle = navigation.overrideUserInterfaceStyle
      material.isHidden = true
      container.clipsToBounds = true
      let scroll = UIScrollView()
      scroll.contentInsetAdjustmentBehavior = .never
      let label = UILabel()
      let font = UIFont.preferredFont(forTextStyle: .largeTitle)
      label.font = UIFont(descriptor: font.fontDescriptor.withSymbolicTraits(.traitBold) ?? font.fontDescriptor, size: 0)
      label.adjustsFontForContentSizeCategory = true
      label.text = args["title"] as? String
      label.textColor = Self.color(args["foreground"])
      label.numberOfLines = 0
      label.lineBreakMode = .byWordWrapping
      label.accessibilityTraits = .header
      scroll.addSubview(label)
      standaloneSurface = scroll
      container.addSubview(scroll)
      container.setNeedsLayout()
      return
    }
    if standaloneControl {
      material.isHidden = true
      container.clipsToBounds = false
      container.overrideUserInterfaceStyle = navigation.overrideUserInterfaceStyle
      navigation.navigationBar.tintColor = Self.color(args["foreground"])
      trackedAvatar = nil
      reportedAvatarFrame = nil
      if let leading = args["leading"] as? [String: Any],
         let surface = makeControlGroup([leading]).customView {
        standaloneSurface = surface
        container.addSubview(surface)
      }
      container.setNeedsLayout()
      return
    }
    let bar = navigation.navigationBar
    let color = Self.color(args["foreground"])
    bar.tintColor = color
    titleColor = color
    largeTitleOpacity = -1
    backdropOnly = args["backdropOnly"] as? Bool == true
    container.isUserInteractionEnabled = !backdropOnly
    navigation.view.isUserInteractionEnabled = !backdropOnly
    navigation.view.accessibilityElementsHidden = backdropOnly
    let largeTitle = args["largeTitle"] as? Bool == true
    alwaysFrosted = args["alwaysFrosted"] as? Bool == true
    if #available(iOS 26.0, *) {
      if #available(iOS 27.0, *) {
        // All headers use the same native soft scroll edge. Custom titles
        // also retain material, like large titles, when embedding clips it.
        usesSystemScrollEdge = true
      }
      content.scrollView.topEdgeEffect.style = .soft
      content.scrollView.topEdgeEffect.isHidden = !usesSystemScrollEdge
    }
    // Older embedded views retain material because their native edge cannot
    // sample Flutter content. On iOS 27, UIKit also suppresses its native
    // edge while the large-title inset is collapsing.
    // Keep the graduated material throughout that transition so partial
    // scrolling never leaves the title and status area unprotected.
    material.isHidden = usesSystemScrollEdge && !largeTitle
      && args["centerTitleInPane"] as? Bool != true && !backdropOnly
    // Let the native fade extend past the bar without enlarging its hit area.
    // The mirrored content is transparent and the bottom edge is hidden.
    container.clipsToBounds = !usesSystemScrollEdge
    let appearance = UINavigationBarAppearance()
    if usesSystemScrollEdge {
      appearance.configureWithDefaultBackground()
    } else {
      appearance.configureWithTransparentBackground()
    }
    appearance.titleTextAttributes = [.foregroundColor: color]
    appearance.largeTitleTextAttributes = [.foregroundColor: color]
    bar.standardAppearance = appearance
    bar.scrollEdgeAppearance = appearance
    bar.compactAppearance = appearance
    if bar.prefersLargeTitles != largeTitle {
      measuredWidth = 0
      titleIsCollapsed = nil
    }
    bar.prefersLargeTitles = largeTitle
    // Conversations request a stable backdrop from the first frame, including
    // loading/empty timelines. Other pages retain their scroll-edge treatment.
    material.alpha = alwaysFrosted ? 1 : min(1, offset / 12)
    floatingTitle?.removeFromSuperview()
    floatingTitle = nil
    let item = content.navigationItem
    item.title = args["title"] as? String
    if let subtitle = args["subtitle"] as? String {
      let button = NavigationTitleView(title: item.title, subtitle: subtitle, color: color, maximumWidth: titleMaxWidth)
      button.accessibilityIdentifier = "channel-navigation-title"
      let enabled = args["titleEnabled"] as? Bool == true
      let accessibleTitle = [item.title ?? "", subtitle].filter { !$0.isEmpty }.joined(separator: ", ")
      button.accessibilityLabel = enabled ? "Open settings for \(accessibleTitle)" : accessibleTitle
      button.accessibilityTraits = enabled ? .button : .header
      button.isUserInteractionEnabled = enabled
      if enabled {
        button.onActivate = { [weak self] in self?.channel.invokeMethod("action", arguments: "title") }
      }
      if let avatar = args["titleAvatar"] as? [String: Any] {
        button.setAvatar(makeItem(avatar).image,
                         presence: args["titlePresenceColor"] is NSNumber ? Self.color(args["titlePresenceColor"]) : nil)
      }
      if args["titleAvatar"] as? [String: Any] == nil, args["titlePresenceColor"] is NSNumber {
        button.setSubtitlePresence(Self.color(args["titlePresenceColor"]))
      }
      if let label = args["ephemeralLabel"] as? String {
        button.setEphemeralStatus(label)
      }
      refreshBar.color = args["titleRefreshColor"] is NSNumber ? Self.color(args["titleRefreshColor"]) : color
      button.attachRefreshBar(refreshBar)
      button.frame.size = button.intrinsicContentSize
      let portraits = args["titleAvatars"] as? [[String: Any]] ?? []
      if portraits.isEmpty {
        item.titleView = button
      } else {
        item.title = nil
        item.titleView = nil
        // Floating titles use explicit frames rather than UINavigationBar constraints.
        button.translatesAutoresizingMaskIntoConstraints = true
        floatingTitle = button
        container.addSubview(button)

      }
      refreshBar.setRefreshing(args["titleRefreshing"] as? Bool == true)
    } else if largeTitle {
      compactTitle.text = item.title
      compactTitle.font = .preferredFont(forTextStyle: .headline)
      compactTitle.adjustsFontForContentSizeCategory = true
      compactTitle.textColor = color
      compactTitle.accessibilityTraits = .header
      compactTitle.sizeToFit()
      compactTitleMask.backgroundColor = UIColor.black.cgColor
      compactTitle.layer.mask = compactTitleMask
      if compactTitle.superview == nil { compactTitleContainer.addArrangedSubview(compactTitle) }
      compactTitleContainer.frame.size = compactTitle.intrinsicContentSize
      if alignCollapsedTitleToLeading {
        // Duo's list title fades in place: the compact label uses the same
        // leading margin as UIKit's expanded title, rather than recentering.
        item.titleView = UIView()
        compactTitleContainer.translatesAutoresizingMaskIntoConstraints = true
        paneTitle = compactTitleContainer
        container.addSubview(compactTitleContainer)
      } else {
        item.titleView = compactTitleContainer
      }
    } else {
      item.titleView = nil
    }
    if args["centerTitleInPane"] as? Bool == true, !largeTitle {
      if let titleView = item.titleView {
        item.titleView = nil
        titleView.translatesAutoresizingMaskIntoConstraints = true
        paneTitle = titleView
      } else if let text = item.title, !text.isEmpty {
        let label = UILabel()
        label.text = text
        label.font = .preferredFont(forTextStyle: .headline)
        label.adjustsFontForContentSizeCategory = true
        label.textColor = color
        label.textAlignment = .center
        label.lineBreakMode = .byTruncatingTail
        label.accessibilityTraits = .header
        paneTitle = label
      }
      item.title = nil
      if let paneTitle { container.addSubview(paneTitle) }
    }
    item.largeTitleDisplayMode = bar.prefersLargeTitles ? .always : .never
    trackedAvatar = nil
    reportedAvatarFrame = nil
    if let leading = args["leading"] as? [String: Any] {
      item.leftBarButtonItems = [makeControlGroup([leading])]
    } else if args["back"] as? Bool == true {
      item.leftBarButtonItems = [makeControlGroup([["id": "back", "label": "Back", "symbol": "chevron.backward", "enabled": true]])]
    } else {
      item.leftBarButtonItems = nil
    }
    var groups: [[[String: Any]]] = []
    for action in args["actions"] as? [[String: Any]] ?? [] {
      let shares = action["sharesBackground"] as? Bool ?? true
      if shares, let previous = groups.last?.last,
         previous["sharesBackground"] as? Bool ?? true,
         previous["plain"] as? Bool != true, action["plain"] as? Bool != true {
        groups[groups.count - 1].append(action)
      } else {
        groups.append([action])
      }
    }
    if horizontalControls {
      item.leftBarButtonItems = nil
      item.rightBarButtonItems = nil
      if let leading = args["leading"] as? [String: Any] {
        headerLeading = makeControlGroup([leading]).customView
      }
      if let actions = args["actions"] as? [[String: Any]], !actions.isEmpty {
        headerTrailing = makeControlGroup(actions).customView
      }
      for surface in [headerLeading, headerTrailing].compactMap({ $0 }) {
        container.addSubview(surface)
      }
    } else {
      item.rightBarButtonItems = groups.reversed().map(makeControlGroup)
    }
    if let metrics {
      channel.invokeMethod("metrics", arguments: metrics)
    }
    container.setNeedsLayout()
  }

  private func measureIfNeeded() {
    guard container.window != nil, container.bounds.width > 0 else { return }
    let category = navigation.traitCollection.preferredContentSizeCategory
    let safeTop = navigation.view.safeAreaInsets.top
    guard measuredWidth != container.bounds.width || measuredSafeTop != safeTop || measuredCategory != category else { return }
    measuring = true
    titleIsCollapsed = nil
    defer { measuring = false }
    let bar = navigation.navigationBar
    let large = bar.prefersLargeTitles
    // Measure UIKit's actual compact and expanded layouts before presenting
    // this frame. Flutter reserves these measured dimensions, never the other
    // way around. Recalculate after rotation or a Dynamic Type change.
    bar.prefersLargeTitles = false
    content.navigationItem.largeTitleDisplayMode = .never
    navigation.view.setNeedsLayout()
    navigation.view.layoutIfNeeded()
    let compact = bar.frame.height
    bar.prefersLargeTitles = large
    content.navigationItem.largeTitleDisplayMode = large ? .always : .never
    content.scrollView.setContentOffset(CGPoint(x: 0, y: -1000), animated: false)
    navigation.view.setNeedsLayout()
    navigation.view.layoutIfNeeded()
    expandedBarHeight = bar.frame.height
    measuredWidth = container.bounds.width
    measuredSafeTop = safeTop
    measuredCategory = category
    var metrics: [String: Any] = ["compactHeight": compact]
    if large { metrics["expandedHeight"] = expandedBarHeight }
    self.metrics = metrics
    // The first measurement may reveal a zero-height Duo control row.
    // Reconcile its reserved title space on the next native layout pass.
    if alignCollapsedTitleToLeading { container.setNeedsLayout() }
    DispatchQueue.main.async { [weak self] in
      self?.channel.invokeMethod("metrics", arguments: metrics)
    }
  }

  private func setScrollOffset(_ value: CGFloat) {
    guard !standaloneControl && !standaloneLargeTitle else { return }
    guard value.isFinite else { return }
    // The mirror scroll view is finite. Past its end UIKit sees no content
    // under the bar and drops the scroll-edge blur, so a long chat whose
    // estimated history depth crosses that end flickers between blurred and
    // sharp. Only the first few hundred points affect the bar's appearance.
    offset = min(max(0, value), Self.maxTrackedDepth)
    material.alpha = alwaysFrosted ? 1 : min(1, offset / 12)
    applyScroll()
    reportAvatarBounds()
  }

  private func applyScroll() {
    let scroll = content.scrollView
    guard container.window != nil else { return }
    // UIKit reduces adjustedContentInset as the title collapses. Using that
    // moving inset as zero leaves the title collapsed when Flutter returns to
    // the top. Keep zero anchored to the expanded bar, including the current
    // safe area, throughout the scroll cycle.
    expandedBarHeight = max(expandedBarHeight, navigation.navigationBar.frame.height)
    let topInset = navigation.view.safeAreaInsets.top + expandedBarHeight
    // Compact conversations always have their content behind the overlay.
    // Zero represents the viewport's top, not an empty native inset; this
    // makes the system edge ready before Flutter's first scroll notification.
    let contentBehindBar = usesSystemScrollEdge && alwaysFrosted && !navigation.navigationBar.prefersLargeTitles
    let desired = CGPoint(x: 0, y: contentBehindBar ? offset : -topInset + offset)
    let compactHeight = (metrics?["compactHeight"] as? CGFloat) ?? navigation.navigationBar.frame.height
    let collapseRange = max(1, expandedBarHeight - compactHeight)
    let collapsed = offset >= collapseRange
    let wasCollapsed = titleIsCollapsed
    titleIsCollapsed = collapsed
    UIView.performWithoutAnimation {
      if navigation.navigationBar.prefersLargeTitles {
        // Fade the actual text throughout the last part of the gesture. A
        // snapshot transition of the platform view gets clipped by Flutter's
        // shrinking header and can still look like a one-frame title switch.
        let progress = min(1, max(0, (offset - collapseRange * 0.35) / (collapseRange * 0.65)))
        let opacity = 1 - progress
        if abs(largeTitleOpacity - opacity) > 0.001 {
          largeTitleOpacity = opacity
          for appearance in [navigation.navigationBar.standardAppearance,
                             navigation.navigationBar.scrollEdgeAppearance,
                             navigation.navigationBar.compactAppearance].compactMap({ $0 }) {
            appearance.largeTitleTextAttributes[.foregroundColor] = titleColor.withAlphaComponent(opacity)
          }
          navigation.navigationBar.largeTitleTextAttributes = [.foregroundColor: titleColor.withAlphaComponent(opacity)]
        }
      }
      scroll.setContentOffset(desired, animated: false)
      navigation.view.layoutIfNeeded()
      // Expanding changes UIKit's inset and can compensate the offset.
      if abs(scroll.contentOffset.y - desired.y) > 0.1 {
        scroll.setContentOffset(desired, animated: false)
        navigation.view.layoutIfNeeded()
      }
    }
    if navigation.navigationBar.prefersLargeTitles {
      // UIKit owns title-view alpha on some iOS versions. A mask keeps the
      // text fade independent from its layout-driven visibility changes.
      CATransaction.begin()
      CATransaction.setDisableActions(true)
      compactTitleMask.frame = compactTitle.bounds
      compactTitleMask.opacity = collapsed ? 1 : 0
      CATransaction.commit()
      if wasCollapsed != collapsed {
        compactTitleMask.removeAllAnimations()
        if collapsed && wasCollapsed != nil && !UIAccessibility.isReduceMotionEnabled {
          let fade = CABasicAnimation(keyPath: "opacity")
          fade.fromValue = 0
          fade.toValue = 1
          fade.duration = 0.18
          fade.timingFunction = CAMediaTimingFunction(name: .easeOut)
          compactTitleMask.add(fade, forKey: "titleFade")
        }
      }
    }
  }

  private func makeAction(_ data: [String: Any]) -> UIAction {
    let id = data["id"] as? String ?? ""
    let enabled = data["enabled"] as? Bool == true
    let symbol = (data["symbol"] as? String).flatMap { NativeTabler.image($0, size: 24) }
    return UIAction(title: data["label"] as? String ?? "", image: symbol,
                    attributes: enabled ? [] : [.disabled],
                    state: data["selected"] as? Bool == true ? .on : .off) { [weak self] _ in
      self?.channel.invokeMethod("action", arguments: id)
    }
  }

  private func makeControlGroup(_ actions: [[String: Any]]) -> UIBarButtonItem {
    if actions.count == 1 && !standaloneControl {
      let item = makeItem(actions[0])
      // Let UIKit retain its navigation typography and sizing for text actions
      // such as Edit/Done, rather than inheriting UIButton's default styling.
      if actions[0]["plain"] as? Bool == true || (item.image == nil && item.customView == nil) {
        return item
      }
    }
    let surface = UIVisualEffectView()
    if #available(iOS 26.0, *) {
      let glass = UIGlassEffect(style: .regular)
      glass.isInteractive = true
      surface.effect = glass
      surface.cornerConfiguration = .capsule()
    } else {
      surface.backgroundColor = controlBackground
    }
    // Use compact controls while retaining the 44-point minimum hit target.
    let controlSize: CGFloat = standaloneControl ? 48 : 44
    surface.layer.cornerRadius = controlSize / 2
    surface.layer.cornerCurve = .circular
    surface.clipsToBounds = true
    let stack = UIStackView()
    stack.axis = .horizontal
    stack.translatesAutoresizingMaskIntoConstraints = false
    surface.contentView.addSubview(stack)
    for data in actions {
      let item = makeItem(data)
      let button: UIButton
      if let avatar = item.customView as? UIButton {
        button = avatar
      } else {
        button = UIButton(type: .system)
        button.setImage(item.image, for: .normal)
        if item.image == nil { button.setTitle(item.title, for: .normal) }
        if let action = item.primaryAction { button.addAction(action, for: .touchUpInside) }
        button.menu = item.menu
        button.showsMenuAsPrimaryAction = item.menu != nil
        if item.menu != nil {
          // UIKit lifts the button itself for its menu transition, not the
          // enclosing glass. Give that return target the same rounded shape.
          if #available(iOS 26.0, *) {
            button.cornerConfiguration = .capsule()
          }
          button.layer.cornerRadius = controlSize / 2
          button.layer.cornerCurve = .circular
          button.clipsToBounds = true
        }
        button.isEnabled = item.isEnabled
        button.accessibilityLabel = item.accessibilityLabel
        button.accessibilityValue = item.accessibilityValue
      }
      button.tintColor = navigation.navigationBar.tintColor
      let slot = UIView()
      slot.addSubview(button)
      button.translatesAutoresizingMaskIntoConstraints = false
      NSLayoutConstraint.activate([
        slot.widthAnchor.constraint(equalToConstant: item.image == nil && item.customView == nil ? max(controlSize, button.intrinsicContentSize.width + 16) : controlSize),
        slot.heightAnchor.constraint(equalToConstant: controlSize),
        button.centerXAnchor.constraint(equalTo: slot.centerXAnchor),
        button.centerYAnchor.constraint(equalTo: slot.centerYAnchor)
      ])
      if item.customView == nil {
        NSLayoutConstraint.activate([
          button.widthAnchor.constraint(equalTo: slot.widthAnchor),
          button.heightAnchor.constraint(equalTo: slot.heightAnchor)
        ])
      }
      stack.addArrangedSubview(slot)
    }
    NSLayoutConstraint.activate([
      stack.leadingAnchor.constraint(equalTo: surface.contentView.leadingAnchor),
      stack.trailingAnchor.constraint(equalTo: surface.contentView.trailingAnchor),
      stack.topAnchor.constraint(equalTo: surface.contentView.topAnchor),
      stack.bottomAnchor.constraint(equalTo: surface.contentView.bottomAnchor)
    ])
    let item = UIBarButtonItem(customView: surface)
    #if canImport(UIKit, _version: 9127.0.85)
    if #available(iOS 27.1, *), actions.allSatisfy({ $0["keepInHeaderOnDuo"] as? Bool == true }) {
      item.axisBehavior = .horizontalOnly
    }
    #endif
    if #available(iOS 26.0, *) { item.hidesSharedBackground = true }
    return item
  }

  private func makeItem(_ data: [String: Any]) -> UIBarButtonItem {
    let action = makeAction(data)
    let children = data["children"] as? [[String: Any]] ?? []
    let item = children.isEmpty
      ? UIBarButtonItem(primaryAction: action)
      : UIBarButtonItem(title: action.title, image: action.image, primaryAction: nil,
                        menu: UIMenu(children: children.map(makeAction)))
    if #available(iOS 26.0, *) {
      item.hidesSharedBackground = data["plain"] as? Bool == true
      item.sharesBackground = data["sharesBackground"] as? Bool ?? true
    }
    item.accessibilityLabel = data["label"] as? String
    item.isEnabled = data["enabled"] as? Bool == true
    if data["avatarInitial"] is String { item.title = nil }
    if let encoded = data["iconData"] as? String,
       let bytes = Data(base64Encoded: encoded), let image = UIImage(data: bytes) {
      item.image = UIGraphicsImageRenderer(size: CGSize(width: 24, height: 24)).image { _ in
        image.draw(in: CGRect(x: 0, y: 0, width: 24, height: 24))
      }.withRenderingMode(.alwaysTemplate)
      item.title = nil
    }

    if let encoded = data["imageData"] as? String,
       let bytes = Data(base64Encoded: encoded), let image = UIImage(data: bytes) {
      // Character artwork floats on the glass; photos fill its circular face.
      // Preserve the 24-point alignment footprint so UIKit does not widen it.
      let diameter: CGFloat = data["avatarIsAgent"] as? Bool == true && data["avatarArtwork"] as? Bool != true ? 44 : 36
      let inset = (diameter - 24) / 2
      let size = CGSize(width: diameter, height: diameter)
      item.image = UIGraphicsImageRenderer(size: size).image { _ in
        image.draw(in: CGRect(origin: .zero, size: size))
      }.withRenderingMode(.alwaysOriginal).withAlignmentRectInsets(
        UIEdgeInsets(top: inset, left: inset, bottom: inset, right: inset)
      )
    }
    if item.image == nil, let initial = data["avatarInitial"] as? String {
      // An avatar-shaped placeholder is available synchronously, before
      // Flutter finishes decoding the photo. Never fall back to a glyph icon.
      let size = CGSize(width: 36, height: 36)
      item.title = nil
      item.image = UIGraphicsImageRenderer(size: size).image { _ in
        Self.color(data["avatarBackground"]).setFill()
        UIBezierPath(roundedRect: CGRect(origin: .zero, size: size),
                     cornerRadius: size.width * 0.5).fill()
        let text = initial as NSString
        let attributes: [NSAttributedString.Key: Any] = [
          .font: UIFont.systemFont(ofSize: 16, weight: .medium),
          .foregroundColor: Self.color(data["avatarForeground"])
        ]
        let textSize = text.size(withAttributes: attributes)
        text.draw(at: CGPoint(x: (size.width - textSize.width) / 2,
                              y: (size.height - textSize.height) / 2), withAttributes: attributes)
      }.withRenderingMode(.alwaysOriginal).withAlignmentRectInsets(
        UIEdgeInsets(top: 6, left: 6, bottom: 6, right: 6)
      )
    }
    if data["activityColor"] is NSNumber, let symbol = item.image {
      let size = CGSize(width: 28, height: 26)
      item.image = UIGraphicsImageRenderer(size: size).image { _ in
        symbol.withTintColor(navigation.navigationBar.tintColor).draw(in: CGRect(x: 0, y: 4, width: 22, height: 22))
        UIColor.systemBackground.setFill()
        UIBezierPath(ovalIn: CGRect(x: 18, y: 0, width: 10, height: 10)).fill()
        Self.color(data["activityColor"]).setFill()
        UIBezierPath(ovalIn: CGRect(x: 19.5, y: 1.5, width: 7, height: 7)).fill()
      }.withRenderingMode(.alwaysOriginal)
      item.accessibilityValue = data["activityLabel"] as? String
    }
    if data["avatarInitial"] is String, children.isEmpty {
      // An explicit native button gives Flutter a public, measured destination
      // without depending on UINavigationBar's private view hierarchy.
      let button = UIButton(type: .custom)
      // UIKit adds the glass button's own padding around this custom view.
      // Match the 36-point image on both axes so that glass stays circular.
      let avatarTarget: CGFloat = standaloneControl ? 48 : 36
      button.frame = CGRect(x: 0, y: 0, width: avatarTarget, height: avatarTarget)
      button.widthAnchor.constraint(equalToConstant: avatarTarget).isActive = true
      button.heightAnchor.constraint(equalToConstant: avatarTarget).isActive = true
      button.setImage(item.image?.withAlignmentRectInsets(.zero), for: .normal)
      button.addAction(action, for: .touchUpInside)
      button.isEnabled = item.isEnabled
      button.accessibilityLabel = item.accessibilityLabel
      button.accessibilityIdentifier = data["id"] as? String == "leading"
        ? "community-navigation-avatar" : "profile-navigation-avatar"
      let hidden = data["avatarHidden"] as? Bool == true
      button.alpha = hidden ? 0 : 1
      button.accessibilityElementsHidden = hidden
      if data["tracksAvatarBounds"] as? Bool == true, data["id"] as? String == "leading" {
        trackedAvatar = button
      }
      // Both edge avatars use the same footprint and native glass padding,
      // keeping their containers aligned to UIKit's symmetric navigation gutter.
      let avatarItem = UIBarButtonItem(customView: button)
      if #available(iOS 26.0, *) {
        avatarItem.hidesSharedBackground = data["plain"] as? Bool == true
        avatarItem.sharesBackground = data["sharesBackground"] as? Bool ?? true
      }
      return avatarItem
    }
    return item
  }

  private static func color(_ value: Any?) -> UIColor {
    guard let argb = (value as? NSNumber)?.uint32Value else { return .label }
    return UIColor(red: CGFloat((argb >> 16) & 255) / 255,
                   green: CGFloat((argb >> 8) & 255) / 255,
                   blue: CGFloat(argb & 255) / 255, alpha: CGFloat(argb >> 24) / 255)
  }

  deinit {
    foldRefresh?.cancel()
    channel.setMethodCallHandler(nil)
    navigation.willMove(toParent: nil)
    navigation.view.removeFromSuperview()
    navigation.removeFromParent()
  }
}
