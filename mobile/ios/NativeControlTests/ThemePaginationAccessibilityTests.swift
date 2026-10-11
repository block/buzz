import Flutter
import UIKit

@main
struct ThemePaginationAccessibilityTests {
  static func check(_ condition: Bool, _ message: String) {
    precondition(condition, message)
  }

  @MainActor static func main() {
    let messenger = PaginationTestMessenger()
    let platform = ThemePaginationGlassControlPlatformView(
      frame: CGRect(x: 0, y: 0, width: 116, height: 54),
      viewIdentifier: 42,
      arguments: ["accessibilityLabel": "Photo", "count": 3, "selected": 1],
      messenger: messenger
    )
    let control = platform.view()
    control.layoutIfNeeded()
    func accessibleViews(_ view: UIView) -> [UIView] {
      (view.isAccessibilityElement ? [view] : []) + view.subviews.flatMap(accessibleViews)
    }
    check(accessibleViews(control).count == 1, "Exactly one native accessibility element")
    check(control.accessibilityTraits.contains(.adjustable), "Paginator must be adjustable")
    check(control.accessibilityLabel == "Photo", "Native label must match content")
    check(control.accessibilityValue == "2 of 3", "Initial page value")
    control.accessibilityIncrement()
    check(control.accessibilityValue == "3 of 3", "Increment updates value")
    check(messenger.selections == [2], "Increment is delivered to Flutter once")
    control.accessibilityIncrement()
    check(messenger.selections == [2], "Increment stops at last page")
    control.accessibilityDecrement()
    control.accessibilityDecrement()
    control.accessibilityDecrement()
    check(control.accessibilityValue == "1 of 3", "Decrement stops at first page")
    check(messenger.selections == [2, 1, 0], "Each decrement is delivered once")
    check(accessibleViews(control).count == 1, "Selection keeps a single owner")
    print("Native pagination accessibility passed")
  }
}

private final class PaginationTestMessenger: NSObject, FlutterBinaryMessenger {
  var selections: [Int] = []
  func send(onChannel channel: String, message: Data?) {
    guard let message else { return }
    let call = FlutterStandardMethodCodec.sharedInstance().decodeMethodCall(message)
    if call.method == "selected", let selected = call.arguments as? Int {
      selections.append(selected)
    }
  }
  func send(onChannel channel: String, message: Data?, binaryReply callback: FlutterBinaryReply?) {
    send(onChannel: channel, message: message)
    callback?(nil)
  }
  func setMessageHandlerOnChannel(
    _ channel: String, binaryMessageHandler handler: FlutterBinaryMessageHandler?
  ) -> FlutterBinaryMessengerConnection { 1 }
  func cleanUpConnection(_ connection: FlutterBinaryMessengerConnection) {}
}
