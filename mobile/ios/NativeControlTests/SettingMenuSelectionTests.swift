import Flutter
import UIKit

private final class SettingMenuMessenger: NSObject, FlutterBinaryMessenger {
  var selections: [Int] = []
  func send(onChannel channel: String, message: Data?) {
    guard let message else { return }
    let call = FlutterStandardMethodCodec.sharedInstance().decodeMethodCall(message)
    if call.method == "menuAction", let index = call.arguments as? Int {
      selections.append(index)
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

@main
struct SettingMenuSelectionTests {
  @MainActor static func main() {
    for deferred in [false, true] {
      let messenger = SettingMenuMessenger()
      let view = NavigationGlassButtonPlatformView(
        frame: CGRect(x: 0, y: 0, width: 340, height: 56),
        viewIdentifier: 1,
        arguments: [
          "plain": true, "label": "", "enabled": true,
          "deferMenuActions": deferred,
          "menu": [
            ["id": 0, "label": "Low", "enabled": true, "checked": false],
            ["id": 1, "label": "High", "enabled": true, "checked": true],
          ],
        ],
        messenger: messenger
      )
      let button = view.view().subviews.compactMap { $0 as? NavigationGlassButton }.first!
      let group = button.menu!.children.first as! UIMenu
      let choices = group.children.compactMap { $0 as? UIAction }
      precondition(choices[0].state == .off && choices[1].state == .on)
      // Invoke the actual menu UIAction through UIKit, without synthesizing a
      // Flutter method-channel message or depending on dismissal callback order.
      let selection = UIButton(primaryAction: choices[0])
      selection.sendActions(for: .touchUpInside)
      precondition(messenger.selections == (deferred ? [] : [0]))
      if deferred {
        precondition(button.pendingMenuAction != nil)
        button.pendingMenuAction?()
        button.pendingMenuAction = nil
        precondition(messenger.selections == [0])
      }
    }
    print("Native setting menu selection checks passed")
  }
}
