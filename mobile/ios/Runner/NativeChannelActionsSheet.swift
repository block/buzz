import Flutter
import UIKit

/// Presents channel actions above the complete Flutter/native view hierarchy.
final class NativeChannelActionsSheetCoordinator: NSObject, UIAdaptivePresentationControllerDelegate {
  private let messenger: FlutterBinaryMessenger
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
  private var zoomAnchor: UIImageView?
  private var landingSource: (session: String, view: UIImageView)?
  private var contentReady = false
  private var needsContentUpdate = false

  // The Duo cover reports the phone idiom even when it exposes no fold strip.
  private static let hasChatSettingsZoomHardware: Bool = {
    guard UIDevice.current.userInterfaceIdiom == .phone else { return false }
    #if targetEnvironment(simulator)
    let model = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"]
    #else
    var system = utsname()
    guard uname(&system) == 0 else { return false }
    let model: String? = withUnsafeBytes(of: &system.machine) {
      String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self)
    }
    #endif
    return model?.hasPrefix("iPhone") == true && model != "iPhone19,4"
  }()

  private static var supportsChatSettingsZoom: Bool {
    guard #available(iOS 18.0, *) else { return false }
    return hasChatSettingsZoomHardware && !UIAccessibility.isReduceMotionEnabled
  }

  init(messenger: FlutterBinaryMessenger, parentViewController: UIViewController?) {
    self.messenger = messenger
    channel = FlutterMethodChannel(name: "buzz/channel_actions_sheet", binaryMessenger: messenger)
    parent = parentViewController
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { result(nil); return }
      if call.method == "supportsChatSettingsZoom" {
        result(Self.supportsChatSettingsZoom)
        return
      }
      guard let data = call.arguments as? [String: Any], let request = data["session"] as? String else {
        result(FlutterError(code: "invalid_arguments", message: "Missing channel actions session", details: nil)); return
      }
      switch call.method {
      case "update":
        if request == self.session {
          if let bytes = (data["zoomArtwork"] as? FlutterStandardTypedData)?.data,
             let image = UIImage(data: bytes) {
            self.zoomAnchor?.image = image
            (self.navigation?.topViewController as? NativeChannelContentController)?.sharedPortrait.image = image
          }
          self.payload = data.filter { $0.key != "zoomArtwork" }
          self.payload["hasNativeHero"] = self.zoomAnchor != nil
          if self.contentReady {
            self.contentChannel?.invokeMethod("update", arguments: self.payload)
          } else {
            self.needsContentUpdate = true
          }
        }
        result(nil)
      case "sourceRestored":
        self.releaseLandingSource(request)
        result(nil)
      case "dismiss":
        self.releaseLandingSource(request)
        if request == self.session {
          self.payload["holdsZoomSource"] = false
          self.finish(nil)
        }
        result(nil)
      case "present":
        if data["zoomSource"] != nil && !Self.supportsChatSettingsZoom {
          result(FlutterError(code: "unavailable", message: "Chat settings zoom is unavailable", details: nil))
          return
        }
        guard self.pending == nil else { result(FlutterError(code: "busy", message: "Channel actions are already open", details: nil)); return }
        var presenter = self.parent ?? UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
          .filter { $0.activationState == .foregroundActive }.flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController
        while let next = presenter?.presentedViewController { presenter = next }
        guard let presenter, let window = presenter.view.window, !presenter.isBeingDismissed else {
          result(FlutterError(code: "unavailable", message: "Cannot present channel actions", details: nil)); return
        }
        var zoomRect: CGRect?
        if data["zoomSource"] != nil {
          guard let frame = data["zoomSource"] as? [CGFloat], frame.count == 4, frame.allSatisfy(\.isFinite) else {
            result(FlutterError(code: "unavailable", message: "Chat settings source is unavailable", details: nil)); return
          }
          let rect = CGRect(x: frame[0], y: frame[1], width: frame[2], height: frame[3])
          guard frame[2] > 0, frame[3] > 0, window.bounds.contains(rect) else {
            result(FlutterError(code: "unavailable", message: "Chat settings source is unavailable", details: nil)); return
          }
          zoomRect = rect
        }
        self.landingSource?.view.removeFromSuperview()
        self.landingSource = nil
        self.payload = data.filter { $0.key != "zoomArtwork" }
        self.session = request
        self.pending = result
        self.finishing = false
        guard self.prepareContent(), let engine = self.contentEngine else {
          self.resolve(FlutterError(code: "engine_failed", message: "Could not start channel settings", details: nil))
          return
        }
        var portraitImage: UIImage?
        if let rect = zoomRect {
          let artwork = (data["zoomArtwork"] as? FlutterStandardTypedData)?.data
          let image = artwork.flatMap { UIImage(data: $0) } ?? UIGraphicsImageRenderer(size: rect.size).image { context in
            context.cgContext.translateBy(x: -rect.minX, y: -rect.minY)
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: false)
          }
          let anchor = UIImageView(image: image)
          anchor.frame = presenter.view.convert(rect, from: window)
          presenter.view.addSubview(anchor)
          portraitImage = image
          anchor.contentMode = .scaleAspectFit
          anchor.isUserInteractionEnabled = false
          anchor.isAccessibilityElement = false
          self.zoomAnchor = anchor
          self.payload["hasNativeHero"] = true
        }
        // Supply the model without loading a second account.
        let flutter = ChannelActionsFlutterViewController(engine: engine, nibName: nil, bundle: nil)
        let content = NativeChannelContentController(flutter: flutter, data: self.payload)
        content.sharedPortrait.image = portraitImage
        content.finishPortraitReturn = { [weak self, weak content] in
          guard let self, let content, !self.finishing else { return }
          guard let anchor = self.zoomAnchor, let window = anchor.window else { self.finish(nil); return }
          self.finishing = true
          content.landPortrait(at: anchor.convert(anchor.bounds, to: window), in: window) {
            self.navigation?.dismiss(animated: false) { self.resolve(nil) }
          }
        }
        let close: () -> Void = { [weak self] in self?.finish(nil) }
        content.close = close
        flutter.onEscape = close
        let navigation = ChannelActionsNavigationController(rootViewController: content)
        let isPhone = presenter.traitCollection.userInterfaceIdiom == .phone && presenter.traitCollection.horizontalSizeClass == .compact
        navigation.overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
        if let anchor = self.zoomAnchor {
          navigation.modalPresentationStyle = .overFullScreen
          if #available(iOS 18.0, *) {
            let options = UIViewController.Transition.ZoomOptions()
            options.interactiveDismissShouldBegin = { [weak navigation] context in
              guard let navigation, !UIAccessibility.isReduceMotionEnabled else { return false }
              let bar = navigation.navigationBar
              let inBar = bar.convert(bar.bounds, to: navigation.view).contains(context.location)
              let rtl = navigation.view.effectiveUserInterfaceLayoutDirection == .rightToLeft
              let x = rtl ? navigation.view.bounds.maxX - context.location.x : context.location.x
              let vx = rtl ? -context.velocity.dx : context.velocity.dx
              return (x <= 24 && vx > abs(context.velocity.dy)) ||
                (inBar && context.velocity.dy > abs(vx))
            }
            options.alignmentRectProvider = { [weak content, weak navigation] _ in
              guard let content, let navigation else { return nil }
              navigation.view.layoutIfNeeded()
              let portrait = content.sharedPortrait
              guard !portrait.bounds.isEmpty, !portrait.isHidden,
                    portrait.frame.intersects(content.heroOverlay.bounds) else { return nil }
              let frame = portrait.convert(portrait.bounds, to: navigation.view)
              guard !navigation.hasAppeared else { return frame }
              let side: CGFloat = 208
              return CGRect(x: frame.midX - side / 2, y: frame.midY - side / 2,
                            width: side, height: side)
            }
            navigation.preferredTransition = .zoom(options: options) { [weak anchor] _ in anchor }
          }
          navigation.onDismiss = { [weak self] in
            if self?.finishing == false { self?.resolve(nil) }
          }
        } else {
          navigation.modalPresentationStyle = isPhone ? .pageSheet : .formSheet
          navigation.preferredContentSize = CGSize(width: 540, height: max(320, presenter.view.bounds.height - 48))
          navigation.sheetPresentationController?.detents = [.large()]
          #if canImport(UIKit, _version: 9127.0.85)
          if #available(iOS 27.0, *), !isPhone { navigation.sheetPresentationController?.preferredPlacement = .center }
          #endif
        }
        self.navigation = navigation
        let timeout = DispatchWorkItem { [weak self] in
          guard let self, self.session == request else { return }
          self.finish(FlutterError(code: "content_timeout", message: "Channel settings did not load", details: nil))
        }
        self.readinessTimeout = timeout
        DispatchQueue.main.asyncAfter(deadline: .now() + 10, execute: timeout)
        let present = {
          guard self.session == request, !self.finishing else { return }
          guard presenter.view.window != nil, !presenter.isBeingDismissed, presenter.presentedViewController == nil,
                zoomRect == nil || Self.supportsChatSettingsZoom else {
            self.resolve(FlutterError(code: "unavailable", message: "Cannot present channel actions", details: nil))
            return
          }
          presenter.present(navigation, animated: !UIAccessibility.isReduceMotionEnabled) {
            navigation.presentationController?.delegate = self
          }
        }
        if self.zoomAnchor != nil, data["holdsZoomSource"] as? Bool == true {
          let source = FlutterMethodChannel(name: "buzz/channel_actions_sheet/source/\(request)", binaryMessenger: self.messenger)
          source.invokeMethod("covered", arguments: nil) { _ in present() }
        } else {
          present()
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
    engine.registrar(forPlugin: "BuzzToastLayer")?.register(ToastLayerFactory(), withId: "buzz/toast_layer")
    // Flutter requires run() before installing message handlers.
    let bridge = FlutterMethodChannel(name: "buzz/channel_sheet_content", binaryMessenger: engine.binaryMessenger)
    contentChannel = bridge
    bridge.setMethodCallHandler { [weak self, weak engine] call, reply in
      guard let self, let engine, self.contentEngine === engine else { reply(nil); return }
      switch call.method {
      case "initial":
        self.needsContentUpdate = false
        reply(self.session == nil ? nil : self.payload)
      case "heroGeometry":
        if let data = call.arguments as? [String: Any],
           let content = self.navigation?.topViewController as? NativeChannelContentController {
          content.updateHero(data)
        }
        reply(nil)
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
        let selection = call.arguments as? String
        if self.zoomAnchor != nil, let session = self.session, let selection,
           ["Star", "Unstar", "Mute", "Unmute", "Pin", "Unpin"].contains(selection) {
          FlutterMethodChannel(name: "buzz/channel_actions_sheet/source/\(session)", binaryMessenger: self.messenger)
            .invokeMethod("selectInPlace", arguments: selection)
        } else {
          self.finish(selection)
        }
      default: reply(FlutterMethodNotImplemented)
      }
    }
    return true
  }

  private func finish(_ action: Any?) {
    guard !finishing, let navigation else { return }
    finishing = true
    if navigation.presentingViewController == nil { resolve(action); return }
    navigation.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in self?.resolve(action) }
  }

  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    guard presentationController.presentedViewController === navigation else { return }
    resolve(nil)
  }

  private func releaseLandingSource(_ request: String) {
    guard let source = landingSource, source.session == request else { return }
    source.view.removeFromSuperview()
    landingSource = nil
  }

  private func resolve(_ action: Any?) {
    guard let callback = pending else { return }
    pending = nil
    if let anchor = zoomAnchor, let session, payload["holdsZoomSource"] as? Bool == true {
      landingSource = (session, anchor)
    } else {
      zoomAnchor?.removeFromSuperview()
    }
    zoomAnchor = nil
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
    callback(action)
  }

  deinit {
    landingSource?.view.removeFromSuperview()
    readinessTimeout?.cancel()
    contentChannel?.setMethodCallHandler(nil)
    contentEngine?.destroyContext()
  }
}

private final class ChannelActionsFlutterViewController: FlutterViewController {
  var onEscape: (() -> Void)?

  override func accessibilityPerformEscape() -> Bool {
    // Flutter's default sends popRoute to this engine's single root route.
    guard let onEscape else { return super.accessibilityPerformEscape() }
    onEscape()
    return true
  }
}

private final class ChannelActionsNavigationController: UINavigationController {
  var onDismiss: (() -> Void)?
  private(set) var hasAppeared = false

  override func viewWillAppear(_ animated: Bool) {
    super.viewWillAppear(animated)
    view.accessibilityViewIsModal = onDismiss != nil
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    hasAppeared = true
  }

  override func viewWillDisappear(_ animated: Bool) {
    super.viewWillDisappear(animated)
    if onDismiss != nil, isBeingDismissed, let window = view.window, let transitionCoordinator {
      NavigationTitleView.revealSettingsTitle(in: window, alongside: transitionCoordinator)
    }
  }

  override func viewDidDisappear(_ animated: Bool) {
    super.viewDidDisappear(animated)
    if isBeingDismissed || presentingViewController == nil { onDismiss?() }
  }
}

// Sample the list before Flutter starts dragging. A swipe that reaches the
// top partway through must finish its scroll and rubber-band normally.
private final class ChannelSettingsPan: UIPanGestureRecognizer {
  var atTop: (() -> Bool)?
  private(set) var startedAtTop = false

  override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
    startedAtTop = atTop?() == true
    super.touchesBegan(touches, with: event)
  }
}

/// UIKit owns only presentation and Close. Buzz's Flutter widgets render content.
private final class NativeChannelContentController: UIViewController, UIGestureRecognizerDelegate {
  var close: (() -> Void)?
  var finishPortraitReturn: (() -> Void)?
  let heroOverlay = UIView()
  let sharedPortrait = UIImageView()
  private let sharedName = UILabel()
  private let headerMaterial = UIVisualEffectView(effect: UIBlurEffect(style: .systemUltraThinMaterial))
  private let headerFade = CAGradientLayer()
  private var isLanding = false
  private var measuredHero = false
  private var scrollOffset: CGFloat = 0
  private let cropSurface = UIView()
  private let pullMask = CAShapeLayer()
  private var pullBackdrop: UIView?
  private lazy var pull = ChannelSettingsPan(target: self, action: #selector(pullSettings(_:)))
  private let flutter: FlutterViewController
  private let background: UIColor
  private let isZoom: Bool
  // Flutter scrollables receive raw touches, not UIScrollView pan recognizers.
  // Claim drags in the content without cancelling those touches, so UIKit's
  // sheet pan waits instead of moving the sheet or cancelling Flutter's drag.
  private let contentPan = UIPanGestureRecognizer()
  private var coordinatedPans = Set<ObjectIdentifier>()

  init(flutter: FlutterViewController, data: [String: Any]) {
    isZoom = data["zoomSource"] != nil
    self.flutter = flutter
    let colors = data["colors"] as? [String: Any]
    let surfaceKey = data["dark"] as? Bool == true ? "control" : "container"
    let argb = (colors?[surfaceKey] as? NSNumber)?.uint32Value ?? 0xfff2f2f7
    background = UIColor(red: CGFloat((argb >> 16) & 255) / 255,
                         green: CGFloat((argb >> 8) & 255) / 255,
                         blue: CGFloat(argb & 255) / 255,
                         alpha: CGFloat((argb >> 24) & 255) / 255)
    super.init(nibName: nil, bundle: nil)
    sharedName.text = data["title"] as? String
    sharedName.textAlignment = .center
    sharedName.numberOfLines = 2
    sharedName.font = .systemFont(ofSize: 24, weight: .semibold)
    if let argb = (colors?["onSurface"] as? NSNumber)?.uint32Value {
      sharedName.textColor = UIColor(red: CGFloat((argb >> 16) & 255) / 255, green: CGFloat((argb >> 8) & 255) / 255, blue: CGFloat(argb & 255) / 255, alpha: CGFloat(argb >> 24) / 255)
    }
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

  override func accessibilityPerformEscape() -> Bool {
    guard let close else { return false }
    close()
    return true
  }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = isZoom ? .clear : background
    let contentSurface: UIView = isZoom ? cropSurface : view
    if isZoom {
      cropSurface.frame = view.bounds
      cropSurface.autoresizingMask = [.flexibleWidth, .flexibleHeight]
      cropSurface.backgroundColor = background
      view.addSubview(cropSurface)
    }
    let dismiss = UIAction { [weak self] _ in
      self?.close?()
    }
    if isZoom {
      let back = UIBarButtonItem(image: UIImage(systemName: "chevron.backward"), primaryAction: dismiss)
      back.accessibilityLabel = NSLocalizedString("Back", comment: "Return to chat")
      navigationItem.leftBarButtonItem = back
    } else {
      navigationItem.leftBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: dismiss)
    }
    let appearance = UINavigationBarAppearance()
    if isZoom {
      appearance.configureWithTransparentBackground()
    } else {
      appearance.configureWithOpaqueBackground()
      appearance.backgroundColor = background
    }
    appearance.shadowColor = .clear
    navigationItem.standardAppearance = appearance
    navigationItem.scrollEdgeAppearance = appearance
    addChild(flutter)
    flutter.view.translatesAutoresizingMaskIntoConstraints = false
    flutter.view.backgroundColor = background
    contentSurface.addSubview(flutter.view)
    contentPan.cancelsTouchesInView = false
    contentPan.delaysTouchesBegan = false
    contentPan.delaysTouchesEnded = false
    if isZoom {
      pull.maximumNumberOfTouches = 1
      pull.delegate = self
      pull.atTop = { [weak self] in self.map { abs($0.scrollOffset) <= 0.5 } ?? false }
      flutter.view.addGestureRecognizer(pull)
    } else {
      flutter.view.addGestureRecognizer(contentPan)
    }
    NSLayoutConstraint.activate([
      flutter.view.leadingAnchor.constraint(equalTo: contentSurface.leadingAnchor),
      flutter.view.trailingAnchor.constraint(equalTo: contentSurface.trailingAnchor),
      flutter.view.topAnchor.constraint(equalTo: isZoom ? contentSurface.topAnchor : view.safeAreaLayoutGuide.topAnchor),
      flutter.view.bottomAnchor.constraint(equalTo: contentSurface.bottomAnchor),
    ])
    flutter.didMove(toParent: self)
    if isZoom {
      heroOverlay.isUserInteractionEnabled = false
      heroOverlay.accessibilityElementsHidden = true
      heroOverlay.clipsToBounds = true
      cropSurface.addSubview(heroOverlay)
      heroOverlay.addSubview(sharedPortrait)
      heroOverlay.addSubview(sharedName)
      sharedPortrait.contentMode = .scaleAspectFit
      headerMaterial.isUserInteractionEnabled = false
      headerMaterial.clipsToBounds = true
      headerMaterial.layer.maskedCorners = [.layerMinXMinYCorner, .layerMaxXMinYCorner]
      headerFade.colors = [UIColor.black.cgColor, UIColor.black.cgColor, UIColor.clear.cgColor]
      headerFade.locations = [0, 0.55, 1]
      headerMaterial.layer.mask = headerFade
      view.addSubview(headerMaterial)
    }

  }
  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    // These are ancestors only: the navigation bar remains available for
    // UIKit's own swipe dismissal, and Flutter keeps all of its touch events.
    if isZoom {
      heroOverlay.frame = flutter.view.frame
      headerMaterial.frame = CGRect(x: 0, y: 0, width: view.bounds.width, height: view.safeAreaInsets.top)
      headerFade.frame = headerMaterial.bounds
      if !measuredHero {
        let midX = heroOverlay.bounds.midX
        sharedPortrait.bounds.size = CGSize(width: 80, height: 80)
        sharedPortrait.center = CGPoint(x: midX, y: view.safeAreaInsets.top + 56)
        let size = sharedName.sizeThatFits(CGSize(width: max(1, heroOverlay.bounds.width - 40), height: 80))
        sharedName.bounds.size = size
        sharedName.center = CGPoint(x: midX, y: view.safeAreaInsets.top + 104 + size.height / 2)
      }
      return
    }
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

  func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
    guard recognizer === pull, pull.startedAtTop, !UIAccessibility.isReduceMotionEnabled,
          navigationController?.transitionCoordinator == nil else { return false }
    let velocity = pull.velocity(in: view.window)
    return velocity.y > abs(velocity.x) * 1.25
  }

  @objc private func pullSettings(_ pan: UIPanGestureRecognizer) {
    guard let navigation = navigationController, let card = navigation.view,
          let window = card.window else { return }
    let delta = pan.translation(in: window)
    let distance = max(0, delta.y)
    let progress = min(1, distance / max(1, window.bounds.height))
    switch pan.state {
    case .began, .changed:
      if pan.state == .began, let container = card.superview {
        let backdrop = UIView(frame: container.bounds)
        backdrop.backgroundColor = .black
        backdrop.isUserInteractionEnabled = false
        container.insertSubview(backdrop, belowSubview: card)
        pullBackdrop = backdrop
        pullMask.removeAnimation(forKey: "return")
      }
      pullBackdrop?.alpha = min(0.13, progress * 0.6)
      card.backgroundColor = .clear
      let scale = 1 - 0.55 * progress
      CATransaction.begin()
      CATransaction.setDisableActions(true)
      // Scale the content uniformly, then crop the last rows independently.
      // In screen coordinates the visible height falls with scale squared.
      let bounds = card.bounds
      let visibleBounds = CGRect(x: bounds.minX, y: bounds.minY, width: bounds.width,
                                 height: bounds.height * scale)
      let radius = min(48, distance * 0.5)
      pullMask.frame = cropSurface.bounds
      pullMask.path = UIBezierPath(roundedRect: visibleBounds, cornerRadius: radius).cgPath
      headerMaterial.layer.cornerRadius = radius
      // UIKit's live blur and navigation glass must stay outside the mask.
      cropSurface.layer.mask = pullMask
      CATransaction.commit()
      let bottomInset = progress * window.safeAreaInsets.bottom
      let top = bounds.height - visibleBounds.height * scale - bottomInset
      let translationY = top - bounds.height * (1 - scale) / 2
      card.transform = CGAffineTransform(translationX: delta.x * 0.7, y: translationY)
        .scaledBy(x: scale, y: scale)
    case .ended where distance > window.bounds.height * 0.18 ||
        (distance > 28 && pan.velocity(in: window).y > 700):
      finishPortraitReturn?()
    case .ended, .cancelled, .failed:
      let backdrop = pullBackdrop
      let duration = 0.32
      if let startPath = pullMask.presentation()?.path ?? pullMask.path {
        let endPath = UIBezierPath(roundedRect: cropSurface.bounds, cornerRadius: 0.01).cgPath
        let crop = CASpringAnimation(keyPath: "path")
        crop.fromValue = startPath
        crop.toValue = endPath
        crop.stiffness = 230
        crop.damping = 27.3
        crop.duration = duration
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        pullMask.path = endPath
        pullMask.add(crop, forKey: "return")
        CATransaction.commit()
      }
      UIView.animate(withDuration: duration, delay: 0, usingSpringWithDamping: 0.9,
                     initialSpringVelocity: 0, options: [.beginFromCurrentState]) {
        backdrop?.alpha = 0
        card.transform = .identity
        self.headerMaterial.layer.cornerRadius = 0
      } completion: { _ in
        backdrop?.removeFromSuperview()
        guard self.pullBackdrop === backdrop else { return }
        self.pullBackdrop = nil
        self.cropSurface.layer.mask = nil
      }
    default: break
    }
  }

  // Complete a content drag from the visible crop. Starting a new UIKit zoom
  // here resets that crop and its dimming for the first dismissal frame.
  func landPortrait(at target: CGRect, in window: UIWindow, completion: @escaping () -> Void) {
    guard let card = navigationController?.view, let container = card.superview,
          target.width > 0 else { completion(); return }
    let startPath = pullMask.path ?? UIBezierPath(roundedRect: cropSurface.bounds, cornerRadius: 1).cgPath
    pullMask.frame = cropSurface.bounds
    cropSurface.layer.mask = pullMask
    let portrait = sharedPortrait.convert(sharedPortrait.bounds, to: card)
    guard portrait.width > 0 else { completion(); return }
    let destination = window.convert(target, to: container)
    let scale = destination.width / portrait.width
    let dx = destination.midX - card.center.x - (portrait.midX - card.bounds.midX) * scale
    let dy = destination.midY - card.center.y - (portrait.midY - card.bounds.midY) * scale
    let cropPortrait = sharedPortrait.convert(sharedPortrait.bounds, to: cropSurface)
    let endPath = UIBezierPath(roundedRect: cropPortrait, cornerRadius: cropPortrait.width / 2).cgPath
    let crop = CABasicAnimation(keyPath: "path")
    crop.fromValue = startPath
    crop.toValue = endPath
    let duration = 0.34
    crop.duration = duration
    crop.timingFunction = CAMediaTimingFunction(name: .easeOut)
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    pullMask.path = endPath
    pullMask.add(crop, forKey: "landing")
    CATransaction.commit()
    isLanding = true
    card.isUserInteractionEnabled = false
    // Flutter paints the sheet surface. Fade the rows, subtitle and native
    // name together before the compact chat caption becomes visible.
    card.backgroundColor = .clear
    cropSurface.backgroundColor = .clear
    UIView.animate(withDuration: duration * 0.46, delay: 0, options: [.curveEaseOut]) {
      self.flutter.view.alpha = 0
      self.sharedName.alpha = 0
      self.headerMaterial.alpha = 0
      self.navigationController?.navigationBar.alpha = 0
    }
    NavigationTitleView.revealSettingsTitle(in: window, duration: duration)
    UIView.animate(withDuration: crop.duration, delay: 0, options: [.curveEaseOut]) {
      card.transform = CGAffineTransform(translationX: dx, y: dy).scaledBy(x: scale, y: scale)
      self.pullBackdrop?.alpha = 0
    } completion: { _ in
      self.pullBackdrop?.removeFromSuperview()
      self.pullBackdrop = nil
      completion()
    }
  }

  func updateHero(_ data: [String: Any]) {
    guard isZoom, !isLanding else { return }
    func rect(_ key: String) -> CGRect? {
      guard let frame = data[key] as? [CGFloat], frame.count == 4 else { return nil }
      return CGRect(x: frame[0], y: frame[1], width: frame[2], height: frame[3])
    }
    let offset = data["scrollOffset"] as? CGFloat ?? 0
    scrollOffset = offset
    let collapsed = rect("name").map { !$0.isEmpty ? $0.maxY <= view.safeAreaInsets.top : offset > 0 } ?? (offset > 0)
    let title = collapsed ? data["title"] as? String : nil
    if navigationItem.title != title {
      if let bar = navigationController?.navigationBar, !UIAccessibility.isReduceMotionEnabled {
        UIView.transition(with: bar, duration: 0.16, options: [.transitionCrossDissolve, .allowUserInteraction]) {
          self.navigationItem.title = title
          bar.layoutIfNeeded()
        }
      } else {
        navigationItem.title = title
      }
    }
    guard let portrait = rect("portrait"), !portrait.isEmpty,
          let name = rect("name"), !name.isEmpty,
          portrait.midX.isFinite, portrait.midY.isFinite, name.midY.isFinite,
          portrait.midX > 0 else {
      // A cold Flutter engine can report before its first list layout. Only
      // scrolling the header out of the list should remove the artwork.
      let scrolled = (data["scrollOffset"] as? CGFloat ?? 0) > 0
      measuredHero = scrolled
      sharedPortrait.isHidden = scrolled
      sharedName.isHidden = scrolled
      if !scrolled { view.setNeedsLayout() }
      return
    }
    measuredHero = true
    sharedPortrait.isHidden = false
    sharedName.isHidden = false
    sharedPortrait.bounds.size = portrait.size
    sharedPortrait.center = CGPoint(x: portrait.midX, y: portrait.midY)
    sharedName.text = data["title"] as? String
    if let size = data["fontSize"] as? CGFloat { sharedName.font = .systemFont(ofSize: size, weight: .semibold) }
    sharedName.bounds.size = name.size
    sharedName.center = CGPoint(x: name.midX, y: name.midY)
  }
}
