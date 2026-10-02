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
    let item = content.navigationItem
    item.title = args["title"] as? String
    if let subtitle = args["subtitle"] as? String {
      var configuration = UIButton.Configuration.plain()
      configuration.title = item.title
      configuration.subtitle = subtitle
      configuration.titleAlignment = .center
      configuration.titleLineBreakMode = .byTruncatingTail
      configuration.subtitleLineBreakMode = .byTruncatingTail
      configuration.contentInsets = .zero
      configuration.baseForegroundColor = color
      configuration.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
        var attributes = incoming
        attributes.font = UIFont.preferredFont(forTextStyle: .headline)
        return attributes
      }
      configuration.subtitleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
        var attributes = incoming
        attributes.font = UIFont.preferredFont(forTextStyle: .caption1)
        attributes.foregroundColor = UIColor.secondaryLabel
        return attributes
      }
      let button = UIButton(configuration: configuration, primaryAction: UIAction { [weak self] _ in
        self?.channel.invokeMethod("action", arguments: "title")
      })
      button.accessibilityIdentifier = "channel-navigation-title"
      button.accessibilityLabel = "Open settings for \(item.title ?? ""), \(subtitle)"
      button.isUserInteractionEnabled = args["titleEnabled"] as? Bool == true
      button.sizeToFit()
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
        UIBezierPath(ovalIn: CGRect(origin: .zero, size: size)).fill()
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
