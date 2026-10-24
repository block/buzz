import Flutter
import UIKit
import XCTest

@testable import Buzz

final class ProfileSetupRecoveryTests: XCTestCase {
  @MainActor
  func testStartedEditorWithoutReadyRestoresProfileAndAllowsRetry() async throws {
    let messenger = ProfileSetupTestMessenger()
    let parent = UIViewController()
    let window = UIWindow(frame: UIScreen.main.bounds)
    window.windowScene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
    window.rootViewController = parent
    window.makeKeyAndVisible()
    defer { window.isHidden = true }
    var watchdogs: [() -> Void] = []
    weak var engine: FlutterEngine?
    let coordinator = NativeProfileSetupCoordinator(messenger: messenger,
      scheduleEditorWatchdog: { watchdogs.append($0) },
      startEditorEngine: {
        engine = $0
        // Start the normal app entry point: it never sends avatar-editor ready.
        return $0.run()
      }, configureEditorEngine: { _ in })
    messenger.invoke("present", arguments: ["session": 81])
    let nav = try XCTUnwrap(parent.presentedViewController as? UINavigationController)
    if let transition = nav.transitionCoordinator {
      await withCheckedContinuation { continuation in
        transition.animate(alongsideTransition: nil) { _ in continuation.resume() }
      }
    }
    let profile = try XCTUnwrap(nav.viewControllers.first)
    let avatar = try XCTUnwrap(descendants(profile.view).compactMap { $0 as? UIButton }
      .first { $0.accessibilityLabel == "Add profile photo" })
    let pan = UIPanGestureRecognizer()
    let disabledPan = UIPanGestureRecognizer()
    disabledPan.isEnabled = false
    nav.view.addGestureRecognizer(pan)
    nav.view.addGestureRecognizer(disabledPan)
    nav.interactivePopGestureRecognizer?.isEnabled = true
    for detent in [UISheetPresentationController.Detent.Identifier("profile"), .large] {
      autoreleasepool {
        nav.sheetPresentationController?.selectedDetentIdentifier = detent
        avatar.sendActions(for: .touchUpInside)
        XCTAssertNotNil(engine)
        XCTAssertTrue(nav.topViewController is FlutterViewController)
        XCTAssertFalse(pan.isEnabled)
        XCTAssertFalse(nav.interactivePopGestureRecognizer?.isEnabled ?? true)
        XCTAssertTrue(nav.isModalInPresentation)
        XCTAssertTrue(nav.isNavigationBarHidden)
        if watchdogs.count > 1 {
          // An old attempt's watchdog cannot tear down the successful retry.
          watchdogs[0]()
          XCTAssertTrue(nav.topViewController is FlutterViewController)
          XCTAssertNotNil(engine)
        }
        watchdogs.last?()
        XCTAssertTrue(nav.topViewController === profile)
        XCTAssertFalse(nav.isNavigationBarHidden)
        XCTAssertEqual(nav.sheetPresentationController?.detents.map(\.identifier), [.init("profile"), .large])
        XCTAssertEqual(nav.sheetPresentationController?.selectedDetentIdentifier, detent)
        XCTAssertTrue(pan.isEnabled)
        XCTAssertFalse(disabledPan.isEnabled)
        XCTAssertTrue(nav.interactivePopGestureRecognizer?.isEnabled ?? false)
        XCTAssertFalse(nav.isModalInPresentation)
        XCTAssertTrue(avatar.isEnabled)
        XCTAssertFalse(avatar.isHidden)
      }
      // UIKit releases popped controllers at the end of its autorelease turn.
      let released = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in engine == nil }, object: nil)
      await fulfillment(of: [released], timeout: 2)
    }
    XCTAssertEqual(watchdogs.count, 2)
    withExtendedLifetime(coordinator) {}
    nav.dismiss(animated: false)
  }

  @MainActor
  private func descendants(_ view: UIView) -> [UIView] {
    view.subviews.flatMap { [$0] + descendants($0) }
  }
}

private final class ProfileSetupTestMessenger: NSObject, FlutterBinaryMessenger {
  private var handler: FlutterBinaryMessageHandler?
  func send(onChannel channel: String, message: Data?) {}
  func send(onChannel channel: String, message: Data?, binaryReply callback: FlutterBinaryReply?) { callback?(nil) }
  func setMessageHandlerOnChannel(_ channel: String, binaryMessageHandler handler: FlutterBinaryMessageHandler?) -> FlutterBinaryMessengerConnection {
    self.handler = handler
    return 1
  }
  func cleanUpConnection(_ connection: FlutterBinaryMessengerConnection) { handler = nil }
  func invoke(_ method: String, arguments: [String: Any]) {
    let codec = FlutterStandardMethodCodec.sharedInstance()
    handler?(codec.encode(FlutterMethodCall(methodName: method, arguments: arguments))) { _ in }
  }
}
