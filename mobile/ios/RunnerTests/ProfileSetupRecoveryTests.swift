import Flutter
import UIKit
import XCTest

@testable import Buzz

final class ProfileSetupRecoveryTests: XCTestCase {
  @MainActor
  func testNativeEditorsPreferRightPaneOnlyWithDuoRail() async throws {
    guard #available(iOS 27.1, *) else { return }
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
        XCTAssertEqual(sheet.preferredPlacement, duo ? .trailing : .automatic)
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
    if #available(iOS 27.1, *) {
      XCTAssertEqual(cancel.axisBehavior, .verticalPreferred)
      XCTAssertEqual(submit.axisBehavior, .verticalPreferred)
    }
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
      if #available(iOS 27.1, *) {
        XCTAssertEqual(cancel.axisBehavior, .verticalPreferred)
        XCTAssertEqual(submit.axisBehavior, .verticalPreferred)
      }
      XCTAssertTrue(cancel.isEnabled)
      XCTAssertFalse(submit.isEnabled)
      UIApplication.shared.sendAction(try XCTUnwrap(cancel.action), to: cancel.target, from: cancel, for: nil)
      XCTAssertTrue(cancelled)
    }
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
