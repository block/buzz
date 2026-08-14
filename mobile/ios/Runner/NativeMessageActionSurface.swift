import Flutter
import UIKit

struct NativeMessageActionDefinition {
  enum Group: String, CaseIterable {
    case primary
    case utility
    case destructive
  }

  let id: String
  let title: String
  let symbol: String
  let group: Group
  let isDestructive: Bool

  init?(arguments: [String: Any]) {
    guard
      let id = arguments["id"] as? String,
      let title = arguments["title"] as? String,
      let symbol = arguments["symbol"] as? String,
      let groupName = arguments["group"] as? String,
      let group = Group(rawValue: groupName)
    else {
      return nil
    }

    self.id = id
    self.title = title
    self.symbol = symbol
    self.group = group
    isDestructive = arguments["destructive"] as? Bool ?? false
  }
}

enum NativeMessageActionSurfaceLayout {
  static let rowHeight: CGFloat = 48
  static let separatorHeight: CGFloat = 0.5
  static let verticalInset: CGFloat = 4
  static let horizontalInset: CGFloat = 16
  static let iconColumnWidth: CGFloat = 32
  static let iconToTextSpacing: CGFloat = 12

  static var cornerRadius: CGFloat {
    if #available(iOS 26.0, *) {
      return 33
    }
    return 12
  }

  static func populatedGroups(
    actions: [NativeMessageActionDefinition]
  ) -> [NativeMessageActionDefinition.Group] {
    NativeMessageActionDefinition.Group.allCases.filter { group in
      actions.contains { $0.group == group }
    }
  }

  static func separatorCount(
    actions: [NativeMessageActionDefinition]
  ) -> Int {
    max(0, populatedGroups(actions: actions).count - 1)
  }

  static func preferredHeight(
    actions: [NativeMessageActionDefinition]
  ) -> CGFloat {
    (verticalInset * 2)
      + (CGFloat(actions.count) * rowHeight)
      + (CGFloat(separatorCount(actions: actions)) * separatorHeight)
  }
}

@available(iOS 16.0, *)
enum NativeMessageActionSurfaceAppearance {
  static func backdropEffect(reduceTransparency: Bool) -> UIVisualEffect? {
    guard !reduceTransparency else { return nil }

    if #available(iOS 26.0, *) {
      let effect = UIGlassEffect(style: .regular)
      effect.isInteractive = true
      return effect
    }
    return UIBlurEffect(style: .systemMaterial)
  }
}

@available(iOS 16.0, *)
final class NativeMessageActionRowControl: UIControl {
  let actionImageView: UIImageView
  let actionTitleLabel = UILabel()

  init(
    definition: NativeMessageActionDefinition,
    foregroundColor: UIColor,
    destructiveColor: UIColor,
    onSelected: @escaping () -> Void
  ) {
    actionImageView = UIImageView(image: UIImage(systemName: definition.symbol))
    super.init(frame: .zero)

    let color = definition.isDestructive ? destructiveColor : foregroundColor
    actionImageView.tintColor = color
    actionImageView.contentMode = .center

    actionTitleLabel.text = definition.title
    actionTitleLabel.textColor = color
    actionTitleLabel.font = UIFont.preferredFont(forTextStyle: .body)
    actionTitleLabel.adjustsFontForContentSizeCategory = true
    actionTitleLabel.numberOfLines = 1
    actionTitleLabel.adjustsFontSizeToFitWidth = true
    actionTitleLabel.minimumScaleFactor = 0.8

    let iconColumn = UIView()
    iconColumn.translatesAutoresizingMaskIntoConstraints = false
    actionImageView.translatesAutoresizingMaskIntoConstraints = false
    actionTitleLabel.translatesAutoresizingMaskIntoConstraints = false
    iconColumn.addSubview(actionImageView)
    addSubview(iconColumn)
    addSubview(actionTitleLabel)

    NSLayoutConstraint.activate([
      iconColumn.leadingAnchor.constraint(
        equalTo: leadingAnchor,
        constant: NativeMessageActionSurfaceLayout.horizontalInset
      ),
      iconColumn.centerYAnchor.constraint(equalTo: centerYAnchor),
      iconColumn.widthAnchor.constraint(
        equalToConstant: NativeMessageActionSurfaceLayout.iconColumnWidth
      ),
      iconColumn.heightAnchor.constraint(
        equalToConstant: NativeMessageActionSurfaceLayout.iconColumnWidth
      ),
      actionImageView.leadingAnchor.constraint(equalTo: iconColumn.leadingAnchor),
      actionImageView.trailingAnchor.constraint(equalTo: iconColumn.trailingAnchor),
      actionImageView.topAnchor.constraint(equalTo: iconColumn.topAnchor),
      actionImageView.bottomAnchor.constraint(equalTo: iconColumn.bottomAnchor),
      actionTitleLabel.leadingAnchor.constraint(
        equalTo: iconColumn.trailingAnchor,
        constant: NativeMessageActionSurfaceLayout.iconToTextSpacing
      ),
      actionTitleLabel.trailingAnchor.constraint(
        equalTo: trailingAnchor,
        constant: -NativeMessageActionSurfaceLayout.horizontalInset
      ),
      actionTitleLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
      heightAnchor.constraint(
        greaterThanOrEqualToConstant: NativeMessageActionSurfaceLayout.rowHeight
      ),
    ])

    accessibilityLabel = definition.title
    accessibilityTraits = .button
    isAccessibilityElement = true
    addAction(UIAction { _ in onSelected() }, for: .touchUpInside)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  override var isHighlighted: Bool {
    didSet {
      backgroundColor =
        isHighlighted
        ? actionTitleLabel.textColor.withAlphaComponent(0.08)
        : .clear
    }
  }
}

@available(iOS 16.0, *)
final class NativeMessageActionSeparatorView: UIView {
  init(color: UIColor) {
    super.init(frame: .zero)
    backgroundColor = color
    heightAnchor.constraint(
      equalToConstant: NativeMessageActionSurfaceLayout.separatorHeight
    ).isActive = true
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }
}

@available(iOS 16.0, *)
final class NativeMessageActionSurfaceFactory: NSObject,
  FlutterPlatformViewFactory
{
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
    NativeMessageActionSurfacePlatformView(
      frame: frame,
      viewIdentifier: viewId,
      messenger: messenger,
      arguments: args
    )
  }
}

@available(iOS 16.0, *)
final class NativeMessageActionSurfacePlatformView: NSObject,
  FlutterPlatformView
{
  private let surfaceView: UIView
  private let backdropView: UIVisualEffectView
  private let channel: FlutterMethodChannel

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    messenger: FlutterBinaryMessenger,
    arguments args: Any?
  ) {
    let arguments = args as? [String: Any]
    let surfaceColor = Self.color(
      from: arguments?["surfaceColor"],
      fallback: .systemBackground
    )
    let foregroundColor = Self.color(
      from: arguments?["foregroundColor"],
      fallback: .label
    )
    let separatorColor = Self.color(
      from: arguments?["separatorColor"],
      fallback: .separator
    )
    let destructiveColor = Self.color(
      from: arguments?["errorColor"],
      fallback: .systemRed
    )
    let actionArguments = arguments?["actions"] as? [[String: Any]]
    let actions =
      actionArguments?.compactMap(
        NativeMessageActionDefinition.init(arguments:)
      ) ?? []

    surfaceView = UIView(frame: frame)
    backdropView = UIVisualEffectView(
      effect: NativeMessageActionSurfaceAppearance.backdropEffect(
        reduceTransparency: UIAccessibility.isReduceTransparencyEnabled
      )
    )
    channel = FlutterMethodChannel(
      name: "buzz/native_message_action_surface/\(viewId)",
      binaryMessenger: messenger
    )
    super.init()

    surfaceView.backgroundColor = .clear
    surfaceView.clipsToBounds = false
    surfaceView.accessibilityViewIsModal = true

    backdropView.translatesAutoresizingMaskIntoConstraints = false
    backdropView.layer.cornerRadius = NativeMessageActionSurfaceLayout.cornerRadius
    backdropView.layer.cornerCurve = .continuous
    backdropView.layer.masksToBounds = true
    if backdropView.effect == nil {
      backdropView.backgroundColor = surfaceColor
    }
    surfaceView.addSubview(backdropView)
    NSLayoutConstraint.activate([
      backdropView.leadingAnchor.constraint(equalTo: surfaceView.leadingAnchor),
      backdropView.trailingAnchor.constraint(equalTo: surfaceView.trailingAnchor),
      backdropView.topAnchor.constraint(equalTo: surfaceView.topAnchor),
      backdropView.bottomAnchor.constraint(equalTo: surfaceView.bottomAnchor),
    ])

    if #unavailable(iOS 26.0) {
      surfaceView.layer.cornerRadius = NativeMessageActionSurfaceLayout.cornerRadius
      surfaceView.layer.shadowRadius = 32
      surfaceView.layer.shadowOffset = CGSize(width: 0, height: 16)
      surfaceView.layer.shadowColor = UIColor.black.cgColor
      surfaceView.layer.shadowOpacity = 0.2
    }

    install(
      actions: actions,
      foregroundColor: foregroundColor,
      destructiveColor: destructiveColor,
      separatorColor: separatorColor
    )
  }

  func view() -> UIView {
    surfaceView
  }

  private func install(
    actions: [NativeMessageActionDefinition],
    foregroundColor: UIColor,
    destructiveColor: UIColor,
    separatorColor: UIColor
  ) {
    let scrollView = UIScrollView()
    scrollView.translatesAutoresizingMaskIntoConstraints = false
    scrollView.alwaysBounceVertical = false
    scrollView.showsVerticalScrollIndicator = false
    scrollView.contentInsetAdjustmentBehavior = .never

    let stack = UIStackView()
    stack.axis = .vertical
    stack.spacing = 0
    stack.translatesAutoresizingMaskIntoConstraints = false

    backdropView.contentView.addSubview(scrollView)
    scrollView.addSubview(stack)
    NSLayoutConstraint.activate([
      scrollView.leadingAnchor.constraint(equalTo: backdropView.contentView.leadingAnchor),
      scrollView.trailingAnchor.constraint(equalTo: backdropView.contentView.trailingAnchor),
      scrollView.topAnchor.constraint(equalTo: backdropView.contentView.topAnchor),
      scrollView.bottomAnchor.constraint(equalTo: backdropView.contentView.bottomAnchor),
      stack.leadingAnchor.constraint(equalTo: scrollView.contentLayoutGuide.leadingAnchor),
      stack.trailingAnchor.constraint(equalTo: scrollView.contentLayoutGuide.trailingAnchor),
      stack.topAnchor.constraint(
        equalTo: scrollView.contentLayoutGuide.topAnchor,
        constant: NativeMessageActionSurfaceLayout.verticalInset
      ),
      stack.bottomAnchor.constraint(
        equalTo: scrollView.contentLayoutGuide.bottomAnchor,
        constant: -NativeMessageActionSurfaceLayout.verticalInset
      ),
      stack.widthAnchor.constraint(equalTo: scrollView.frameLayoutGuide.widthAnchor),
    ])

    var installedGroup = false
    for group in NativeMessageActionDefinition.Group.allCases {
      let groupActions = actions.filter { $0.group == group }
      guard !groupActions.isEmpty else { continue }
      if installedGroup {
        stack.addArrangedSubview(
          NativeMessageActionSeparatorView(color: separatorColor)
        )
      }
      for definition in groupActions {
        stack.addArrangedSubview(
          NativeMessageActionRowControl(
            definition: definition,
            foregroundColor: foregroundColor,
            destructiveColor: destructiveColor,
            onSelected: { [weak self] in self?.select(definition) }
          )
        )
      }
      installedGroup = true
    }
  }

  private func select(_ definition: NativeMessageActionDefinition) {
    channel.invokeMethod("selected", arguments: ["id": definition.id])
  }

  private static func color(from value: Any?, fallback: UIColor) -> UIColor {
    guard let number = value as? NSNumber else { return fallback }
    let color = number.uint32Value
    let alpha = CGFloat((color >> 24) & 0xFF) / 255
    let red = CGFloat((color >> 16) & 0xFF) / 255
    let green = CGFloat((color >> 8) & 0xFF) / 255
    let blue = CGFloat(color & 0xFF) / 255
    return UIColor(red: red, green: green, blue: blue, alpha: alpha)
  }
}
