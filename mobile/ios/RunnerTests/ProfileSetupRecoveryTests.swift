import Flutter
import UIKit
import XCTest

@testable import Buzz

final class ProfileSetupRecoveryTests: XCTestCase {
  @MainActor
  func testNativeEditorsPreferRightPaneOnlyWithDuoRail() async throws {
    for duo in [false, true] {
      for method in ["presentStatus", "present"] {
        let messenger = ProfileSetupTestMessenger()
        let parent = SheetPlacementPresenter()
        parent.view = SheetPlacementView(duo: duo)
        let coordinator = NativeProfileTextEditorCoordinator(
          messenger: messenger, parentViewController: parent)
        defer { withExtendedLifetime(coordinator) {} }
        messenger.invoke(method, arguments: [
          "title": "Profile description", "initialValue": "", "placeholder": "",
          "multiline": true, "brightness": "light",
          "pageBackgroundArgb": 0xFFFFFFFF, "containerBackgroundArgb": 0xFFF0F0F0,
          "containerCornerRadius": 16,
        ])
        await withCheckedContinuation { continuation in
          DispatchQueue.main.async { continuation.resume() }
        }
        let sheet = try XCTUnwrap(parent.captured?.sheetPresentationController)
        #if canImport(UIKit, _version: 9127.0.85)
        if #available(iOS 27.1, *) {
          XCTAssertEqual(sheet.preferredPlacement, duo ? .trailing : .automatic)
        }
        #endif
        XCTAssertEqual(sheet.detents.map(\.identifier), [.large])
        XCTAssertNil(sheet.sourceView)
      }
    }
  }

  @MainActor
  func testStatusFormPresetsCustomExpirationClearAndRailActions() throws {
    var values: [[String: Any]] = []
    var cancelled = false
    let expiration = Date().addingTimeInterval(172800)
    let editor = NativeStatusEditorViewController(
      configuration: ["text": "", "emoji": "", "duration": 0,
                      "expiresAt": expiration.timeIntervalSince1970,
                      "hasExistingStatus": true],
      onChooseEmoji: { $0(":buzz:") },
      onCancel: { cancelled = true }, onSet: { values.append($0) })
    let navigation = UINavigationController(rootViewController: editor)
    navigation.loadViewIfNeeded()
    editor.loadViewIfNeeded()
    for style in [UIUserInterfaceStyle.light, .dark] {
      let traits = UITraitCollection(userInterfaceStyle: style)
      XCTAssertEqual(editor.view.tintColor.resolvedColor(with: traits), UIColor.label.resolvedColor(with: traits))
      XCTAssertEqual(navigation.navigationBar.tintColor.resolvedColor(with: traits), UIColor.label.resolvedColor(with: traits))
      XCTAssertEqual(editor.datePicker.tintColor.resolvedColor(with: traits), UIColor.label.resolvedColor(with: traits))
    }
    let cancel = try XCTUnwrap(editor.navigationItem.leftBarButtonItem)
    let submit = try XCTUnwrap(editor.navigationItem.rightBarButtonItem)
    XCTAssertEqual(cancel.accessibilityLabel, "Close")
    XCTAssertFalse(submit.isEnabled)
    #if canImport(UIKit, _version: 9127.0.85)
    if #available(iOS 27.1, *) {
      XCTAssertEqual(cancel.axisBehavior, .verticalPreferred)
      XCTAssertEqual(submit.axisBehavior, .verticalPreferred)
    }
    #endif
    XCTAssertEqual(editor.tableView(editor.tableView, numberOfRowsInSection: 1), 2)
    editor.tableView(editor.tableView, didSelectRowAt: IndexPath(row: 0, section: 2))
    XCTAssertTrue(submit.isEnabled)
    XCTAssertEqual(editor.statusField.text, "In a meeting")
    let emojiCell = editor.tableView(editor.tableView, cellForRowAt: IndexPath(row: 1, section: 0))
    let chooseEmoji = try XCTUnwrap(descendants(emojiCell).compactMap { $0 as? UIButton }.first { $0.title(for: .normal) == "Choose emoji" })
    chooseEmoji.sendActions(for: .touchUpInside)
    XCTAssertEqual(editor.emojiField.text, ":buzz:")
    UIApplication.shared.sendAction(try XCTUnwrap(submit.action), to: submit.target, from: submit, for: nil)
    XCTAssertEqual(values.last?["text"] as? String, "In a meeting")
    XCTAssertEqual(values.last?["emoji"] as? String, ":buzz:")
    XCTAssertEqual(try XCTUnwrap(values.last?["expiresAt"] as? Double), expiration.timeIntervalSince1970, accuracy: 1)
    editor.tableView(editor.tableView, didSelectRowAt: IndexPath(row: 0, section: 3))
    XCTAssertEqual(values.last?["clear"] as? Bool, true)
    UIApplication.shared.sendAction(try XCTUnwrap(cancel.action), to: cancel.target, from: cancel, for: nil)
    XCTAssertTrue(cancelled)
  }

  @MainActor
  func testNativeProfileFieldsRetainCancelAndPreferVerticalControls() throws {
    for multiline in [false, true] {
      var cancelled = false
      let editor = NativeProfileTextEditorViewController(
        title: multiline ? "Profile description" : "Display name",
        initialValue: "Original", placeholder: "Text", multiline: multiline,
        pageBackgroundColor: .systemBackground, containerBackgroundColor: .secondarySystemBackground,
        containerCornerRadius: 16, allowUnchangedSubmission: false,
        channelDescription: nil, originalName: nil, originalDescription: nil,
        canEditDetails: true, canEditCanvas: false, canvasContent: "", canvasLoaded: false,
        chatOptions: nil, onCancel: { cancelled = true }, onSet: { _ in }
      )
      editor.loadViewIfNeeded()
      let cancel = try XCTUnwrap(editor.navigationItem.leftBarButtonItem)
      let submit = try XCTUnwrap(editor.navigationItem.rightBarButtonItem)
      #if canImport(UIKit, _version: 9127.0.85)
      if #available(iOS 27.1, *) {
        XCTAssertEqual(cancel.axisBehavior, .verticalPreferred)
        XCTAssertEqual(submit.axisBehavior, .verticalPreferred)
      }
      #endif
      XCTAssertTrue(cancel.isEnabled)
      XCTAssertFalse(submit.isEnabled)
      UIApplication.shared.sendAction(try XCTUnwrap(cancel.action), to: cancel.target, from: cancel, for: nil)
      XCTAssertTrue(cancelled)
    }
  }

  @MainActor
  func testSelectedEditorIconsContrastInBothAppearances() throws {
    for dark in [false, true] {
      for icon in ["emoji", "palette"] {
        let control = NavigationGlassButtonPlatformView(
          frame: CGRect(x: 0, y: 0, width: 64, height: 64), viewIdentifier: 911,
          arguments: ["icon": icon, "controlSize": 64, "selected": true,
            "brightness": dark ? "dark" : "light", "foregroundColor": dark ? 0xFFFFFFFF : 0xFF000000],
          messenger: ProfileSetupTestMessenger())
        let button = try XCTUnwrap(descendants(control.view()).compactMap { $0 as? UIButton }.first)
        let image = try XCTUnwrap(button.configuration?.image)
        XCTAssertEqual(image.renderingMode, .alwaysOriginal)
        let rendered = UIGraphicsImageRenderer(size: image.size).image { _ in
          image.draw(at: .zero)
        }
        let cgImage = try XCTUnwrap(rendered.cgImage)
        let width = cgImage.width, height = cgImage.height
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        try pixels.withUnsafeMutableBytes { bytes in
          let context = try XCTUnwrap(CGContext(data: bytes.baseAddress, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
          context.draw(cgImage, in: CGRect(x: 0, y: 0, width: CGFloat(width), height: CGFloat(height)))
        }
        let opaque = stride(from: 0, to: pixels.count, by: 4).filter { pixels[$0 + 3] > 250 }
        XCTAssertFalse(opaque.isEmpty)
        XCTAssertTrue(opaque.allSatisfy { pixels[$0] == (dark ? 0 : pixels[$0 + 3]) })
        let attachment = XCTAttachment(image: rendered)
        attachment.name = "Selected \(icon) dark=\(dark)"
        attachment.lifetime = .keepAlways
        add(attachment)
      }
    }
  }

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
    XCTAssertEqual(avatar.transform.a, 0.85, accuracy: 0.001)
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
        let editor = profile.children.compactMap { $0 as? FlutterViewController }.first
        XCTAssertEqual(editor?.isViewOpaque, false, "Native chrome must remain visible through the exiting editor")
        XCTAssertEqual(editor?.view.backgroundColor, UIColor.clear)
        for dark in [false, true] {
          messenger.invoke("appearance", arguments: ["session": 81, "dark": dark,
            "pageBackgroundArgb": dark ? 0xFF000000 : 0xFFFFFFFF])
          XCTAssertEqual(editor?.isViewOpaque, false)
          XCTAssertEqual(editor?.view.backgroundColor, UIColor.clear)
          XCTAssertEqual(nav.overrideUserInterfaceStyle, dark ? .dark : .light)
        }
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


private final class SheetPlacementView: UIView {
  let duo: Bool
  init(duo: Bool) {
    self.duo = duo
    super.init(frame: CGRect(x: 0, y: 0, width: 932, height: 656))
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
  override var safeAreaInsets: UIEdgeInsets {
    duo ? UIEdgeInsets(top: 0, left: 0, bottom: 0, right: 84)
        : UIEdgeInsets(top: 62, left: 0, bottom: 34, right: 0)
  }
}

private final class SheetPlacementPresenter: UIViewController {
  var captured: UIViewController?
  override func present(_ viewControllerToPresent: UIViewController,
                        animated: Bool, completion: (() -> Void)? = nil) {
    captured = viewControllerToPresent
    completion?()
  }
}
