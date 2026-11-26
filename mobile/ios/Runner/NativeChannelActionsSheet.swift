import Flutter
import UIKit

/// Presents channel actions above the complete Flutter/native view hierarchy.
final class NativeChannelActionsSheetCoordinator: NSObject, UIAdaptivePresentationControllerDelegate {
  private let channel: FlutterMethodChannel
  private weak var parent: UIViewController?
  private var navigation: UINavigationController?
  private var pending: FlutterResult?
  private var session: String?
  private var finishing = false
  private var contentEngine: FlutterEngine?
  private var contentChannel: FlutterMethodChannel?
  private var payload: [String: Any] = [:]
  private var readinessTimeout: DispatchWorkItem?
  private var contentReady = false
  private var needsContentUpdate = false

  init(messenger: FlutterBinaryMessenger, parentViewController: UIViewController?) {
    channel = FlutterMethodChannel(name: "buzz/channel_actions_sheet", binaryMessenger: messenger)
    parent = parentViewController
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { result(nil); return }
      guard let data = call.arguments as? [String: Any], let request = data["session"] as? String else {
        result(FlutterError(code: "invalid_arguments", message: "Missing channel actions session", details: nil)); return
      }
      switch call.method {
      case "update":
        if request == self.session {
          self.payload = data
          if self.contentReady {
            self.contentChannel?.invokeMethod("update", arguments: data)
          } else {
            self.needsContentUpdate = true
          }
        }
        result(nil)
      case "dismiss":
        if request == self.session { self.finish(nil) }
        result(nil)
      case "present":
        guard self.pending == nil else { result(FlutterError(code: "busy", message: "Channel actions are already open", details: nil)); return }
        var presenter = self.parent ?? UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
          .filter { $0.activationState == .foregroundActive }.flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController
        while let next = presenter?.presentedViewController { presenter = next }
        guard let presenter, presenter.view.window != nil, !presenter.isBeingDismissed else {
          result(FlutterError(code: "unavailable", message: "Cannot present channel actions", details: nil)); return
        }
        self.payload = data
        self.session = request
        self.pending = result
        self.finishing = false
        guard self.prepareContent(), let engine = self.contentEngine else {
          self.resolve(FlutterError(code: "engine_failed", message: "Could not start channel settings", details: nil))
          return
        }
        // Supply the model without loading a second account.
        let flutter = FlutterViewController(engine: engine, nibName: nil, bundle: nil)
        let content = NativeChannelContentController(flutter: flutter, data: data)
        content.close = { [weak self] in self?.finish(nil) }
        let navigation = UINavigationController(rootViewController: content)
        let isPhone = presenter.traitCollection.userInterfaceIdiom == .phone && presenter.traitCollection.horizontalSizeClass == .compact
        navigation.modalPresentationStyle = isPhone ? .pageSheet : .formSheet
        navigation.overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
        navigation.preferredContentSize = CGSize(width: 540, height: max(320, presenter.view.bounds.height - 48))
        navigation.sheetPresentationController?.detents = [.large()]
        #if canImport(UIKit, _version: 9127.0.85)
        if #available(iOS 27.0, *), !isPhone { navigation.sheetPresentationController?.preferredPlacement = .center }
        #endif
        self.navigation = navigation
        let timeout = DispatchWorkItem { [weak self] in
          guard let self, self.session == request else { return }
          self.finish(FlutterError(code: "content_timeout", message: "Channel settings did not load", details: nil))
        }
        self.readinessTimeout = timeout
        DispatchQueue.main.asyncAfter(deadline: .now() + 10, execute: timeout)
        presenter.present(navigation, animated: !UIAccessibility.isReduceMotionEnabled) {
          navigation.presentationController?.delegate = self
        }
      default: result(FlutterMethodNotImplemented)
      }
    }
  }

  /// Start only for an eligible presentation, never at app launch or dismissal.
  /// The app's camera, audio, storage and account plugins are not used here.
  @discardableResult
  private func prepareContent() -> Bool {
    if contentEngine != nil { return true }
    let engine = FlutterEngine(name: "channel-sheet", project: nil, allowHeadlessExecution: true)
    guard engine.run(withEntrypoint: "channelSheetMain", libraryURI: "package:buzz/features/channels/channel_actions_sheet.dart") else {
      engine.destroyContext()
      return false
    }
    contentEngine = engine
    // Flutter requires run() before installing message handlers.
    let bridge = FlutterMethodChannel(name: "buzz/channel_sheet_content", binaryMessenger: engine.binaryMessenger)
    contentChannel = bridge
    bridge.setMethodCallHandler { [weak self, weak engine] call, reply in
      guard let self, let engine, self.contentEngine === engine else { reply(nil); return }
      switch call.method {
      case "initial":
        self.needsContentUpdate = false
        reply(self.session == nil ? nil : self.payload)
      case "ready":
        guard self.session != nil else { reply(nil); return }
        self.contentReady = true
        self.readinessTimeout?.cancel()
        if self.needsContentUpdate {
          self.contentChannel?.invokeMethod("update", arguments: self.payload)
          self.needsContentUpdate = false
        }
        reply(nil)
      case "select":
        reply(nil)
        self.finish(call.arguments as? String)
      default: reply(FlutterMethodNotImplemented)
      }
    }
    return true
  }

  private func finish(_ action: Any?) {
    guard !finishing, let navigation else { return }
    finishing = true
    navigation.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in self?.resolve(action) }
  }

  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    guard presentationController.presentedViewController === navigation else { return }
    resolve(nil)
  }

  private func resolve(_ action: Any?) {
    let callback = pending
    pending = nil
    navigation = nil
    readinessTimeout?.cancel()
    readinessTimeout = nil
    contentChannel?.setMethodCallHandler(nil)
    contentChannel = nil
    contentEngine?.destroyContext()
    contentEngine = nil
    contentReady = false
    needsContentUpdate = false
    payload = [:]
    session = nil
    finishing = false
    callback?(action)
  }

  deinit {
    readinessTimeout?.cancel()
    contentChannel?.setMethodCallHandler(nil)
    contentEngine?.destroyContext()
  }
}

/// UIKit owns only presentation and Close. Buzz's Flutter widgets render content.
private final class NativeChannelContentController: UIViewController {
  var close: (() -> Void)?
  private let flutter: FlutterViewController
  private let background: UIColor
  // Flutter scrollables receive raw touches, not UIScrollView pan recognizers.
  // Claim drags in the content without cancelling those touches, so UIKit's
  // sheet pan waits instead of moving the sheet or cancelling Flutter's drag.
  private let contentPan = UIPanGestureRecognizer()
  private var coordinatedPans = Set<ObjectIdentifier>()

  init(flutter: FlutterViewController, data: [String: Any]) {
    self.flutter = flutter
    let colors = data["colors"] as? [String: Any]
    let surfaceKey = data["dark"] as? Bool == true ? "control" : "container"
    let argb = (colors?[surfaceKey] as? NSNumber)?.uint32Value ?? 0xfff2f2f7
    background = UIColor(red: CGFloat((argb >> 16) & 255) / 255,
                         green: CGFloat((argb >> 8) & 255) / 255,
                         blue: CGFloat(argb & 255) / 255,
                         alpha: CGFloat((argb >> 24) & 255) / 255)
    super.init(nibName: nil, bundle: nil)
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = background
    navigationItem.leftBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in self?.close?() })
    let appearance = UINavigationBarAppearance()
    appearance.configureWithOpaqueBackground()
    appearance.backgroundColor = background
    appearance.shadowColor = .clear
    navigationItem.standardAppearance = appearance
    navigationItem.scrollEdgeAppearance = appearance
    addChild(flutter)
    flutter.view.translatesAutoresizingMaskIntoConstraints = false
    flutter.view.backgroundColor = background
    view.addSubview(flutter.view)
    contentPan.cancelsTouchesInView = false
    contentPan.delaysTouchesBegan = false
    contentPan.delaysTouchesEnded = false
    flutter.view.addGestureRecognizer(contentPan)
    NSLayoutConstraint.activate([
      flutter.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
      flutter.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
      flutter.view.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
      flutter.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
    ])
    flutter.didMove(toParent: self)

  }
  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    // These are ancestors only: the navigation bar remains available for
    // UIKit's own swipe dismissal, and Flutter keeps all of its touch events.
    var ancestor = flutter.view.superview
    while let current = ancestor {
      for case let pan as UIPanGestureRecognizer in current.gestureRecognizers ?? [] {
        if coordinatedPans.insert(ObjectIdentifier(pan)).inserted {
          pan.require(toFail: contentPan)
        }
      }
      if current === navigationController?.presentationController?.containerView { break }
      ancestor = current.superview
    }
  }

}
