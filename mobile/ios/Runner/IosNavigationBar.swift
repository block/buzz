import Flutter
import UIKit

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

final class NavigationTitleView: UIVisualEffectView, UIGestureRecognizerDelegate {
  var maximumWidth: CGFloat = 240 {
    didSet { if maximumWidth != oldValue { invalidateIntrinsicContentSize() } }
  }
  var onActivate: (() -> Void)?
  private let titleLabel = UILabel()
  private let subtitleLabel = UILabel()
  private var avatarView: UIImageView?
  private var presenceView: UIView?
  private var subtitlePresenceView: UIView?

  init(title: String?, subtitle: String, color: UIColor) {
    if #available(iOS 26.0, *) {
      super.init(effect: UIGlassEffect(style: .regular))
    } else {
      super.init(effect: UIBlurEffect(style: .systemMaterial))
    }
    clipsToBounds = true
    layer.cornerCurve = .continuous
    titleLabel.text = title
    titleLabel.font = .preferredFont(forTextStyle: .headline)
    titleLabel.textColor = color
    subtitleLabel.text = subtitle
    subtitleLabel.font = .preferredFont(forTextStyle: .caption1)
    subtitleLabel.textColor = .secondaryLabel
    for label in [titleLabel, subtitleLabel] {
      label.textAlignment = .center
      label.lineBreakMode = .byTruncatingTail
      label.adjustsFontForContentSizeCategory = false
      contentView.addSubview(label)
    }
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
    let avatar = UIImageView(image: image)
    avatar.accessibilityIdentifier = "dm-navigation-avatar"
    avatar.contentMode = .scaleAspectFit
    contentView.addSubview(avatar)
    avatarView = avatar
    for label in [titleLabel, subtitleLabel] { label.textAlignment = .natural }
    if let presence {
      let badge = UIView()
      badge.backgroundColor = presence
      badge.layer.cornerRadius = 4
      badge.layer.borderWidth = 1.5
      badge.layer.borderColor = UIColor.systemBackground.cgColor
      badge.accessibilityIdentifier = "dm-navigation-presence"
      contentView.addSubview(badge)
      presenceView = badge
    }
  }

  func setSubtitlePresence(_ color: UIColor) {
    let dot = UIView()
    dot.backgroundColor = color
    dot.layer.cornerRadius = 3
    dot.isAccessibilityElement = false
    dot.accessibilityIdentifier = "dm-navigation-status-dot"
    contentView.addSubview(dot)
    subtitlePresenceView = dot
    invalidateIntrinsicContentSize()
  }

  func setEphemeralStatus(_ label: String) {
    // The subtitle carries the visible temporary/expiry text. Keep the full
    // retention explanation on the title's single VoiceOver element too.
    accessibilityLabel = [accessibilityLabel, label].compactMap { $0 }.joined(separator: ", ")
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
    CGSize(width: min(maximumWidth, max(titleLabel.intrinsicContentSize.width, subtitleLabel.intrinsicContentSize.width + (subtitlePresenceView == nil ? 0 : 12)) + 24 + (avatarView == nil ? 0 : 40)),
           height: max(44, titleLabel.intrinsicContentSize.height + subtitleLabel.intrinsicContentSize.height))
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    layer.cornerRadius = bounds.height / 2
    // Compact navigation remains a 44pt toolbar. Scale within that budget;
    // the complete title/subtitle remains available as one VoiceOver label.
    titleLabel.font = UIFontMetrics(forTextStyle: .headline).scaledFont(
      for: .systemFont(ofSize: 17, weight: .semibold), maximumPointSize: 20,
      compatibleWith: traitCollection)
    subtitleLabel.font = UIFontMetrics(forTextStyle: .caption1).scaledFont(
      for: .systemFont(ofSize: 12), maximumPointSize: 14,
      compatibleWith: traitCollection)
    let titleHeight = titleLabel.intrinsicContentSize.height
    let subtitleHeight = subtitleLabel.intrinsicContentSize.height
    let top = (bounds.height - titleHeight - subtitleHeight) / 2
    let hasAvatar = avatarView != nil
    let rtl = effectiveUserInterfaceLayoutDirection == .rightToLeft
    let textX: CGFloat = hasAvatar && !rtl ? 52 : 12
    let textWidth = max(0, bounds.width - 24 - (hasAvatar ? 40 : 0))
    titleLabel.frame = CGRect(x: textX, y: top, width: textWidth, height: titleHeight)
    subtitleLabel.frame = CGRect(x: textX, y: top + titleHeight, width: textWidth, height: subtitleHeight)
    if let dot = subtitlePresenceView {
      let labelWidth = min(subtitleLabel.intrinsicContentSize.width, max(0, textWidth - 12))
      let groupX = textX + (textWidth - labelWidth - 12) / 2
      dot.frame = CGRect(x: rtl ? groupX + labelWidth + 6 : groupX,
                         y: top + titleHeight + (subtitleHeight - 6) / 2, width: 6, height: 6)
      subtitleLabel.frame = CGRect(x: rtl ? groupX : groupX + 12,
                                   y: top + titleHeight, width: labelWidth, height: subtitleHeight)
    }
    let avatarX: CGFloat = rtl ? bounds.width - 44 : 12
    avatarView?.frame = CGRect(x: avatarX, y: (bounds.height - 32) / 2, width: 32, height: 32)
    presenceView?.frame = CGRect(x: avatarX + (rtl ? 0 : 24), y: (bounds.height - 32) / 2 + 24, width: 8, height: 8)
  }
}

private final class NavigationClipView: UIView {
  var onLayout: (() -> Void)?
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
  private let container: NavigationClipView
  private let material = UIVisualEffectView(effect: UIBlurEffect(style: .systemUltraThinMaterial))
  private let materialFade = CAGradientLayer()
  // A transparent, noninteractive viewport provides the system status fade
  // independently of the layout-only scroll view that collapses large titles.
  private let edgeScrollView = UIScrollView()
  private let content = NavigationContentController()
  private let navigation: UINavigationController
  private let channel: FlutterMethodChannel
  private var offset: CGFloat = 0
  private var alwaysFrosted = false
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
    if #available(iOS 27.0, *) {
      edgeScrollView.accessibilityIdentifier = "navigation-status-edge"
      edgeScrollView.isUserInteractionEnabled = false
      edgeScrollView.accessibilityElementsHidden = true
      edgeScrollView.backgroundColor = .clear
      edgeScrollView.contentInsetAdjustmentBehavior = .never
      edgeScrollView.contentSize = CGSize(width: 1, height: 10000)
      edgeScrollView.topEdgeEffect.style = .soft
      edgeScrollView.bottomEdgeEffect.isHidden = true
      container.addSubview(edgeScrollView)
    }
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
  }

  func view() -> UIView { container }

  private func layout() {
    // Only the bar is exposed by the platform-view clip. A full viewport is
    // needed for UIKit's scroll-edge and large-title calculations.
    guard !measuring else { return }
    let viewportHeight = container.window?.bounds.height ?? container.bounds.height
    edgeScrollView.frame = CGRect(x: 0, y: 0, width: container.bounds.width, height: viewportHeight)
    navigation.view.frame = edgeScrollView.frame
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
      glassBackedControls = content.navigationItem.titleView is NavigationTitleView
    } else {
      glassBackedControls = false
    }
    let statusOnly = alwaysFrosted && !navigation.navigationBar.prefersLargeTitles && glassBackedControls
    let height = statusOnly
      ? min(container.bounds.height, statusHeight > 0 ? statusHeight + 6 : 0)
      : container.bounds.height
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    material.frame = CGRect(x: 0, y: 0, width: container.bounds.width, height: height)
    materialFade.frame = material.bounds
    materialFade.locations = statusOnly
      ? [0, NSNumber(value: Double(max(0, height - 12) / max(1, height))), 1]
      : (navigation.navigationBar.prefersLargeTitles ? [0, 0.55, 1] : [0, 0.85, 1])
    CATransaction.commit()
    reportAvatarBounds()
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
    navigation.overrideUserInterfaceStyle = args["dark"] as? Bool == true ? .dark : .light
    material.overrideUserInterfaceStyle = navigation.overrideUserInterfaceStyle
    edgeScrollView.overrideUserInterfaceStyle = navigation.overrideUserInterfaceStyle
    // Let the ultra-thin system material provide its own adaptive tint.
    material.contentView.backgroundColor = .clear
    let bar = navigation.navigationBar
    let color = Self.color(args["foreground"])
    bar.tintColor = color
    titleColor = color
    largeTitleOpacity = -1
    let largeTitle = args["largeTitle"] as? Bool == true
    alwaysFrosted = args["alwaysFrosted"] as? Bool == true
    if #available(iOS 26.0, *) {
      if #available(iOS 27.0, *) { usesSystemScrollEdge = true }
      content.scrollView.topEdgeEffect.style = .soft
      content.scrollView.topEdgeEffect.isHidden = !usesSystemScrollEdge || !alwaysFrosted || largeTitle
      edgeScrollView.isHidden = alwaysFrosted && !largeTitle
    }
    // iOS 26 does not composite this native edge over sibling Flutter content
    // in our embedding, so retain the ultra-thin material fallback there.
    // Conversations use the navigation controller's native edge. Other pages
    // use a stationary native edge so UIKit's large-title inset adjustments
    // cannot hide the status-area fade when the title expands or collapses.
    material.isHidden = usesSystemScrollEdge
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
    let item = content.navigationItem
    item.title = args["title"] as? String
    if let subtitle = args["subtitle"] as? String {
      let button = NavigationTitleView(title: item.title, subtitle: subtitle, color: color)
      button.accessibilityIdentifier = "channel-navigation-title"
      let enabled = args["titleEnabled"] as? Bool == true
      button.accessibilityLabel = enabled
        ? "Open settings for \(item.title ?? ""), \(subtitle)"
        : "\(item.title ?? ""), \(subtitle)"
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
      button.frame.size = button.intrinsicContentSize
      item.titleView = button
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
      item.titleView = compactTitleContainer
    } else {
      item.titleView = nil
    }
    item.largeTitleDisplayMode = bar.prefersLargeTitles ? .always : .never
    trackedAvatar = nil
    reportedAvatarFrame = nil
    if let leading = args["leading"] as? [String: Any] {
      item.leftBarButtonItems = [makeItem(leading)]
    } else if args["back"] as? Bool == true {
      item.leftBarButtonItems = [makeItem(["id": "back", "label": "Back", "symbol": "chevron.backward", "enabled": true])]
    } else {
      item.leftBarButtonItems = nil
    }
    item.rightBarButtonItems = (args["actions"] as? [[String: Any]] ?? []).reversed().map(makeItem)
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
    DispatchQueue.main.async { [weak self] in
      self?.channel.invokeMethod("metrics", arguments: metrics)
    }
  }

  private func setScrollOffset(_ value: CGFloat) {
    offset = max(0, value)
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
    let symbol = (data["symbol"] as? String).flatMap { UIImage(systemName: $0) }
    return UIAction(title: data["label"] as? String ?? "", image: symbol,
                    attributes: enabled ? [] : [.disabled],
                    state: data["selected"] as? Bool == true ? .on : .off) { [weak self] _ in
      self?.channel.invokeMethod("action", arguments: id)
    }
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
    }
    item.accessibilityLabel = data["label"] as? String
    item.isEnabled = data["enabled"] as? Bool == true
    if data["avatarInitial"] is String { item.title = nil }
    if let encoded = data["imageData"] as? String,
       let bytes = Data(base64Encoded: encoded), let image = UIImage(data: bytes) {
      // Fill the 44-point native button with a 4-point inset. Keep the
      // original 24-point alignment footprint so UIKit does not widen it.
      let size = CGSize(width: 36, height: 36)
      item.image = UIGraphicsImageRenderer(size: size).image { _ in
        image.draw(in: CGRect(origin: .zero, size: size))
      }.withRenderingMode(.alwaysOriginal).withAlignmentRectInsets(
        UIEdgeInsets(top: 6, left: 6, bottom: 6, right: 6)
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
                     cornerRadius: size.width * (data["avatarIsAgent"] as? Bool == true ? 0.3 : 0.5)).fill()
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
    if data["tracksAvatarBounds"] as? Bool == true, data["id"] as? String == "leading",
       data["avatarInitial"] is String {
      // An explicit native button gives Flutter a public, measured destination
      // without depending on UINavigationBar's private view hierarchy.
      let button = UIButton(type: .custom)
      // UIKit adds the glass button's own padding around this custom view.
      // Match the 36-point image on both axes so that glass stays circular.
      button.frame = CGRect(x: 0, y: 0, width: 36, height: 36)
      button.widthAnchor.constraint(equalToConstant: 36).isActive = true
      button.heightAnchor.constraint(equalToConstant: 36).isActive = true
      button.setImage(item.image?.withAlignmentRectInsets(.zero), for: .normal)
      button.addAction(action, for: .touchUpInside)
      button.isEnabled = item.isEnabled
      button.accessibilityLabel = item.accessibilityLabel
      button.accessibilityIdentifier = "community-navigation-avatar"
      let hidden = data["avatarHidden"] as? Bool == true
      button.alpha = hidden ? 0 : 1
      button.accessibilityElementsHidden = hidden
      trackedAvatar = button
      return UIBarButtonItem(customView: button)
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
    channel.setMethodCallHandler(nil)
    navigation.willMove(toParent: nil)
    navigation.view.removeFromSuperview()
    navigation.removeFromParent()
  }
}
