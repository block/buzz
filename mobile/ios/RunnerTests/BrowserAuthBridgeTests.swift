import Flutter
import UIKit
import XCTest
@testable import Buzz

final class BrowserAuthBridgeTests: XCTestCase {
  @MainActor
  func testInviteHandoffWaitsForActivationAndCancelledAttemptCannotReopen() {
    let messenger = BrowserAuthTestMessenger()
    var active = false
    var starts = 0
    let bridge = BuzzBrowserAuthBridge(messenger: messenger, isActive: { active },
      startSession: { _ in starts += 1; return true })
    defer { withExtendedLifetime(bridge) {} }
    let arguments = ["url": "https://app.builderlabstaging.xyz/api/goose/v1/auth/login"]
    var results: [String] = []
    messenger.invoke("start", arguments: arguments) {
      results.append(($0 as? FlutterError)?.code ?? "success")
    }
    XCTAssertEqual(starts, 0, "An incoming invite must not present auth while inactive")
    XCTAssertTrue(results.isEmpty)
    messenger.invoke("cancel", arguments: [:]) { _ in }
    XCTAssertEqual(results, ["cancelled"])
    active = true
    NotificationCenter.default.post(name: UIApplication.didBecomeActiveNotification, object: nil)
    XCTAssertEqual(starts, 0, "Cancelled handoffs must not reopen sign-in")

    active = false
    messenger.invoke("start", arguments: arguments) {
      results.append(($0 as? FlutterError)?.code ?? "success")
    }
    XCTAssertEqual(starts, 0)
    active = true
    NotificationCenter.default.post(name: UIApplication.didBecomeActiveNotification, object: nil)
    XCTAssertEqual(starts, 1, "Activation must present the original request without a retry")
    NotificationCenter.default.post(name: UIApplication.didBecomeActiveNotification, object: nil)
    XCTAssertEqual(starts, 1)
    XCTAssertEqual(results, ["cancelled"])
    messenger.invoke("cancel", arguments: [:]) { _ in }
    XCTAssertEqual(results, ["cancelled", "cancelled"])
    messenger.invoke("start", arguments: arguments) { _ in }
    XCTAssertEqual(starts, 2, "Ordinary active-app login should start immediately")
    messenger.invoke("cancel", arguments: [:]) { _ in }
  }

  func testAllowsOnlyBuilderlabLoginPages() {
    for allowed in [
      "https://app.builderlab.xyz/auth/native?state=s",
      "https://app.builderlabstaging.xyz/auth/native",
      "https://block.builderlab.xyz/api/goose/v1/auth/login?type=cli",
      "https://block.builderlabstaging.xyz/api/goose/v1/auth/login",
    ] {
      XCTAssertTrue(BuzzBrowserAuthBridge.allowsLogin(URL(string: allowed)!), allowed)
    }
    for refused in [
      "http://block.builderlab.xyz/api/goose/v1/auth/login",
      "https://builderlab.xyz/auth/native",
      "https://evilbuilderlab.xyz/auth/native",
      "https://block.builderlab.xyz.example.com/auth/native",
      "https://block.builderlab.xyz/api/goose/v1/auth/me",
      "https://block.builderlab.xyz:8443/auth/native",
      "https://user@block.builderlab.xyz/auth/native",
      "https://block.builderlab.xyz/auth/native#fragment",
    ] {
      XCTAssertFalse(BuzzBrowserAuthBridge.allowsLogin(URL(string: refused)!), refused)
    }
  }
}

private final class BrowserAuthTestMessenger: NSObject, FlutterBinaryMessenger {
  private var handler: FlutterBinaryMessageHandler?
  func send(onChannel channel: String, message: Data?) {}
  func send(onChannel channel: String, message: Data?, binaryReply callback: FlutterBinaryReply?) {
    callback?(nil)
  }
  func setMessageHandlerOnChannel(_ channel: String,
    binaryMessageHandler handler: FlutterBinaryMessageHandler?) -> FlutterBinaryMessengerConnection {
    self.handler = handler
    return 1
  }
  func cleanUpConnection(_ connection: FlutterBinaryMessengerConnection) { handler = nil }
  func invoke(_ method: String, arguments: [String: String], reply: @escaping (Any?) -> Void) {
    let codec = FlutterStandardMethodCodec.sharedInstance()
    handler?(codec.encode(FlutterMethodCall(methodName: method, arguments: arguments))) {
      reply($0.flatMap { codec.decodeEnvelope($0) })
    }
  }
}
