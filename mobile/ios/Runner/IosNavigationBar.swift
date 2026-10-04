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

final class NavigationTitleView: UIView, UIGestureRecognizerDelegate {
  var maximumWidth: CGFloat = 240 {
    didSet { if maximumWidth != oldValue { invalidateIntrinsicContentSize() } }
  }
  var onActivate: (() -> Void)?
  var onExpiryPressed: ((String) -> Void)?
  private let titleLabel = UILabel()
  private let subtitleLabel = UILabel()
  private var avatarView: UIImageView?
  private var presenceView: UIView?
  private var subtitlePresenceView: UIView?
  private var expiryView: UIButton?

  init(title: String?, subtitle: String, color: UIColor) {
    super.init(frame: .zero)
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
      addSubview(label)
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
    addSubview(avatar)
    avatarView = avatar
    for label in [titleLabel, subtitleLabel] { label.textAlignment = .natural }
    if let presence {
      let badge = UIView()
      badge.backgroundColor = presence
      badge.layer.cornerRadius = 4
      badge.layer.borderWidth = 1.5
      badge.layer.borderColor = UIColor.systemBackground.cgColor
      badge.accessibilityIdentifier = "dm-navigation-presence"
      addSubview(badge)
      presenceView = badge
    }
  }

  func setSubtitlePresence(_ color: UIColor) {
    let dot = UIView()
    dot.backgroundColor = color
    dot.layer.cornerRadius = 3
    dot.isAccessibilityElement = false
    dot.accessibilityIdentifier = "dm-navigation-status-dot"
    addSubview(dot)
    subtitlePresenceView = dot
    invalidateIntrinsicContentSize()
  }

  func setEphemeralStatus(_ label: String) {
    let clock = UIButton(type: .custom)
    clock.setImage(UIImage(systemName: "clock"), for: .normal)
    clock.tintColor = .secondaryLabel
    clock.addAction(UIAction { [weak self] _ in self?.onExpiryPressed?(label) }, for: .touchUpInside)
    clock.accessibilityIdentifier = "navigation-ephemeral-status"
    // The title is one accessibility element; include the full retention
    // explanation there so the clock is never announced without its meaning.
    clock.isAccessibilityElement = false
    addSubview(clock)
    expiryView = clock
    accessibilityLabel = [accessibilityLabel, label].compactMap { $0 }.joined(separator: ", ")
  }

  func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
    acceptsTitleTouch(in: touch.view)
  }

  // UIKit can wrap titleView in a UIControl. Only controls inside our title
  // (such as the retention disclosure) should consume the title tap.
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
    CGSize(width: min(maximumWidth, max(titleLabel.intrinsicContentSize.width, subtitleLabel.intrinsicContentSize.width + (subtitlePresenceView == nil ? 0 : 12)) + 16 + (avatarView == nil ? 0 : 40) + (expiryView == nil ? 0 : 44)),
           height: max(44, titleLabel.intrinsicContentSize.height + subtitleLabel.intrinsicContentSize.height))
  }

  override func layoutSubviews() {
    super.layoutSubviews()
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
    let statusWidth: CGFloat = expiryView == nil ? 0 : 44
    let textX: CGFloat = (hasAvatar && !rtl ? 48 : 8) + (rtl ? statusWidth : 0)
    let textWidth = max(0, bounds.width - 16 - (hasAvatar ? 40 : 0) - statusWidth)
    expiryView?.frame = CGRect(x: rtl ? 8 : bounds.width - 52,
                              y: 0, width: 44, height: bounds.height)
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
    let avatarX: CGFloat = rtl ? bounds.width - 40 : 8
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
      // This scroll view only drives title layout. Its automatic edge effect
      // otherwise adds a second, hard-edged backdrop when the title collapses.
      // The material behind the navigation controller owns the visible blur.
      scrollView.topEdgeEffect.isHidden = true
      scrollView.bottomEdgeEffect.isHidden = true
    }
  }
}

private final class IosNavigationBarView: NSObject, FlutterPlatformView {
  private let container: NavigationClipView
  private let material = UIVisualEffectView(effect: UIBlurEffect(style: .systemUltraThinMaterial))
  private let materialFade = CAGradientLayer()
  private let content = NavigationContentController()
  private let navigation: UINavigationController
  private let channel: FlutterMethodChannel
  private var offset: CGFloat = 0
  private var expandedBarHeight: CGFloat = 0
  private var measuredWidth: CGFloat = 0
  private var measuredSafeTop: CGFloat = -1
  private var measuredCategory: UIContentSizeCategory?
  private var measuring = false
  private var metrics: [String: Any]?

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
    material.frame = container.bounds
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    materialFade.frame = material.bounds
    CATransaction.commit()
    let viewportHeight = container.window?.bounds.height ?? container.bounds.height
    navigation.view.frame = CGRect(x: 0, y: 0, width: container.bounds.width, height: viewportHeight)
    navigation.view.layoutIfNeeded()
    measureIfNeeded()
    applyScroll()
  }

  private func configure(_ args: [String: Any]) {
    navigation.overrideUserInterfaceStyle = args["dark"] as? Bool == true ? .dark : .light
    material.overrideUserInterfaceStyle = navigation.overrideUserInterfaceStyle
    // Refresh the system material tint whenever the Flutter theme changes.
    material.contentView.backgroundColor = args["background"] is NSNumber
      ? Self.color(args["background"]).withAlphaComponent(0.25) : .clear
    let bar = navigation.navigationBar
    let color = Self.color(args["foreground"])
    bar.tintColor = color
    let appearance = UINavigationBarAppearance()
    // The mirrored UIScrollView contains no rendered Flutter content, so
    // UIKit's automatic scroll-edge treatment cannot detect its backdrop.
    // A native material below the bar samples the real composited page instead.
    appearance.configureWithTransparentBackground()
    appearance.titleTextAttributes = [.foregroundColor: color]
    appearance.largeTitleTextAttributes = [.foregroundColor: color]
    bar.standardAppearance = appearance
    bar.scrollEdgeAppearance = appearance
    bar.compactAppearance = appearance
    let largeTitle = args["largeTitle"] as? Bool == true
    if bar.prefersLargeTitles != largeTitle { measuredWidth = 0 }
    bar.prefersLargeTitles = largeTitle
    // A title alone does not need a backdrop. Reveal material only as the
    // page scrolls beneath the navigation controls, for every title size.
    material.alpha = min(1, offset / 12)
    // Use the same ultra-thin material and soft lower edge on every page.
    // Compact titles have a member-count line, so begin their fade below it
    // rather than washing out the subtitle or ending in a hard rectangle.
    materialFade.locations = largeTitle ? [0, 0.55, 1] : [0, 0.85, 1]
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
      button.isUserInteractionEnabled = enabled || args["ephemeralLabel"] is String
      if enabled {
        button.onActivate = { [weak self] in self?.channel.invokeMethod("action", arguments: "title") }
      }
      button.onExpiryPressed = { [weak self] label in
        guard let self, self.content.presentedViewController == nil else { return }
        let disclosure = UIAlertController(title: "Temporary conversation", message: label, preferredStyle: .alert)
        disclosure.addAction(UIAlertAction(title: "OK", style: .default))
        self.content.present(disclosure, animated: true)
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
    } else {
      item.titleView = nil
    }
    item.largeTitleDisplayMode = bar.prefersLargeTitles ? .always : .never
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
    material.alpha = min(1, offset / 12)
    applyScroll()
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
    let desired = CGPoint(x: 0, y: -topInset + offset)
    if abs(scroll.contentOffset.y - desired.y) > 0.1 {
      scroll.setContentOffset(desired, animated: false)
      navigation.view.layoutIfNeeded()
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
