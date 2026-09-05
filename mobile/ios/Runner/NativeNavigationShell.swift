import Flutter
import UIKit

final class NativeNavigationShellFactory: NSObject, FlutterPlatformViewFactory {
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
    NativeNavigationShellPlatformView(
      frame: frame,
      viewIdentifier: viewId,
      arguments: args,
      messenger: messenger
    )
  }
}

private final class NativeNavigationShellPlatformView: NSObject,
  FlutterPlatformView, UINavigationBarDelegate
{
  private let containerView: UIView
  private let navigationBar = UINavigationBar()
  private let rootItem = UINavigationItem(title: "")
  private let channel: FlutterMethodChannel
  private var activeItem: UINavigationItem?
  private var titleButton: UIButton?
  private var avatarTask: URLSessionDataTask?
  private var pendingHide: DispatchWorkItem?
  private var foregroundColor = UIColor.label
  private var currentArguments: [String: Any] = [:]

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?,
    messenger: FlutterBinaryMessenger
  ) {
    containerView = UIView(frame: frame)
    channel = FlutterMethodChannel(
      name: "buzz/native_navigation_shell/\(viewId)",
      binaryMessenger: messenger
    )
    super.init()

    containerView.backgroundColor = .clear
    containerView.isOpaque = false
    containerView.isHidden = true
    rootItem.backButtonDisplayMode = .minimal

    let topInset = (args as? [String: Any])?["topInset"] as? NSNumber
    let navigationHeight =
      (args as? [String: Any])?["navigationHeight"] as? NSNumber
    let resolvedTopInset = topInset?.doubleValue ?? 0
    let resolvedNavigationHeight = navigationHeight?.doubleValue ?? 56

    navigationBar.translatesAutoresizingMaskIntoConstraints = false
    navigationBar.delegate = self
    navigationBar.isTranslucent = true
    navigationBar.shadowImage = UIImage()
    navigationBar.setItems([rootItem], animated: false)
    applyTransparentAppearance()
    containerView.addSubview(navigationBar)
    NSLayoutConstraint.activate([
      navigationBar.leadingAnchor.constraint(equalTo: containerView.leadingAnchor),
      navigationBar.trailingAnchor.constraint(equalTo: containerView.trailingAnchor),
      navigationBar.topAnchor.constraint(
        equalTo: containerView.topAnchor,
        constant: resolvedTopInset
      ),
      navigationBar.heightAnchor.constraint(equalToConstant: resolvedNavigationHeight),
    ])

    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "setNavigation" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.setNavigation(call.arguments as? [String: Any] ?? [:])
      result(nil)
    }
  }

  func view() -> UIView {
    containerView
  }

  func navigationBar(
    _ navigationBar: UINavigationBar,
    shouldPop item: UINavigationItem
  ) -> Bool {
    guard item === activeItem else { return true }
    channel.invokeMethod("back", arguments: nil)
    activeItem = nil
    return true
  }

  private func setNavigation(_ arguments: [String: Any]) {
    pendingHide?.cancel()
    pendingHide = nil
    guard arguments["visible"] as? Bool == true else {
      hideNavigation()
      return
    }

    currentArguments = arguments
    foregroundColor = Self.color(arguments["foregroundColor"]) ?? .label
    containerView.overrideUserInterfaceStyle =
      arguments["brightness"] as? String == "dark" ? .dark : .light
    containerView.isHidden = false
    navigationBar.tintColor = foregroundColor

    if activeItem == nil {
      let item = UINavigationItem()
      activeItem = item
      configure(item, from: arguments)
      navigationBar.setItems([rootItem, item], animated: true)
    } else if let activeItem {
      configure(activeItem, from: arguments)
    }
  }

  private func hideNavigation() {
    avatarTask?.cancel()
    avatarTask = nil
    currentArguments = [:]
    if activeItem != nil {
      navigationBar.setItems([rootItem], animated: true)
      activeItem = nil
    }
    let work = DispatchWorkItem { [weak self] in
      guard self?.activeItem == nil else { return }
      self?.containerView.isHidden = true
    }
    pendingHide = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.28, execute: work)
  }

  private func configure(
    _ item: UINavigationItem,
    from arguments: [String: Any]
  ) {
    let titleButton = makeTitleButton(from: arguments)
    item.titleView = nil
    item.leftItemsSupplementBackButton = true
    item.leftBarButtonItems = [UIBarButtonItem(customView: titleButton)]
    item.rightBarButtonItems = makeActions(from: arguments)
    item.backButtonDisplayMode = .minimal
  }

  private func makeTitleButton(from arguments: [String: Any]) -> UIButton {
    avatarTask?.cancel()
    let button = UIButton(type: .system)
    titleButton = button
    button.accessibilityLabel =
      arguments["accessibilityLabel"] as? String
      ?? arguments["title"] as? String
    button.addAction(
      UIAction { [weak self] _ in
        self?.channel.invokeMethod("title", arguments: nil)
      },
      for: .touchUpInside
    )

    var configuration: UIButton.Configuration
    if #available(iOS 26.0, *) {
      // UINavigationBar supplies Liquid Glass for leading custom bar items.
      // Keeping glass on the nested button draws a second capsule.
      configuration = .plain()
    } else {
      configuration = .gray()
      configuration.baseBackgroundColor = .secondarySystemBackground
    }
    configuration.cornerStyle = .capsule
    configuration.baseForegroundColor = foregroundColor
    configuration.title = arguments["title"] as? String
    configuration.subtitle = arguments["subtitle"] as? String
    configuration.titleLineBreakMode = .byTruncatingTail
    configuration.imagePadding = 8
    configuration.contentInsets = NSDirectionalEdgeInsets(
      top: 6,
      leading: 10,
      bottom: 6,
      trailing: 12
    )
    configuration.image = titleImage(from: arguments)
    button.configuration = configuration
    button.titleLabel?.font = .preferredFont(forTextStyle: .headline)
    button.titleLabel?.adjustsFontForContentSizeCategory = true
    button.titleLabel?.lineBreakMode = .byTruncatingTail

    if let avatarURLString = arguments["avatarImageUrl"] as? String,
      let avatarURL = URL(string: avatarURLString)
    {
      avatarTask = URLSession.shared.dataTask(with: avatarURL) {
        [weak self, weak button] data, _, _ in
        guard let data, let image = UIImage(data: data) else { return }
        let avatar = Self.circularImage(image, diameter: 32)
        DispatchQueue.main.async {
          guard
            self?.currentArguments["avatarImageUrl"] as? String
              == avatarURLString,
            let button,
            var current = button.configuration
          else { return }
          current.image = avatar
          button.configuration = current
        }
      }
      avatarTask?.resume()
    }
    return button
  }

  private func titleImage(from arguments: [String: Any]) -> UIImage? {
    if arguments["avatarImageUrl"] != nil || arguments["avatarFallback"] != nil {
      let fallback = (arguments["avatarFallback"] as? String)?.first.map(String.init)
        ?? "?"
      return Self.fallbackAvatar(
        fallback,
        diameter: 32,
        foregroundColor: foregroundColor
      )
    }
    guard let symbol = arguments["systemIconName"] as? String else {
      return nil
    }
    return UIImage(
      systemName: symbol,
      withConfiguration: UIImage.SymbolConfiguration(pointSize: 13, weight: .semibold)
    )
  }

  private func makeActions(from arguments: [String: Any]) -> [UIBarButtonItem] {
    var actions: [UIBarButtonItem] = []
    if arguments["showsMore"] as? Bool == true {
      actions.append(
        makeAction(
          symbol: "ellipsis",
          accessibilityLabel: "Channel actions",
          method: "more"
        )
      )
    }
    if arguments["showsMembers"] as? Bool == true {
      actions.append(
        makeAction(
          symbol: "person.2.fill",
          accessibilityLabel: "View members",
          method: "members"
        )
      )
    }
    if arguments["showsHuddle"] as? Bool == true {
      let huddle = makeAction(
        symbol: "headphones",
        accessibilityLabel: arguments["huddleLabel"] as? String ?? "Huddle",
        method: "huddle"
      )
      huddle.isEnabled = arguments["huddleEnabled"] as? Bool == true
      actions.append(huddle)
    }
    return actions
  }

  private func makeAction(
    symbol: String,
    accessibilityLabel: String,
    method: String
  ) -> UIBarButtonItem {
    let image = UIImage(
      systemName: symbol,
      withConfiguration: UIImage.SymbolConfiguration(pointSize: 17, weight: .semibold)
    )
    let action = UIAction(
      title: accessibilityLabel,
      image: image
    ) { [weak self] _ in
      self?.channel.invokeMethod(method, arguments: nil)
    }
    let item = UIBarButtonItem(primaryAction: action)
    item.tintColor = foregroundColor
    item.accessibilityLabel = accessibilityLabel
    return item
  }

  private func applyTransparentAppearance() {
    let appearance = UINavigationBarAppearance()
    appearance.configureWithTransparentBackground()
    appearance.backgroundColor = .clear
    appearance.shadowColor = .clear
    navigationBar.standardAppearance = appearance
    navigationBar.scrollEdgeAppearance = appearance
    navigationBar.compactAppearance = appearance
  }

  private static func color(_ value: Any?) -> UIColor? {
    guard let argb = (value as? NSNumber)?.uint32Value else { return nil }
    return UIColor(
      red: CGFloat((argb >> 16) & 0xff) / 255,
      green: CGFloat((argb >> 8) & 0xff) / 255,
      blue: CGFloat(argb & 0xff) / 255,
      alpha: CGFloat((argb >> 24) & 0xff) / 255
    )
  }

  private static func fallbackAvatar(
    _ text: String,
    diameter: CGFloat,
    foregroundColor: UIColor
  ) -> UIImage {
    let renderer = UIGraphicsImageRenderer(size: CGSize(width: diameter, height: diameter))
    return renderer.image { context in
      UIColor.secondarySystemFill.setFill()
      context.cgContext.fillEllipse(in: CGRect(x: 0, y: 0, width: diameter, height: diameter))
      let attributes: [NSAttributedString.Key: Any] = [
        .font: UIFont.systemFont(ofSize: 14, weight: .semibold),
        .foregroundColor: foregroundColor,
      ]
      let size = text.size(withAttributes: attributes)
      text.draw(
        at: CGPoint(x: (diameter - size.width) / 2, y: (diameter - size.height) / 2),
        withAttributes: attributes
      )
    }
  }

  private static func circularImage(_ image: UIImage, diameter: CGFloat) -> UIImage {
    let renderer = UIGraphicsImageRenderer(size: CGSize(width: diameter, height: diameter))
    return renderer.image { _ in
      UIBezierPath(ovalIn: CGRect(x: 0, y: 0, width: diameter, height: diameter)).addClip()
      let scale = max(diameter / image.size.width, diameter / image.size.height)
      let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
      image.draw(
        in: CGRect(
          x: (diameter - size.width) / 2,
          y: (diameter - size.height) / 2,
          width: size.width,
          height: size.height
        )
      )
    }
  }

  deinit {
    pendingHide?.cancel()
    avatarTask?.cancel()
    channel.setMethodCallHandler(nil)
  }
}
