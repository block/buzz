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
    let arguments = ["url": "https://app.builderlabstaging.xyz/auth/native"]
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

  @MainActor
  func testConfiguredAccountOriginReachesNativeBrowserSession() {
    let messenger = BrowserAuthTestMessenger()
    var starts = 0
    let bridge = BuzzBrowserAuthBridge(messenger: messenger, isActive: { true },
      startSession: { _ in starts += 1; return true })
    defer { withExtendedLifetime(bridge) {} }
    let origin = "https://accounts.example.com"
    for refused in [
      "https://accounts.example.com.attacker.test/auth/native",
      "https://child.accounts.example.com/auth/native",
      "https://accounts.example.com/other",
      "http://accounts.example.com/auth/native",
      "https://user@accounts.example.com/auth/native",
      "https://accounts.example.com:8443/auth/native",
      "https://accounts.example.com/auth/native#fragment",
    ] {
      messenger.invoke("start", arguments: ["url": refused,
        "configuredAccountOrigin": origin]) {
        XCTAssertEqual(($0 as? FlutterError)?.code, "unavailable", refused)
      }
    }
    XCTAssertEqual(starts, 0)
    messenger.invoke("start", arguments: ["url": origin + "/auth/native?state=test",
      "configuredAccountOrigin": origin]) { _ in }
    XCTAssertEqual(starts, 1)
    messenger.invoke("cancel", arguments: [:]) { _ in }
    for malformedOrigin in ["http://accounts.example.com", origin + "/path",
      origin + "?query=1", origin + "#fragment", "https://user@accounts.example.com"] {
      XCTAssertFalse(BuzzBrowserAuthBridge.allowsLogin(URL(string: origin + "/auth/native")!,
        configuredAccountOrigin: malformedOrigin))
    }
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
      "https://evilbuilderlab.xyz/api/goose/v1/auth/login",
      "https://block.builderlab.xyz.example.com/api/goose/v1/auth/login",
      "https://block.builderlab.xyz/api/goose/v1/auth/me",
      "https://app.builderlab.xyz/api/goose/v1/auth/login",
      "https://block.builderlab.xyz/auth/native",
      "https://a.block.builderlab.xyz/api/goose/v1/auth/login",
      "https://app.builderlab.xyz:8443/auth/native",
      "https://user@app.builderlab.xyz/auth/native",
      "https://app.builderlab.xyz/auth/native#fragment",
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
