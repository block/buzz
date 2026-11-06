import Flutter
import UIKit
import XCTest
@testable import Buzz

final class NativeSwitchTests: XCTestCase {
  @MainActor
  func testSystemSwitchKeepsLabelStateAndDisabledBehavior() throws {
    let messenger = SwitchTestMessenger()
    var platform: NativeSwitchPlatformView? = NativeSwitchPlatformView(
      frame: CGRect(x: 0, y: 0, width: 60, height: 48), viewId: 42,
      arguments: ["label": "Hold to talk to Bestie", "value": true, "enabled": true],
      messenger: messenger
    )
    let container = try XCTUnwrap(platform?.view())
    container.layoutIfNeeded()
    let control = try XCTUnwrap(container.subviews.first as? UISwitch)
    XCTAssertTrue(control.isOn)
    XCTAssertEqual(control.accessibilityLabel, "Hold to talk to Bestie")
    XCTAssertTrue(container.hitTest(CGPoint(x: 2, y: 2), with: nil) === control)
    control.setOn(false, animated: false)
    control.sendActions(for: .valueChanged)
    XCTAssertEqual(messenger.values, [false])
    XCTAssertTrue(control.isOn)
    messenger.update(["label": "Notifications", "value": true, "enabled": false, "brightness": "dark"])
    XCTAssertEqual(control.accessibilityLabel, "Notifications")
    XCTAssertTrue(control.isOn)
    XCTAssertFalse(control.isEnabled)
    XCTAssertEqual(container.overrideUserInterfaceStyle, .dark)
    XCTAssertNil(container.hitTest(CGPoint(x: 2, y: 2), with: nil))
    control.sendActions(for: .valueChanged)
    XCTAssertEqual(messenger.values, [false])
    platform = nil
    XCTAssertNil(messenger.handler)
  }
}

private final class SwitchTestMessenger: NSObject, FlutterBinaryMessenger {
  var values: [Bool] = []
  var handler: FlutterBinaryMessageHandler?
  func send(onChannel channel: String, message: Data?) {
    guard let message else { return }
    let call = FlutterStandardMethodCodec.sharedInstance().decodeMethodCall(message)
    if call.method == "changed", let value = call.arguments as? Bool { values.append(value) }
  }
  func send(onChannel channel: String, message: Data?, binaryReply callback: FlutterBinaryReply?) {
    send(onChannel: channel, message: message)
    callback?(nil)
  }
  func setMessageHandlerOnChannel(_ channel: String, binaryMessageHandler handler: FlutterBinaryMessageHandler?) -> FlutterBinaryMessengerConnection {
    self.handler = handler
    return 1
  }
  func cleanUpConnection(_ connection: FlutterBinaryMessengerConnection) { handler = nil }
  func update(_ state: [String: Any]) {
    handler?(FlutterStandardMethodCodec.sharedInstance().encode(FlutterMethodCall(methodName: "update", arguments: state))) { _ in }
  }
}
