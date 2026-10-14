import Flutter
import UIKit

final class NativePresenceMenuFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger
  init(messenger: FlutterBinaryMessenger) { self.messenger = messenger; super.init() }
  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol { FlutterStandardMessageCodec.sharedInstance() }
  func create(withFrame frame: CGRect, viewIdentifier viewId: Int64, arguments args: Any?) -> FlutterPlatformView {
    NativePresenceMenu(frame: frame, id: viewId, args: args, messenger: messenger)
  }
}

private final class NativePresenceMenu: NSObject, FlutterPlatformView {
  private let container: UIView
  private let button = UIButton(type: .system)
  private let channel: FlutterMethodChannel

  init(frame: CGRect, id: Int64, args: Any?, messenger: FlutterBinaryMessenger) {
    container = UIView(frame: frame)
    channel = FlutterMethodChannel(name: "buzz/presence_menu/\(id)", binaryMessenger: messenger)
    super.init()
    container.backgroundColor = .clear
    container.isOpaque = false
    button.translatesAutoresizingMaskIntoConstraints = false
    button.showsMenuAsPrimaryAction = true
    container.addSubview(button)
    NSLayoutConstraint.activate([
      button.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      button.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      button.topAnchor.constraint(equalTo: container.topAnchor),
      button.bottomAnchor.constraint(equalTo: container.bottomAnchor),
    ])
    update(args as? [String: Any] ?? [:])
    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "update", let data = call.arguments as? [String: Any] else {
        result(FlutterMethodNotImplemented); return
      }
      self?.update(data)
      result(nil)
    }
  }

  func view() -> UIView { container }

  private func update(_ data: [String: Any]) {
    let presence = data["presence"] as? String ?? "offline"
    let label = data["label"] as? String ?? "Offline"
    container.overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
    var configuration = UIButton.Configuration.plain()
    configuration.title = label
    configuration.baseForegroundColor = Self.color(data["foreground"])
    configuration.background.backgroundColor = Self.color(data["background"])
    configuration.background.cornerRadius = 100
    configuration.background.backgroundInsets = NSDirectionalEdgeInsets(top: 7, leading: 0, bottom: 7, trailing: 0)
    configuration.contentInsets = NSDirectionalEdgeInsets(top: 7, leading: 12, bottom: 7, trailing: 12)
    let fontSize = (data["fontSize"] as? NSNumber)?.doubleValue ?? 15
    configuration.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
      var attributes = incoming
      attributes.font = UIFont.systemFont(ofSize: fontSize, weight: .medium)
      return attributes
    }
    button.configuration = configuration
    button.accessibilityLabel = "Availability"
    button.accessibilityValue = label
    let options: [(String, String, UIColor)] = [("online", "Online", .systemGreen), ("away", "Away", .systemOrange), ("offline", "Offline", .systemGray)]
    button.menu = UIMenu(children: options.map { value, title, color in
      UIAction(title: title,
        image: UIImage(systemName: "circle.fill")?.withTintColor(color, renderingMode: .alwaysOriginal),
        state: presence == value ? .on : .off
      ) { [weak self] _ in self?.channel.invokeMethod("selected", arguments: value) }
    })
  }

  private static func color(_ value: Any?) -> UIColor {
    let argb = (value as? NSNumber)?.uint32Value ?? 0
    return UIColor(red: CGFloat((argb >> 16) & 255) / 255, green: CGFloat((argb >> 8) & 255) / 255,
      blue: CGFloat(argb & 255) / 255, alpha: CGFloat((argb >> 24) & 255) / 255)
  }
}
