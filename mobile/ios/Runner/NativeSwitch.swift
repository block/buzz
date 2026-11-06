import Flutter
import UIKit

final class NativeSwitchFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger
  init(messenger: FlutterBinaryMessenger) {
    self.messenger = messenger
    super.init()
  }
  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }
  func create(withFrame frame: CGRect, viewIdentifier viewId: Int64, arguments args: Any?) -> FlutterPlatformView {
    NativeSwitchPlatformView(frame: frame, viewId: viewId, arguments: args, messenger: messenger)
  }
}

final class NativeSwitchPlatformView: NSObject, FlutterPlatformView {
  private let container: NativeSwitchContainer
  private let channel: FlutterMethodChannel
  private var value = false

  init(frame: CGRect, viewId: Int64, arguments: Any?, messenger: FlutterBinaryMessenger) {
    container = NativeSwitchContainer(frame: frame)
    channel = FlutterMethodChannel(name: "buzz/switch/\(viewId)", binaryMessenger: messenger)
    super.init()
    container.control.addTarget(self, action: #selector(changed), for: .valueChanged)
    update(arguments)
    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "update" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.update(call.arguments)
      result(nil)
    }
  }

  deinit { channel.setMethodCallHandler(nil) }
  func view() -> UIView { container }

  private func update(_ arguments: Any?) {
    guard let state = arguments as? [String: Any] else { return }
    value = state["value"] as? Bool ?? false
    container.control.setOn(value, animated: container.window != nil)
    container.control.isEnabled = state["enabled"] as? Bool ?? false
    container.control.accessibilityLabel = state["label"] as? String
    container.overrideUserInterfaceStyle = state["brightness"] as? String == "dark" ? .dark : .light
  }

  @objc private func changed() {
    guard container.control.isEnabled else { return }
    let requested = container.control.isOn
    // Flutter owns the persisted value. A rejected or failed change must not
    // leave UIKit displaying a value the app never accepted.
    container.control.setOn(value, animated: false)
    channel.invokeMethod("changed", arguments: requested)
  }
}

private final class NativeSwitchContainer: UIView {
  let control = UISwitch()
  override init(frame: CGRect) {
    super.init(frame: frame)
    backgroundColor = .clear
    isOpaque = false
    addSubview(control)
  }
  required init?(coder: NSCoder) { return nil }
  override func layoutSubviews() {
    super.layoutSubviews()
    control.sizeToFit()
    control.center = CGPoint(x: bounds.midX, y: bounds.midY)
  }
  // Keep the system switch's native accessibility and gestures, while giving
  // it the full 60 x 48 point Flutter hit target.
  override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
    guard !isHidden, alpha > 0, isUserInteractionEnabled,
      bounds.contains(point), control.isEnabled else { return nil }
    return control
  }
}
