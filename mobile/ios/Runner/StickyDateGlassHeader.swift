import Flutter
import UIKit

private final class StickyDateGlassView: UIVisualEffectView {
  override func layoutSubviews() {
    super.layoutSubviews()
    layer.cornerRadius = bounds.height / 2
  }
}

final class StickyDateGlassHeaderFactory: NSObject, FlutterPlatformViewFactory {
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
    StickyDateGlassHeaderPlatformView(
      frame: frame,
      viewIdentifier: viewId,
      arguments: args,
      messenger: messenger
    )
  }
}

final class StickyDateGlassHeaderPlatformView: NSObject, FlutterPlatformView {
  private let glassView: StickyDateGlassView
  private let channel: FlutterMethodChannel
  private let dateLabel = UILabel()

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?,
    messenger: FlutterBinaryMessenger
  ) {
    let arguments = args as? [String: Any]
    let text = arguments?["label"] as? String ?? ""
    channel = FlutterMethodChannel(
      name: "buzz/sticky_date_glass/\(viewId)",
      binaryMessenger: messenger
    )

    if #available(iOS 26.0, *) {
      let glassEffect = UIGlassEffect(style: .regular)
      glassEffect.isInteractive = false
      glassView = StickyDateGlassView(effect: glassEffect)
    } else {
      glassView = StickyDateGlassView(
        effect: UIBlurEffect(style: .systemMaterial)
      )
    }

    super.init()

    glassView.frame = frame
    glassView.isOpaque = false
    glassView.isUserInteractionEnabled = false
    glassView.clipsToBounds = true
    glassView.layer.cornerCurve = .continuous
    applyBrightness(from: arguments?["brightness"])

    dateLabel.translatesAutoresizingMaskIntoConstraints = false
    dateLabel.text = text
    dateLabel.textAlignment = .center
    dateLabel.textColor = .secondaryLabel
    dateLabel.font = UIFontMetrics(forTextStyle: .caption1).scaledFont(
      for: UIFont.systemFont(ofSize: 14, weight: .medium)
    )
    dateLabel.adjustsFontForContentSizeCategory = true
    dateLabel.numberOfLines = 1
    dateLabel.lineBreakMode = .byTruncatingTail
    dateLabel.isAccessibilityElement = false

    glassView.contentView.addSubview(dateLabel)
    NSLayoutConstraint.activate([
      dateLabel.leadingAnchor.constraint(
        equalTo: glassView.contentView.leadingAnchor,
        constant: 12
      ),
      dateLabel.trailingAnchor.constraint(
        equalTo: glassView.contentView.trailingAnchor,
        constant: -12
      ),
      dateLabel.centerYAnchor.constraint(
        equalTo: glassView.contentView.centerYAnchor
      ),
    ])

    channel.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "setLabel":
        guard let text = call.arguments as? String else {
          result(FlutterMethodNotImplemented)
          return
        }
        self?.dateLabel.text = text
        result(nil)
      case "setBrightness":
        self?.applyBrightness(from: call.arguments)
        result(nil)
      default:
        result(FlutterMethodNotImplemented)
      }
    }
  }

  func view() -> UIView {
    glassView
  }

  private func applyBrightness(from value: Any?) {
    let interfaceStyle: UIUserInterfaceStyle = value as? String == "dark" ? .dark : .light
    glassView.overrideUserInterfaceStyle = interfaceStyle
  }

  deinit {
    channel.setMethodCallHandler(nil)
  }
}
