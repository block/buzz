import Flutter
import UIKit

final class NativeSkinToneControlFactory: NSObject, FlutterPlatformViewFactory {
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
    NativeSkinToneControlPlatformView(
      frame: frame,
      viewIdentifier: viewId,
      arguments: args,
      messenger: messenger
    )
  }
}

private final class NativeSkinToneControlPlatformView: NSObject, FlutterPlatformView {
  private let containerView: UIView
  private let channel: FlutterMethodChannel
  private let button = UIButton(type: .system)
  private var colors: [UIColor] = []
  private var labels: [String] = []
  private var value = 0

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?,
    messenger: FlutterBinaryMessenger
  ) {
    containerView = UIView(frame: frame)
    channel = FlutterMethodChannel(
      name: "buzz/native_skin_tone_control/\(viewId)",
      binaryMessenger: messenger
    )
    super.init()

    let arguments = args as? [String: Any]
    colors = (arguments?["colors"] as? [NSNumber] ?? []).map {
      Self.color(fromARGB: $0.uint32Value)
    }
    labels = arguments?["labels"] as? [String] ?? []
    value = (arguments?["value"] as? NSNumber)?.intValue ?? 0
    let style = arguments?["brightness"] as? String == "dark"
      ? UIUserInterfaceStyle.dark
      : UIUserInterfaceStyle.light
    containerView.overrideUserInterfaceStyle = style

    containerView.backgroundColor = .clear
    containerView.isOpaque = false
    button.translatesAutoresizingMaskIntoConstraints = false
    button.accessibilityLabel = "Skin tone"
    button.showsMenuAsPrimaryAction = true
    containerView.addSubview(button)
    NSLayoutConstraint.activate([
      button.centerXAnchor.constraint(equalTo: containerView.centerXAnchor),
      button.centerYAnchor.constraint(equalTo: containerView.centerYAnchor),
      button.widthAnchor.constraint(equalToConstant: 44),
      button.heightAnchor.constraint(equalToConstant: 44),
    ])

    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "setValue", let value = call.arguments as? NSNumber else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.value = value.intValue
      self?.refresh()
      result(nil)
    }
    refresh()
  }

  func view() -> UIView { containerView }

  private func refresh() {
    guard !colors.isEmpty else { return }
    value = min(max(0, value), colors.count - 1)
    var configuration: UIButton.Configuration
    if #available(iOS 26.0, *) {
      configuration = .glass()
    } else {
      configuration = .gray()
      configuration.baseBackgroundColor = .secondarySystemBackground
    }
    configuration.cornerStyle = .capsule
    configuration.image = Self.toneImage(color: colors[value], size: 20)
    button.configuration = configuration
    button.menu = UIMenu(children: colors.indices.map { index in
      UIAction(
        title: index < labels.count ? labels[index] : "Skin tone \(index + 1)",
        image: Self.toneImage(color: colors[index], size: 18),
        state: index == value ? .on : .off
      ) { [weak self] _ in
        self?.value = index
        self?.refresh()
        self?.channel.invokeMethod("changed", arguments: index)
      }
    })
  }

  private static func toneImage(color: UIColor, size: CGFloat) -> UIImage {
    let renderer = UIGraphicsImageRenderer(size: CGSize(width: size, height: size))
    return renderer.image { context in
      let rect = CGRect(x: 0.5, y: 0.5, width: size - 1, height: size - 1)
      color.setFill()
      context.cgContext.fillEllipse(in: rect)
      UIColor.label.withAlphaComponent(0.35).setStroke()
      context.cgContext.setLineWidth(1)
      context.cgContext.strokeEllipse(in: rect)
    }.withRenderingMode(.alwaysOriginal)
  }

  private static func color(fromARGB value: UInt32) -> UIColor {
    UIColor(
      red: CGFloat((value >> 16) & 0xff) / 255,
      green: CGFloat((value >> 8) & 0xff) / 255,
      blue: CGFloat(value & 0xff) / 255,
      alpha: CGFloat((value >> 24) & 0xff) / 255
    )
  }

  deinit { channel.setMethodCallHandler(nil) }
}
