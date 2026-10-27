import Flutter
import UIKit
import XCTest

@testable import Buzz

final class ProfileSetupRecoveryTests: XCTestCase {
  @MainActor
  func testCameraShutterHasAnOpaqueCenter() throws {
    let control = NavigationGlassButtonPlatformView(
      frame: CGRect(x: 0, y: 0, width: 100, height: 100), viewIdentifier: 910,
      arguments: ["icon": "shutter", "controlSize": 100, "accessibilityLabel": "Take photo"],
      messenger: ProfileSetupTestMessenger())
    let button = try XCTUnwrap(descendants(control.view()).compactMap { $0 as? UIButton }.first)
    let image = try XCTUnwrap(button.configuration?.image?.cgImage)
    var pixel = [UInt8](repeating: 0, count: 4)
    let context = try XCTUnwrap(CGContext(data: &pixel, width: 1, height: 1, bitsPerComponent: 8,
      bytesPerRow: 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    context.translateBy(x: -CGFloat(image.width / 2), y: -CGFloat(image.height / 2))
    context.draw(image, in: CGRect(x: 0, y: 0, width: CGFloat(image.width), height: CGFloat(image.height)))
    XCTAssertEqual(pixel[3], 255, "The shutter must be filled, not an outlined circle")
    let attachment = XCTAttachment(image: UIImage(cgImage: image))
    attachment.lifetime = .keepAlways
    add(attachment)
  }

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
    messenger.invoke("present", arguments: ["session": 81, "dark": true, "primaryArgb": 0xFFFFFFFF, "onPrimaryArgb": 0xFF000000])
    let nav = try XCTUnwrap(parent.presentedViewController as? UINavigationController)
    if let transition = nav.transitionCoordinator {
      await withCheckedContinuation { continuation in
        transition.animate(alongsideTransition: nil) { _ in continuation.resume() }
      }
    }
    let profile = try XCTUnwrap(nav.viewControllers.first)
    let avatar = try XCTUnwrap(descendants(profile.view).compactMap { $0 as? UIButton }
      .first { $0.accessibilityLabel == "Add profile photo" })
    XCTAssertEqual(nav.modalPresentationStyle, .overFullScreen)
    XCTAssertNil(profile.navigationItem.rightBarButtonItem)
    let name = try XCTUnwrap(descendants(profile.view).compactMap { $0 as? UITextField }.first)
    let submit = try XCTUnwrap(descendants(profile.view).compactMap { $0 as? UIButton }
      .first { $0.configuration?.title == "Continue" })
    let skip = try XCTUnwrap(descendants(profile.view).compactMap { $0 as? UIButton }
      .first { $0.configuration?.title == "Skip" })
    let foreground = try XCTUnwrap(submit.configuration?.baseForegroundColor)
    var red: CGFloat = 1, green: CGFloat = 1, blue: CGFloat = 1, alpha: CGFloat = 0
    XCTAssertTrue(foreground.getRed(&red, green: &green, blue: &blue, alpha: &alpha))
    XCTAssertEqual(red, 0); XCTAssertEqual(green, 0); XCTAssertEqual(blue, 0); XCTAssertEqual(alpha, 1)
    XCTAssertTrue(skip.isHidden)
    XCTAssertTrue(avatar.superview!.isHidden)
    XCTAssertFalse(submit.isEnabled)
    name.text = "Kenny"
    name.sendActions(for: .editingChanged)
    submit.sendActions(for: .touchUpInside)
    XCTAssertNotNil(engine, "Prepare the editor before the avatar is tapped")
    XCTAssertNil(profile.navigationItem.rightBarButtonItem)
    XCTAssertEqual(submit.alpha, 1)
    XCTAssertTrue(name.isHidden)
    XCTAssertFalse(skip.isHidden)
    XCTAssertFalse(submit.isEnabled, "Continue requires an avatar saved from the editor")
    XCTAssertFalse(avatar.superview!.isHidden)
    profile.view.layoutIfNeeded()
    XCTAssertEqual(avatar.bounds.width, 220, accuracy: 0.5)
    XCTAssertTrue(descendants(profile.view).compactMap { $0 as? UILabel }
      .contains { $0.text == "Choose your avatar" && $0.accessibilityTraits.contains(.header) })
    XCTAssertTrue(avatar.point(inside: CGPoint(x: 4, y: 110), with: nil))
    XCTAssertTrue(avatar.point(inside: CGPoint(x: 110, y: 110), with: nil))
    let pan = UIPanGestureRecognizer()
    let disabledPan = UIPanGestureRecognizer()
    disabledPan.isEnabled = false
    nav.view.addGestureRecognizer(pan)
    nav.view.addGestureRecognizer(disabledPan)
    nav.interactivePopGestureRecognizer?.isEnabled = true
    for _ in 0..<2 {
      autoreleasepool {
        avatar.sendActions(for: .touchUpInside)
        XCTAssertNotNil(engine)
        XCTAssertEqual(profile.navigationItem.rightBarButtonItem?.title, "Done")
        XCTAssertFalse(profile.navigationItem.rightBarButtonItem?.isEnabled ?? true)
        XCTAssertTrue(profile.navigationItem.leftBarButtonItem?.isEnabled ?? false)
        XCTAssertTrue(profile.children.contains { $0 is FlutterViewController })
        XCTAssertFalse(pan.isEnabled)
        XCTAssertFalse(nav.interactivePopGestureRecognizer?.isEnabled ?? true)
        XCTAssertTrue(nav.isModalInPresentation)
        XCTAssertFalse(nav.isNavigationBarHidden)
        if watchdogs.count > 1 {
          // An old attempt's watchdog cannot tear down the successful retry.
          watchdogs[0]()
          XCTAssertTrue(profile.children.contains { $0 is FlutterViewController })
          XCTAssertNotNil(engine)
        }
        watchdogs.last?()
        XCTAssertTrue(nav.topViewController === profile)
        XCTAssertFalse(nav.isNavigationBarHidden)
        XCTAssertEqual(nav.modalPresentationStyle, .overFullScreen)
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
    skip.sendActions(for: .touchUpInside)
    XCTAssertEqual(messenger.savedArguments?["name"] as? String, "Kenny")
    XCTAssertEqual(messenger.savedArguments?["skipAvatar"] as? Bool, true)
    XCTAssertTrue(skip.isEnabled, "Failed name-only save must remain retryable through Skip")
    XCTAssertFalse(submit.isEnabled)
    let notNow = try XCTUnwrap(descendants(profile.view).compactMap { $0 as? UIButton }
      .first { $0.configuration?.title == "Not now" })
    XCTAssertFalse(notNow.isHidden)
    XCTAssertTrue(notNow.isEnabled)
    notNow.sendActions(for: .touchUpInside)
    let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in parent.presentedViewController == nil }, object: nil)
    await fulfillment(of: [dismissed], timeout: 2)
    XCTAssertEqual(messenger.completionAction, "skip")
    withExtendedLifetime(coordinator) {}
    nav.dismiss(animated: false)
  }

  @MainActor
  func testSelectedEmojiShowsEditablePreview() async throws {
    let messenger = ProfileSetupTestMessenger()
    let parent = UIViewController()
    let window = UIWindow(frame: UIScreen.main.bounds)
    window.windowScene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
    window.rootViewController = parent
    window.makeKeyAndVisible()
    defer { window.isHidden = true }
    let coordinator = NativeProfileSetupCoordinator(messenger: messenger, startEditorEngine: { _ in false }, configureEditorEngine: { _ in })
    messenger.invoke("present", arguments: ["session": 82, "name": "Kenny", "emoji": "😊", "emojiColor": 0xFFFFCC00, "primaryArgb": 0xFF000000, "onPrimaryArgb": 0xFFFFFFFF])
    let nav = try XCTUnwrap(parent.presentedViewController as? UINavigationController)
    if let transition = nav.transitionCoordinator {
      await withCheckedContinuation { continuation in
        transition.animate(alongsideTransition: nil) { _ in continuation.resume() }
      }
    }
    let profile = try XCTUnwrap(nav.viewControllers.first)
    let avatar = try XCTUnwrap(descendants(profile.view).compactMap { $0 as? UIButton }
      .first { $0.accessibilityLabel == "Edit profile photo" })
    let submit = try XCTUnwrap(descendants(profile.view).compactMap { $0 as? UIButton }
      .first { $0.configuration?.title == "Continue" })
    submit.sendActions(for: .touchUpInside)
    XCTAssertTrue(submit.isEnabled)
    for dark in [true, false] {
      let background: UInt32 = dark ? 0xFF000000 : 0xFFFFFFFF
      messenger.invoke("appearance", arguments: ["session": 82, "dark": dark, "pageBackgroundArgb": background,
        "primaryArgb": dark ? 0xFFFFFFFF : 0xFF000000, "onPrimaryArgb": background])
      XCTAssertTrue(parent.presentedViewController === nav)
      XCTAssertEqual(nav.overrideUserInterfaceStyle, dark ? .dark : .light)
      XCTAssertTrue(submit.isEnabled)
      XCTAssertEqual(avatar.accessibilityLabel, "Edit profile photo")
      let expected = dark ? UIColor.black : UIColor.white
      var actualWhite: CGFloat = -1, alpha: CGFloat = 0
      XCTAssertTrue(profile.view.backgroundColor!.getWhite(&actualWhite, alpha: &alpha))
      var expectedWhite: CGFloat = -1
      expected.getWhite(&expectedWhite, alpha: &alpha)
      XCTAssertEqual(actualWhite, expectedWhite, accuracy: 0.001)
    }
    let image = try XCTUnwrap(avatar.image(for: .normal))
    XCTAssertEqual(image.size, CGSize(width: 220, height: 220))
    let attachment = XCTAttachment(image: image)
    attachment.name = "Selected emoji with edit badge"
    attachment.lifetime = .keepAlways
    add(attachment)
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
  var savedArguments: [String: Any]?
  var completionAction: String?
  func send(onChannel channel: String, message: Data?) {}
  func send(onChannel channel: String, message: Data?, binaryReply callback: FlutterBinaryReply?) {
    let codec = FlutterStandardMethodCodec.sharedInstance()
    if let message {
      let call = codec.decodeMethodCall(message)
      if call.method == "save" { savedArguments = call.arguments as? [String: Any] }
    }
    callback?(codec.encodeSuccessEnvelope(["saved": false]))
  }
  func setMessageHandlerOnChannel(_ channel: String, binaryMessageHandler handler: FlutterBinaryMessageHandler?) -> FlutterBinaryMessengerConnection {
    self.handler = handler
    return 1
  }
  func cleanUpConnection(_ connection: FlutterBinaryMessengerConnection) { handler = nil }
  func invoke(_ method: String, arguments: [String: Any]) {
    let codec = FlutterStandardMethodCodec.sharedInstance()
    handler?(codec.encode(FlutterMethodCall(methodName: method, arguments: arguments))) { data in
      if let data { self.completionAction = try? codec.decodeEnvelope(data) as? String }
    }
  }
}
