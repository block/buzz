import Flutter
import SwiftUI
import UIKit

private struct NativeEmojiMediaBytesError: Error {}

final class NativeEmojiPickerCoordinator: NSObject,
  UIAdaptivePresentationControllerDelegate
{
  private let channel: FlutterMethodChannel
  private static weak var activeCoordinator: NativeEmojiPickerCoordinator?
  private weak var parentViewController: UIViewController?
  private weak var presentedController: UIViewController?
  private var didNotifyDismissal = false
  private var isDismissing = false

  init(
    messenger: FlutterBinaryMessenger,
    parentViewController: UIViewController?
  ) {
    channel = FlutterMethodChannel(
      name: "buzz/native_emoji_picker",
      binaryMessenger: messenger
    )
    self.parentViewController = parentViewController
    super.init()
    Self.activeCoordinator = self
    channel.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result)
    }
  }

  @MainActor private static var nextMediaRequestID = 0

  /// [url]'s encoded bytes from Flutter, which serves them from the shared
  /// media disk cache or downloads them once. Cancelling the task cancels the
  /// download when no other surface needs it.
  @MainActor
  static func mediaBytes(for url: URL) async throws -> Data {
    guard let channel = activeCoordinator?.channel else {
      throw NativeEmojiMediaBytesError()
    }
    nextMediaRequestID += 1
    let id = nextMediaRequestID
    return try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { continuation in
        channel.invokeMethod(
          "mediaBytes",
          arguments: ["id": id, "url": url.absoluteString]
        ) { result in
          guard let bytes = result as? FlutterStandardTypedData else {
            continuation.resume(throwing: NativeEmojiMediaBytesError())
            return
          }
          continuation.resume(returning: bytes.data)
        }
      }
    } onCancel: {
      Task { @MainActor in
        channel.invokeMethod("cancelMediaBytes", arguments: id)
      }
    }
  }

  private func handle(
    _ call: FlutterMethodCall,
    result: @escaping FlutterResult
  ) {
    guard call.method == "present" else {
      result(FlutterMethodNotImplemented)
      return
    }
    guard let arguments = call.arguments as? [String: Any] else {
      result(
        FlutterError(
          code: "invalid_arguments",
          message: "Expected emoji-picker configuration.",
          details: nil
        )
      )
      return
    }

    DispatchQueue.main.async { [weak self] in
      result(self?.present(arguments: arguments) ?? false)
    }
  }

  @MainActor
  private func present(arguments: [String: Any]) -> Bool {
    // A sheet is already owned by an earlier caller; report busy rather than a
    // successful presentation so the caller does not treat this as its own.
    guard presentedController == nil else { return false }
    guard
      let data = NativeEmojiPickerDataLoader.load(arguments: arguments),
      let presenter = topViewController(
        from: parentViewController ?? activeWindowRootViewController()
      )
    else {
      return false
    }

    didNotifyDismissal = false
    isDismissing = false
    let appearance = NativeEmojiPickerAppearance(arguments: arguments)
    let content = NativeEmojiPickerView(
      data: data,
      appearance: appearance,
      largeEmoji: arguments["largeEmoji"] as? Bool ?? false,
      initialSkinTone: (arguments["skinTone"] as? NSNumber)?.intValue ?? 0,
      onSelect: { [weak self] emoji in self?.select(emoji) },
      onSkinToneChanged: { [weak self] value in
        self?.channel.invokeMethod("skinToneChanged", arguments: value)
      },
      onClose: { [weak self] in self?.dismiss() }
    )
    let controller = UIHostingController(rootView: content)
    controller.view.backgroundColor = appearance.surface
    controller.modalPresentationStyle = .pageSheet
    controller.overrideUserInterfaceStyle = appearance.isDark ? .dark : .light

    if let sheet = controller.sheetPresentationController {
      let compactID = UISheetPresentationController.Detent.Identifier(
        "buzz.emoji.compact"
      )
      let mediumID = UISheetPresentationController.Detent.Identifier(
        "buzz.emoji.medium"
      )
      sheet.detents = [
        .custom(identifier: compactID) { context in
          context.maximumDetentValue * 0.34
        },
        .custom(identifier: mediumID) { context in
          context.maximumDetentValue * 0.67
        },
        .large(),
      ]
      sheet.selectedDetentIdentifier = mediumID
      sheet.prefersGrabberVisible = true
      sheet.prefersScrollingExpandsWhenScrolledToEdge = false
      sheet.prefersEdgeAttachedInCompactHeight = false
      sheet.widthFollowsPreferredContentSizeWhenEdgeAttached = true
    }

    presentedController = controller
    presenter.present(controller, animated: true) { [weak self, weak controller] in
      controller?.presentationController?.delegate = self
    }
    return true
  }

  @MainActor
  private func select(_ emoji: String) {
    // A single presentation returns at most one selection. The sheet stays
    // live through its dismissal animation, so ignore extra taps that arrive
    // before dismissal completes to avoid emitting duplicate selections.
    guard !isDismissing else { return }
    channel.invokeMethod("selected", arguments: emoji)
    dismiss()
  }

  @MainActor
  private func dismiss() {
    isDismissing = true
    guard let controller = presentedController else {
      notifyDismissalIfNeeded()
      return
    }
    controller.dismiss(animated: true) { [weak self] in
      self?.notifyDismissalIfNeeded()
    }
  }

  func presentationControllerDidDismiss(
    _ presentationController: UIPresentationController
  ) {
    notifyDismissalIfNeeded()
  }

  @MainActor
  private func notifyDismissalIfNeeded() {
    guard !didNotifyDismissal else { return }
    didNotifyDismissal = true
    presentedController = nil
    channel.invokeMethod("dismissed", arguments: nil)
  }

  @MainActor
  private func activeWindowRootViewController() -> UIViewController? {
    UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap(\.windows)
      .first(where: \.isKeyWindow)?
      .rootViewController
  }

  @MainActor
  private func topViewController(
    from viewController: UIViewController?
  ) -> UIViewController? {
    if let presented = viewController?.presentedViewController {
      return topViewController(from: presented)
    }
    if let navigation = viewController as? UINavigationController {
      return topViewController(from: navigation.visibleViewController)
    }
    if let tab = viewController as? UITabBarController {
      return topViewController(from: tab.selectedViewController)
    }
    return viewController
  }
}

// A profile gets a dedicated Flutter view controller backed by the running Dart
// isolate. The presenting controller stays in its original UIKit hierarchy.
@MainActor
final class ProfileSheetPresenter: NSObject, UISheetPresentationControllerDelegate {
  private let compact = UISheetPresentationController.Detent.Identifier("profile.header")
  private let channel: FlutterMethodChannel
  // UIKit uses the same scroll view for detent negotiation and list scrolling.
  // Keep the Flutter viewport at its expanded height while the sheet reveals it.
  private final class ProfileHost: UIViewController, UIScrollViewDelegate {
    let scroll = UIScrollView()
    let surface: FlutterViewController
    let onScroll: (CGFloat) -> Void
    var documentHeight: CGFloat = 340
    var viewportHeight: CGFloat
    init(surface: FlutterViewController, viewportHeight: CGFloat, onScroll: @escaping (CGFloat) -> Void) {
      self.surface = surface
      self.viewportHeight = viewportHeight
      self.onScroll = onScroll
      super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
      view = scroll
      scroll.contentInsetAdjustmentBehavior = .never
      scroll.alwaysBounceVertical = true
      scroll.showsVerticalScrollIndicator = false
      scroll.delegate = self
      // The embedded navigation backdrop has its own scroll view. Register the
      // profile viewport explicitly for UIKit’s sheet expansion gesture.
      setContentScrollView(scroll)
      addChild(surface)
      scroll.addSubview(surface.view)
      surface.didMove(toParent: self)
    }
    override func viewDidLayoutSubviews() {
      super.viewDidLayoutSubviews()
      let size = scroll.bounds.size
      let height = max(viewportHeight, size.height)
      scroll.contentSize = CGSize(width: size.width, height: max(size.height, documentHeight))
      surface.view.frame = CGRect(x: 0, y: scroll.contentOffset.y, width: size.width, height: height)
    }
    func scrollViewDidScroll(_ scrollView: UIScrollView) {
      surface.view.frame.origin.y = scrollView.contentOffset.y
      onScroll(scrollView.contentOffset.y)
    }
  }
  private final class Session {
    let id: Int
    let source: FlutterViewController
    let backdrop: UIView
    let surface: FlutterViewController
    let host: ProfileHost
    var ordinaryDetents: [UISheetPresentationController.Detent] = []
    var height: CGFloat = 340
    init(id: Int, source: FlutterViewController, backdrop: UIView, surface: FlutterViewController, host: ProfileHost) {
      self.host = host
      self.id = id
      self.source = source
      self.backdrop = backdrop
      self.surface = surface
    }
  }
  private var sessions = [Session]()
  init(messenger: FlutterBinaryMessenger) {
    channel = FlutterMethodChannel(name: "buzz/profile_sheet", binaryMessenger: messenger)
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self, let args = call.arguments as? [String: Any], let id = args["id"] as? Int else { result(false); return }
      switch call.method {
      case "prepare":
        let root = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
          .flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController
        guard let source = self.sessions.last?.surface ?? root as? FlutterViewController,
              source.presentedViewController == nil,
              source.engine.viewController === source,
              let backdrop = self.chatBackdrop(for: source.view) else { result(false); return }
        backdrop.frame = source.view.bounds
        backdrop.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        backdrop.isUserInteractionEnabled = false
        source.view.addSubview(backdrop)
        let engine = source.engine
        engine.viewController = nil
        let surface = FlutterViewController(engine: engine, nibName: nil, bundle: nil)
        let host = ProfileHost(surface: surface,
          viewportHeight: (source.view.window?.bounds.height ?? source.view.bounds.height) - source.view.safeAreaInsets.top
        ) { [weak self] offset in
          self?.channel.invokeMethod("scroll", arguments: ["id": id, "offset": offset])
        }
        host.modalPresentationStyle = .pageSheet
        let session = Session(id: id, source: source, backdrop: backdrop, surface: surface, host: host)
        self.sessions.append(session)
        if let sheet = host.sheetPresentationController {
          sheet.detents = [.custom(identifier: self.compact) { [weak session] context in
            min(context.maximumDetentValue, max(1, (session?.height ?? 340) - (session?.source.view.safeAreaInsets.bottom ?? 0)))
          }, .large()]
          session.ordinaryDetents = sheet.detents
          sheet.selectedDetentIdentifier = self.compact
          sheet.prefersGrabberVisible = true
          sheet.prefersScrollingExpandsWhenScrolledToEdge = true
          sheet.delegate = self
        }
        result(true)
      case "present":
        guard let s = self.sessions.last, s.id == id else { result(false); return }
        guard s.source.viewIfLoaded?.window != nil,
              s.source.presentedViewController == nil,
              !s.source.isBeingDismissed else { result(false); return }
        s.source.present(s.host, animated: true)
        result(s.host.presentingViewController != nil)
      case "photo":
        if let s = self.sessions.last, s.id == id, let sheet = s.host.sheetPresentationController,
           let open = args["open"] as? Bool {
          s.host.isModalInPresentation = open
          sheet.animateChanges {
            sheet.detents = open ? [.large()] : s.ordinaryDetents
            sheet.selectedDetentIdentifier = .large
          }
          self.channel.invokeMethod("expanded", arguments: ["id": id, "expanded": true])
        }
        result(nil)
      case "expand":
        if let s = self.sessions.last, s.id == id, let sheet = s.host.sheetPresentationController,
           sheet.selectedDetentIdentifier != .large {
          sheet.animateChanges { sheet.selectedDetentIdentifier = .large }
        }
        result(nil)
      case "height":
        if let s = self.sessions.last, s.id == id, let h = args["height"] as? Double {
          if let document = args["document"] as? Double {
            s.host.documentHeight = document
            s.host.scroll.isScrollEnabled = args["active"] as? Bool ?? true
            s.host.view.setNeedsLayout()
          }
          s.host.sheetPresentationController?.prefersGrabberVisible = args["showGrabber"] as? Bool ?? true
          guard s.height != h else { result(nil); return }
          s.height = h
          s.host.sheetPresentationController?.invalidateDetents()
        }
        result(nil)
      case "dismiss":
        guard let s = self.sessions.last, s.id == id else { result(nil); return }
        if s.host.presentingViewController == nil {
          self.restore(s)
          result(nil)
        } else {
          s.host.dismiss(animated: true) { self.restore(s); result(nil) }
        }
      default: result(FlutterMethodNotImplemented)
      }
    }
  }

  // Capture the window while the chat and its material backdrops are still
  // composited together, before the Flutter surface moves into the sheet.
  private func chatBackdrop(for view: UIView) -> UIView? {
    guard let window = view.window, !view.bounds.isEmpty else { return nil }
    let frame = view.convert(view.bounds, to: window)
    var captured = false
    let image = UIGraphicsImageRenderer(size: frame.size).image { context in
      context.cgContext.translateBy(x: -frame.minX, y: -frame.minY)
      captured = window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
    }
    guard captured else { return nil }
    return UIImageView(image: image)
  }

  private func restore(_ session: Session) {
    guard sessions.last === session else { return }
    sessions.removeLast()
    let engine = session.surface.engine
    engine.viewController = session.source
    session.source.beginAppearanceTransition(true, animated: false)
    session.source.view.setNeedsLayout()
    session.source.view.layoutIfNeeded()
    session.source.endAppearanceTransition()
    session.backdrop.removeFromSuperview()
  }
  func presentationControllerDidDismiss(_ controller: UIPresentationController) {
    guard let s = sessions.last, controller.presentedViewController === s.host else { return }
    restore(s)
    channel.invokeMethod("dismissed", arguments: ["id": s.id])
  }
  func sheetPresentationControllerDidChangeSelectedDetentIdentifier(_ sheet: UISheetPresentationController) {
    guard let s = sessions.last, sheet.presentedViewController === s.host else { return }
    channel.invokeMethod("expanded", arguments: ["id": s.id, "expanded": sheet.selectedDetentIdentifier == .large])
  }
}
