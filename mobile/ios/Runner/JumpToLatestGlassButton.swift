import Flutter
import UIKit

final class JumpToLatestGlassButtonFactory: NSObject, FlutterPlatformViewFactory {
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
    JumpToLatestGlassButtonPlatformView(
      frame: frame,
      viewIdentifier: viewId,
      arguments: args,
      messenger: messenger
    )
  }
}

private final class JumpToLatestGlassButton: UIButton {
  private static let hitTargetExpansion: CGFloat = 4

  override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
    bounds
      .insetBy(
        dx: -Self.hitTargetExpansion,
        dy: -Self.hitTargetExpansion
      )
      .contains(point)
  }
}

final class JumpToLatestGlassButtonPlatformView: NSObject, FlutterPlatformView {
  private let containerView: UIView
  private let channel: FlutterMethodChannel
  private let button = JumpToLatestGlassButton(type: .system)

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?,
    messenger: FlutterBinaryMessenger
  ) {
    containerView = UIView(frame: frame)
    channel = FlutterMethodChannel(
      name: "buzz/jump_to_latest_glass/\(viewId)",
      binaryMessenger: messenger
    )
    super.init()

    containerView.backgroundColor = .clear
    containerView.isOpaque = false
    applyBrightness(from: args)

    var configuration: UIButton.Configuration
    if #available(iOS 26.0, *) {
      configuration = .glass()
    } else {
      configuration = .gray()
      configuration.baseBackgroundColor = UIColor.secondarySystemBackground
    }
    configuration.cornerStyle = .capsule
    configuration.baseForegroundColor = .label
    configuration.image = UIImage(
      systemName: "arrow.down",
      withConfiguration: UIImage.SymbolConfiguration(
        pointSize: 16,
        weight: .semibold
      )
    )
    button.configuration = configuration
    button.accessibilityLabel = "Jump to latest message"
    button.translatesAutoresizingMaskIntoConstraints = false
    button.addAction(
      UIAction { [weak self] _ in
        self?.channel.invokeMethod("pressed", arguments: nil)
      },
      for: .touchUpInside
    )

    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "setBrightness" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.applyBrightness(from: call.arguments)
      result(nil)
    }

    containerView.addSubview(button)
    NSLayoutConstraint.activate([
      button.centerXAnchor.constraint(equalTo: containerView.centerXAnchor),
      button.bottomAnchor.constraint(equalTo: containerView.bottomAnchor),
      button.widthAnchor.constraint(equalToConstant: 40),
      button.heightAnchor.constraint(equalToConstant: 40),
    ])
  }

  func view() -> UIView {
    containerView
  }

  private func applyBrightness(from value: Any?) {
    let brightness = (value as? [String: Any])?["brightness"] as? String
      ?? value as? String
    let interfaceStyle: UIUserInterfaceStyle = brightness == "dark" ? .dark : .light
    containerView.overrideUserInterfaceStyle = interfaceStyle
    button.overrideUserInterfaceStyle = interfaceStyle
    button.setNeedsUpdateConfiguration()
  }

  deinit {
    channel.setMethodCallHandler(nil)
  }
}
