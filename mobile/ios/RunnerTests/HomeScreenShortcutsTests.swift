import Flutter
import UIKit
import XCTest
@testable import Buzz

final class HomeScreenShortcutsTests: XCTestCase {
  private var savedItems: [UIApplicationShortcutItem]?

  @MainActor
  override func setUp() {
    super.setUp()
    savedItems = UIApplication.shared.shortcutItems
  }

  @MainActor
  override func tearDown() {
    UIApplication.shared.shortcutItems = savedItems
    super.tearDown()
  }

  @MainActor
  private func publish(_ types: [String]) {
    UIApplication.shared.shortcutItems = types.map {
      UIApplicationShortcutItem(type: $0, localizedTitle: $0)
    }
  }

  @MainActor
  private func open(_ link: String) -> Bool {
    HomeScreenShortcuts().open(URL(string: link)!)
  }

  @MainActor
  func testShortcutLinksAreIgnoredWhileNoQuickActionsArePublished() {
    // Signed out (or before Home first publishes), the app icon has no quick
    // actions. A link then must not queue an action for the next account.
    publish([])
    XCTAssertFalse(open("buzz://shortcut/new-chat"))
    XCTAssertFalse(open("buzz://shortcut/message-bestie"))
  }

  @MainActor
  func testShortcutLinksRunOnlyPublishedQuickActions() {
    publish(["new-chat"])
    XCTAssertTrue(open("buzz://shortcut/new-chat"))
    XCTAssertFalse(open("buzz://shortcut/message-bestie"))
  }

  @MainActor
  func testOtherLinksAreNotShortcuts() {
    publish(["new-chat", "new-dm", "message-bestie"])
    XCTAssertFalse(open("buzz://message?channel=c&id=e"))
  }

  @MainActor
  func testDiscardDropsTheMenuAndATapWaitingForHome() {
    // A menu left from an earlier session can buffer a tap before auth
    // resolves signed out; discarding must drop both.
    publish(["new-chat"])
    let bridge = HomeScreenShortcuts()
    let messenger = ShortcutsTestMessenger()
    bridge.attach(messenger: messenger)
    XCTAssertTrue(bridge.open(URL(string: "buzz://shortcut/new-chat")!))
    messenger.invoke("discard") { _ in }
    XCTAssertEqual(UIApplication.shared.shortcutItems?.count ?? 0, 0)
    var pending: Any? = "unset"
    messenger.invoke("takePending") { pending = $0 }
    XCTAssertNil(pending)
  }
}

private final class ShortcutsTestMessenger: NSObject, FlutterBinaryMessenger {
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
  func invoke(_ method: String, reply: @escaping (Any?) -> Void) {
    let codec = FlutterStandardMethodCodec.sharedInstance()
    handler?(codec.encode(FlutterMethodCall(methodName: method, arguments: nil))) {
      reply($0.flatMap { codec.decodeEnvelope($0) })
    }
  }
}
