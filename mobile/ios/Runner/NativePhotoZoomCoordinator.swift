import Flutter
import UIKit
import AVKit

/// Preview-only presentation. Flutter owns authenticated image loading.
final class NativePhotoZoomCoordinator {
  private let channel: FlutterMethodChannel
  private weak var parent: UIViewController?
  private var active: UIViewController?
  private var activeSession: Int?

  init(messenger: FlutterBinaryMessenger, parentViewController: UIViewController?) {
    channel = FlutterMethodChannel(name: "buzz/native_photo_zoom", binaryMessenger: messenger)
    parent = parentViewController
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { result("unavailable"); return }
      if call.method == "isSupported" {
        if #available(iOS 18.0, *) { result(true) } else { result(false) }
        return
      }
      if call.method == "imageFrame" {
        if #available(iOS 18.0, *),
          let args = call.arguments as? [String: Any],
          let session = args["session"] as? Int, session == self.activeSession,
          let index = args["index"] as? Int,
          let bytes = args["bytes"] as? FlutterStandardTypedData,
          let controller = self.active as? NativePhotoZoomViewController
        {
          controller.updateFrame(bytes.data, at: index)
        }
        result(nil)
        return
      }
      guard call.method == "present" else { result(FlutterMethodNotImplemented); return }
      guard #available(iOS 18.0, *), self.active == nil,
        let args = call.arguments as? [String: Any],
        let bytes = args["bytes"] as? FlutterStandardTypedData,
        let image = UIImage(data: bytes.data),
        let x = args["x"] as? Double, let y = args["y"] as? Double,
        let width = args["width"] as? Double, let height = args["height"] as? Double,
        let angle = args["angle"] as? Double,
        [x, y, width, height, angle].allSatisfy({ $0.isFinite }), width > 0, height > 0,
        let presenter = self.presentationController(), presenter.presentedViewController == nil,
        let window = presenter.view.window
      else { result("unavailable"); return }

      let anchor = UIImageView(image: image)
      anchor.contentMode = .scaleAspectFill
      anchor.clipsToBounds = true
      anchor.layer.cornerRadius = 26
      anchor.layer.cornerCurve = .continuous
      anchor.bounds = CGRect(x: 0, y: 0, width: width, height: height)
      anchor.center = presenter.view.convert(CGPoint(x: x, y: y), from: window)
      anchor.transform = CGAffineTransform(rotationAngle: angle)
      anchor.isUserInteractionEnabled = false
      anchor.isAccessibilityElement = false
      presenter.view.addSubview(anchor)

      let labels = args["labels"] as? [String] ?? ["Photo"]
      let initial = args["initialIndex"] as? Int ?? 0
      guard !labels.isEmpty, labels.indices.contains(initial), let session = args["session"] as? Int else {
        anchor.removeFromSuperview(); result("unavailable"); return
      }
      let controller = NativePhotoZoomViewController(
        image: image, labels: labels, initialIndex: initial,
        canReply: args["canReply"] as? Bool ?? false,
        canMore: args["canMore"] as? Bool ?? false,
        canDelete: args["canDelete"] as? Bool ?? false,
        videoIndices: Set(args["videoIndices"] as? [Int] ?? [])
      )
      if let center = args["railCenterX"] as? Double,
         let width = args["railWindowWidth"] as? Double,
         let height = args["railWindowHeight"] as? Double,
         [center, width, height].allSatisfy({ $0.isFinite }), width > 0, height > 0 {
        controller.sourceRailGeometry = (CGSize(width: width, height: height), CGFloat(center))
      }
      controller.request = { [weak self] method, index, action, completion in
        var payload: [String: Any] = ["session": session, "index": index]
        if let action { payload["action"] = action }
        self?.channel.invokeMethod(method, arguments: payload, result: completion)
      }
      controller.onSelection = { [weak anchor, weak presenter] image, geometry in
        guard let anchor, let presenter else { return }
        anchor.image = image
        guard let geometry,
          let x = geometry["x"] as? Double, let y = geometry["y"] as? Double,
          let width = geometry["width"] as? Double, let height = geometry["height"] as? Double,
          let angle = geometry["angle"] as? Double,
          [x, y, width, height, angle].allSatisfy({ $0.isFinite }), width > 0, height > 0
        else { anchor.isHidden = true; return }
        anchor.isHidden = false
        anchor.transform = .identity
        anchor.bounds = CGRect(x: 0, y: 0, width: width, height: height)
        anchor.center = presenter.view.convert(CGPoint(x: x, y: y), from: presenter.view.window)
        anchor.transform = CGAffineTransform(rotationAngle: angle)
      }
      // Keep Flutter rendering underneath so page changes can update the return card.
      controller.modalPresentationStyle = .overFullScreen
      let animate = !(args["reduceMotion"] as? Bool ?? false) && !UIAccessibility.isReduceMotionEnabled
      if animate {
        let options = UIViewController.Transition.ZoomOptions()
        options.interactiveDismissShouldBegin = { [weak controller] context in
          controller?.shouldBeginDismissal(location: context.location, velocity: context.velocity) ?? false
        }
        options.alignmentRectProvider = { [weak controller] _ in controller?.photoAlignmentRect() }
        controller.preferredTransition = .zoom(options: options) { [weak anchor] _ in
          guard let anchor, !anchor.isHidden else { return nil }; return anchor
        }
      }
      let previousAccessibilityHidden = presenter.view.accessibilityElementsHidden
      presenter.view.accessibilityElementsHidden = true
      var finished = false
      controller.onClose = { [weak self, weak anchor, weak presenter] action in
        guard !finished else { return }
        finished = true
        presenter?.view.accessibilityElementsHidden = previousAccessibilityHidden
        anchor?.removeFromSuperview()
        self?.active = nil
        self?.activeSession = nil
        result(action)
      }
      self.active = controller
      self.activeSession = session
      presenter.present(controller, animated: animate)
    }
  }

  // Implicit-engine registration can happen before the Flutter view controller
  // exists. Resolve the active scene at presentation time, as the other native
  // presentation bridges do, instead of retaining that initial nil value.
  private func presentationController() -> UIViewController? {
    var controller = parent?.viewIfLoaded?.window != nil ? parent :
      UIApplication.shared.connectedScenes
        .compactMap { $0 as? UIWindowScene }
        .filter { $0.activationState == .foregroundActive }
        .flatMap(\.windows)
        .first(where: \.isKeyWindow)?.rootViewController
    while let current = controller {
      if let navigation = current as? UINavigationController {
        controller = navigation.visibleViewController
      } else if let tab = current as? UITabBarController {
        controller = tab.selectedViewController
      } else {
        return current
      }
    }
    return nil
  }

}

@available(iOS 18.0, *)
private final class NativePhotoZoomViewController: UIViewController, UIPageViewControllerDataSource, UIPageViewControllerDelegate, UIGestureRecognizerDelegate {
  private let pager = UIPageViewController(transitionStyle: .scroll, navigationOrientation: .horizontal)
  private let indicator = UIPageControl()
  private let close = UIButton(type: .system)
  private let navigationSizingBar = UINavigationBar()
  private let pageContainer = UIView()
  private let reply = UIButton(type: .system)
  private let more = UIButton(type: .system)
  private let share = UIButton(type: .system)
  private let labels: [String]
  private let videoIndices: Set<Int>
  private var videoPath: (Int, String)?
  private var appeared = false
  private var previews: [Int: UIImage] = [:]
  private var images: [Int: UIImage] = [:]
  private var pending: [String: UUID] = [:]
  private let canReply: Bool, canMore: Bool, canDelete: Bool
  private var index: Int
  private var action = "closed"
  private var finishing = false
  private lazy var downwardDismiss = UIPanGestureRecognizer(target: self, action: #selector(dragToDismiss(_:)))
  // Video pages hide the overlaid actions and transport on tap, like other video apps.
  private lazy var chromeTap = UITapGestureRecognizer(target: self, action: #selector(toggleChrome))
  private var galleryInsets = UIEdgeInsets.zero
  private var photoInsets = UIEdgeInsets.zero
  private var chromeHidden = false
  private var chromeTimer: Timer?
  var selectionFeedback = UISelectionFeedbackGenerator()
  private var paging = false
  private var acting = false
  var sourceRailGeometry: (size: CGSize, centerX: CGFloat)?
  var onClose: ((String) -> Void)?
  var request: ((String, Int, String?, @escaping FlutterResult) -> Void)?
  var onSelection: ((UIImage?, [String: Any]?) -> Void)?

  init(image: UIImage, labels: [String], initialIndex: Int, canReply: Bool, canMore: Bool, canDelete: Bool, videoIndices: Set<Int> = []) {
    self.previews[initialIndex] = image
    self.labels = labels; index = initialIndex
    self.videoIndices = videoIndices
    self.canReply = canReply; self.canMore = canMore; self.canDelete = canDelete
    super.init(nibName: nil, bundle: nil)
  }
  required init?(coder: NSCoder) { nil }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .black
    overrideUserInterfaceStyle = .dark
    view.accessibilityViewIsModal = true
    downwardDismiss.delegate = self
    downwardDismiss.maximumNumberOfTouches = 1
    view.addGestureRecognizer(downwardDismiss)
    chromeTap.delegate = self
    chromeTap.cancelsTouchesInView = false
    view.addGestureRecognizer(chromeTap)
    NotificationCenter.default.addObserver(self, selector: #selector(voiceOverChanged),
                                           name: UIAccessibility.voiceOverStatusDidChangeNotification, object: nil)
    pager.dataSource = self; pager.delegate = self
    addChild(pager); view.addSubview(pager.view); pager.didMove(toParent: self)
    pager.setViewControllers([page(index)], direction: .forward, animated: false)
    loadWindow()
    configure(close, symbol: "xmark", label: videoIndices.isEmpty ? "Close photo" : "Close media", selector: #selector(closePhoto))
    configure(reply, symbol: "arrowshape.turn.up.left", label: "Reply in thread", selector: #selector(replyToMessage))
    configure(more, symbol: "ellipsis", label: "More image actions", selector: nil)
    configure(share, symbol: "square.and.arrow.up", label: "Share video", selector: #selector(shareVideo))
    reply.isEnabled = canReply; updateChrome()
    more.showsMenuAsPrimaryAction = true
    var menu = [UIAction(title: "Save image", image: UIImage(systemName: "square.and.arrow.down")) { [weak self] _ in self?.perform("save") },
      UIAction(title: "Share image", image: UIImage(systemName: "square.and.arrow.up")) { [weak self] _ in self?.perform("share") },
      UIAction(title: "Copy image link", image: UIImage(systemName: "link")) { [weak self] _ in self?.perform("copy") }]
    if canDelete {
      menu.append(UIAction(title: "Delete message", image: UIImage(systemName: "trash"), attributes: .destructive) { [weak self] _ in self?.confirmDelete() })
    }
    more.menu = UIMenu(children: menu)
    indicator.allowsContinuousInteraction = true
    indicator.numberOfPages = labels.count; indicator.currentPage = index
    indicator.hidesForSinglePage = true
    indicator.currentPageIndicatorTintColor = .white
    indicator.pageIndicatorTintColor = .gray
    indicator.accessibilityLabel = videoIndices.isEmpty ? "Photo" : "Attachment"
    indicator.addTarget(self, action: #selector(selectPage), for: .valueChanged)
    let pageSurface: UIVisualEffectView
    if #available(iOS 26.0, *) {
      let glass = UIGlassEffect(style: .regular)
      glass.isInteractive = true
      pageSurface = UIVisualEffectView(effect: glass)
    } else {
      pageSurface = UIVisualEffectView(effect: UIBlurEffect(style: .systemMaterialDark))
    }
    pageSurface.isHidden = labels.count < 2
    pageSurface.clipsToBounds = true
    pageSurface.layer.cornerRadius = 22
    pageSurface.layer.cornerCurve = .continuous
    pageContainer.addSubview(pageSurface)
    pageSurface.contentView.addSubview(indicator)
    pageSurface.translatesAutoresizingMaskIntoConstraints = false
    indicator.translatesAutoresizingMaskIntoConstraints = false
    NSLayoutConstraint.activate([
      pageContainer.heightAnchor.constraint(equalToConstant: 44),
      pageSurface.centerXAnchor.constraint(equalTo: pageContainer.centerXAnchor),
      pageSurface.topAnchor.constraint(equalTo: pageContainer.topAnchor),
      pageSurface.bottomAnchor.constraint(equalTo: pageContainer.bottomAnchor),
      pageSurface.leadingAnchor.constraint(greaterThanOrEqualTo: pageContainer.leadingAnchor),
      pageSurface.trailingAnchor.constraint(lessThanOrEqualTo: pageContainer.trailingAnchor),
      indicator.leadingAnchor.constraint(equalTo: pageSurface.contentView.leadingAnchor, constant: 8),
      indicator.trailingAnchor.constraint(equalTo: pageSurface.contentView.trailingAnchor, constant: -8),
      indicator.topAnchor.constraint(equalTo: pageSurface.contentView.topAnchor),
      indicator.bottomAnchor.constraint(equalTo: pageSurface.contentView.bottomAnchor),
    ])
    // Keep all controls in one hierarchy while the window folds or rotates.
    // The photo surface can extend behind the rail without moving its pages.
    for control in [close, reply, more, share, pageContainer] { view.addSubview(control) }
    pager.view.clipsToBounds = true
  }
  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    layoutGallery(insets: view.safeAreaInsets)
  }
  func layoutGallery(insets: UIEdgeInsets) {
    galleryInsets = insets
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) { page.chromeInsets = insets }
    let bounds = view.bounds
    let right = insets.right > 0 && insets.left == 0
    let left = insets.left > 0 && insets.right == 0
    let inRail = insets.top == 0 && (right || left)
    let safe = bounds.inset(by: insets)
    let bottom = safe.maxY - 56
    let indicatorBounds = inRail ? bounds : safe
    // Every page shares one full-window pager frame, so swiping between a photo and a
    // video never resizes a page after it settles. Photos keep main's band inside their page.
    pager.view.frame = bounds
    let photoInsets = inRail ? .zero : UIEdgeInsets(top: safe.minY + 56, left: safe.minX,
                                                    bottom: max(0, bounds.maxY - (bottom - 16)), right: max(0, bounds.maxX - safe.maxX))
    self.photoInsets = photoInsets
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) { page.photoInsets = photoInsets }
    pageContainer.frame = CGRect(x: indicatorBounds.minX + 68, y: bottom,
                                 width: max(0, indicatorBounds.width - 136), height: 44)
    if inRail { layoutRail(insets: insets, onRight: right) } else { layoutRows(safe: safe, bottom: bottom) }
  }
  // Duo rail: Close at the top of the hinge-side strip, Reply and More at its bottom.
  private func layoutRail(insets: UIEdgeInsets, onRight right: Bool) {
    let bounds = view.bounds
    let left = !right
    let width = right ? insets.right : insets.left
    var centerX = right ? bounds.maxX - width / 2 : width / 2
    var top: CGFloat = min(bounds.width, bounds.height) >= 600 ? 110 : 170
    var railBottom = bounds.maxY - max(insets.bottom, 12)
    if let measured = measuredRail(width: width, onRight: right) {
      centerX = measured.centerX
      railBottom = min(railBottom, measured.bottom ?? railBottom)
      top = measured.top ?? top
    }
    centerX = sourceRailCenter(width: width, onRight: right) ?? centerX
    // Match IosNavigationMetrics.railCenter's inward optical alignment.
    centerX += left ? 6 : -6
    close.frame = CGRect(x: centerX - 24, y: top + 6, width: 48, height: 48)
    reply.frame = CGRect(x: centerX - 24, y: railBottom - 108, width: 48, height: 48)
    more.frame = CGRect(x: centerX - 24, y: railBottom - 48, width: 48, height: 48)
    share.frame = more.frame
  }
  // iOS 27.1 reports the hinge and camera housing, so the rail can center on and clear them.
  private func measuredRail(width: CGFloat, onRight right: Bool) -> (centerX: CGFloat, top: CGFloat?, bottom: CGFloat?)? {
    #if canImport(UIKit, _version: 9127.0.85)
    guard #available(iOS 27.1, *), let window = view.window else { return nil }
    let strip = CGRect(x: right ? window.bounds.maxX - width : 0, y: 0,
                       width: width, height: window.bounds.height)
    let occlusions = window.reservedRegions(kind: .occlusion)
      .filter { $0.isActive && $0.frame.intersects(strip) }
    let regions = occlusions.filter { $0.frame.midY < window.bounds.midY }
      .sorted { $0.frame.minY < $1.frame.minY }
    let margins = window.rootViewController?.view.layoutMargins ?? .zero
    let margin = right ? margins.left : margins.right
    let axis = regions.first.map { $0.frame.inset(by: $0.margins).midX }
      ?? (right ? window.bounds.maxX - margin - 28 : margin + 28)
    let bottom = occlusions.filter { $0.frame.midY >= window.bounds.midY }.map { $0.frame.minY }.min()
    let top = regions.map { $0.frame.maxY }.max()
    return (view.convert(CGPoint(x: axis, y: 0), from: window).x,
            top.map { view.convert(CGPoint(x: 0, y: $0), from: window).y + 12 },
            bottom.map { view.convert(CGPoint(x: 0, y: $0), from: window).y - 12 })
    #else
    return nil
    #endif
  }
  // Reuse the app rail's measured axis while its window/side still match.
  // Presentation can change UIKit's fallback margins under the modal.
  private func sourceRailCenter(width: CGFloat, onRight right: Bool) -> CGFloat? {
    guard let source = sourceRailGeometry, source.size == view.bounds.size else { return nil }
    let side = right ? view.bounds.maxX - width...view.bounds.maxX : 0...width
    return side.contains(source.centerX) ? source.centerX : nil
  }
  // Phones and tablets: Close on the top row; other actions on the bottom row, or the top row for video.
  private func layoutRows(safe: CGRect, bottom: CGFloat) {
    let centerY = safe.minY + navigationSizingBar.sizeThatFits(view.bounds.size).height / 2
    close.frame = CGRect(x: safe.maxX - 60, y: centerY - 22, width: 44, height: 44)
    if videoIndices.contains(index) {
      // The video transport owns the bottom of the screen, so every action moves to the top row.
      reply.frame = CGRect(x: safe.minX + 16, y: centerY - 22, width: 44, height: 44)
      share.frame = CGRect(x: safe.maxX - 112, y: centerY - 22, width: 44, height: 44)
      pageContainer.frame = CGRect(x: safe.minX + 68, y: centerY - 22,
                                   width: max(0, safe.width - 188), height: 44)
    } else {
      reply.frame = CGRect(x: safe.minX + 16, y: bottom, width: 44, height: 44)
    }
    more.frame = CGRect(x: safe.maxX - 60, y: bottom, width: 44, height: 44)
  }
  func shouldBeginDismissal(location: CGPoint, velocity: CGVector) -> Bool {
    !paging && !finishing && pager.view.frame.contains(location)
      && velocity.dy > abs(velocity.dx) * 1.5
      && ((pager.viewControllers?.first as? NativePhotoPage)?.isAtRestingZoom ?? false)
  }
  func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
    if gestureRecognizer === chromeTap {
      return !finishing && videoIndices.contains(index) && !UIAccessibility.isVoiceOverRunning
    }
    guard gestureRecognizer === downwardDismiss else { return true }
    let velocity = downwardDismiss.velocity(in: view)
    return shouldBeginDismissal(location: downwardDismiss.location(in: view), velocity: CGVector(dx: velocity.x, dy: velocity.y))
  }
  func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer) -> Bool {
    [downwardDismiss, chromeTap].contains { $0 === gestureRecognizer || $0 === otherGestureRecognizer }
  }
  func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
    guard gestureRecognizer === chromeTap else { return true }
    // Only the bare video toggles the actions: never our buttons, Retry or the video transport.
    var touched = touch.view
    while let current = touched, current !== view {
      if isOwnControl(current) { return false }
      touched = current.superview
    }
    return true
  }
  private func isOwnControl(_ view: UIView) -> Bool {
    view is UIControl || view is NativeVideoTransport || view === pageContainer
  }
  @objc private func toggleChrome() { setChrome(hidden: !chromeHidden) }
  // VoiceOver can't tap the bare video to reveal faded actions, so bring them back
  // as soon as it starts. The fade timer and the reveal tap both skip VoiceOver.
  @objc private func voiceOverChanged() { revealForVoiceOver(running: UIAccessibility.isVoiceOverRunning) }
  func revealForVoiceOver(running: Bool) {
    guard running, chromeHidden || chromeTimer != nil else { return }
    setChrome(hidden: false)
  }
  func setChrome(hidden: Bool) {
    chromeTimer?.invalidate(); chromeTimer = nil
    chromeHidden = hidden
    let animations = { self.updateChrome() }
    if UIAccessibility.isReduceMotionEnabled { animations() }
    else { UIView.animate(withDuration: 0.2, delay: 0, options: [.beginFromCurrentState, .allowUserInteraction], animations: animations) }
    if !hidden { scheduleChromeHide() }
  }
  // Fade the actions and transport 3s into playback, as AVKit does with its own controls.
  // Not while a speed menu or AirPlay picker is open over the transport, nor under VoiceOver.
  private func scheduleChromeHide() {
    chromeTimer?.invalidate(); chromeTimer = nil
    guard canFadeChrome else { return }
    chromeTimer = Timer.scheduledTimer(withTimeInterval: 3, repeats: false) { [weak self] _ in
      guard let self, self.presentedViewController == nil, self.canFadeChrome else { return }
      self.setChrome(hidden: true)
    }
  }
  var isChromeFadeScheduled: Bool { chromeTimer != nil }
  private var canFadeChrome: Bool {
    guard !chromeHidden, !finishing, videoIndices.contains(index), !UIAccessibility.isVoiceOverRunning,
      let page = pager.viewControllers?.first as? NativePhotoPage else { return false }
    return page.isPlaying && !page.isShowingMenu
  }
  @objc private func shareVideo() {
    guard !finishing, presentedViewController == nil, let (value, path) = videoPath, value == index else { return }
    let sheet = UIActivityViewController(activityItems: [URL(fileURLWithPath: path)], applicationActivities: nil)
    sheet.popoverPresentationController?.sourceView = share
    present(sheet, animated: true)
  }
  @objc private func dragToDismiss(_ gesture: UIPanGestureRecognizer) {
    // Nested paging/zoom scroll views can consume UIKit's content-dismiss drag.
    // Once an intentional downward drag is established, use the same native
    // zoom dismissal as Close. Horizontal drags never begin this recognizer.
    guard !isBeingDismissed, !finishing,
      gesture.state == .changed || gesture.state == .ended else { return }
    let translation = gesture.translation(in: view)
    if translation.y > 60 && translation.y > abs(translation.x) * 1.5 { closePhoto() }
  }
  func photoAlignmentRect() -> CGRect? {
    view.layoutIfNeeded()
    guard let page = pager.viewControllers?.first as? NativePhotoPage else { return nil }
    page.view.layoutIfNeeded()
    return page.photoFrame(in: view)
  }
  private func configure(_ button: UIButton, symbol: String, label: String, selector: Selector?) {
    var config: UIButton.Configuration
    if #available(iOS 26.0, *) {
      config = .glass()
    } else {
      config = .filled()
      config.baseBackgroundColor = .darkGray
    }
    config.image = UIImage(systemName: symbol)
    config.baseForegroundColor = .white; config.cornerStyle = .capsule
    button.configuration = config; button.accessibilityLabel = label
    if let selector { button.addTarget(self, action: selector, for: .touchUpInside) }
  }
  private func isNearby(_ value: Int) -> Bool { abs(value - index) <= 2 }
  private func trimWindow() {
    previews = previews.filter { isNearby($0.key) }
    images = images.filter { isNearby($0.key) }
    pending = pending.filter { key, _ in
      key.split(separator: ":").last.flatMap { Int($0) }.map(isNearby) ?? false
    }
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) where !isNearby(page.index) {
      page.setImage(nil)
    }
  }
  private func loadWindow() {
    trimWindow()
    for value in max(0, index - 2)...min(labels.count - 1, index + 2) {
      load("preview", value)
    }
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) where isNearby(page.index) {
      if !page.isVideo { load("image", page.index) }
    }
    loadVideo()
  }
  private func load(_ method: String, _ value: Int) {
    guard !finishing, isNearby(value),
      (method == "preview" ? previews[value] : images[value]) == nil else { return }
    let key = "\(method):\(value)"
    guard pending[key] == nil else { return }
    let token = UUID()
    pending[key] = token
    request?(method, value, nil) { [weak self] result in
      guard let self, !self.finishing, self.isNearby(value), self.pending[key] == token else { return }
      self.pending.removeValue(forKey: key)
      let image = (result as? FlutterStandardTypedData).flatMap { UIImage(data: $0.data) }
      // A continuing frame can arrive before this initial reply. Keep the
      // newer full image authoritative over both delayed image and preview data.
      if let image, self.images[value] == nil {
        if method == "preview" { self.previews[value] = image }
        else { self.images[value] = image }
      }
      for page in self.pager.children.compactMap({ $0 as? NativePhotoPage }) where page.index == value {
        if let image = self.images[value] ?? self.previews[value] { page.setImage(image) }
        else if method == "image" { page.setImage(nil) }
      }
      if self.index == value, method == "image" { self.syncSelection() }
    }
  }
  // Flutter decodes authenticated originals and forwards continuing frames.
  // Ignore old pages and closed presentations before allocating a UIImage.
  func updateFrame(_ data: Data, at value: Int) {
    guard !finishing, value == index, let image = UIImage(data: data) else { return }
    if videoIndices.contains(value) { previews[value] = image }
    else { images[value] = image }
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) where page.index == value {
      page.setImage(image)
    }
  }
  private func page(_ value: Int) -> NativePhotoPage {
    let page = NativePhotoPage(index: value, label: labels[value], image: images[value] ?? previews[value], isVideo: videoIndices.contains(value))
    page.chromeInsets = galleryInsets; page.photoInsets = photoInsets
    page.onRetry = { [weak self] in self?.videoPath = nil; self?.updateChrome(); self?.loadVideo() }
    page.onPlayback = { [weak self, weak page] playing in
      guard let self, let page, self.pager.viewControllers?.first === page else { return }
      // Bring the actions back on pause, at the end and while scrubbing.
      if playing { self.scheduleChromeHide() } else { self.setChrome(hidden: false) }
    }
    page.onActivity = { [weak self, weak page] in
      guard let self, let page, self.pager.viewControllers?.first === page else { return }
      self.setChrome(hidden: false)
    }
    if page.isVideo {
      if let (loaded, path) = videoPath, loaded == value { page.setVideo(path, autoplay: appeared) }
    } else { load("image", value) }
    return page
  }
  private func loadVideo() {
    let value = index
    guard !finishing, videoIndices.contains(value), videoPath?.0 != value else { return }
    let key = "video:\(value)"
    guard pending[key] == nil else { return }
    let token = UUID(); pending[key] = token
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) where page.index == value { page.showVideoLoading() }
    request?("video", value, nil) { [weak self] result in
      guard let self, !self.finishing, self.index == value, self.pending[key] == token else { return }
      self.pending.removeValue(forKey: key)
      let path = result as? String
      if let path { self.videoPath = (value, path) }
      self.updateChrome()
      for page in self.pager.children.compactMap({ $0 as? NativePhotoPage }) where page.index == value {
        page.setVideo(path, autoplay: self.appeared)
      }
    }
  }
  private func stopVideos() {
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) { page.stopVideo() }
    videoPath = nil
    pending = pending.filter { !$0.key.hasPrefix("video:") }
    chromeTimer?.invalidate(); chromeTimer = nil
  }
  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated); appeared = true
    selectionFeedback.prepare()
    (pager.viewControllers?.first as? NativePhotoPage)?.playVideo()
  }
  override func viewWillDisappear(_ animated: Bool) {
    super.viewWillDisappear(animated); appeared = false
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) { page.pauseVideo() }
  }
  func pageViewController(_ pageViewController: UIPageViewController, viewControllerBefore viewController: UIViewController) -> UIViewController? {
    guard let page = viewController as? NativePhotoPage, page.index > 0 else { return nil }; return self.page(page.index - 1)
  }
  func pageViewController(_ pageViewController: UIPageViewController, viewControllerAfter viewController: UIViewController) -> UIViewController? {
    guard let page = viewController as? NativePhotoPage, page.index + 1 < labels.count else { return nil }; return self.page(page.index + 1)
  }
  func pageViewController(_ pageViewController: UIPageViewController, willTransitionTo pendingViewControllers: [UIViewController]) {
    // Respond when the swipe starts, not after UIKit finishes settling the page.
    if !paging { selectionFeedback.selectionChanged() }
    paging = true; indicator.isEnabled = false
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) { page.pauseVideo() }
  }
  func pageViewController(_ pageViewController: UIPageViewController, didFinishAnimating finished: Bool, previousViewControllers: [UIViewController], transitionCompleted completed: Bool) {
    paging = false; indicator.isEnabled = true
    selectionFeedback.prepare()
    if completed, let page = pager.viewControllers?.first as? NativePhotoPage {
      for previous in previousViewControllers.compactMap({ $0 as? NativePhotoPage }) where previous !== page { previous.stopVideo() }
      selected(page.index)
    }
    else if appeared { (pager.viewControllers?.first as? NativePhotoPage)?.playVideo() }
  }
  private func updateChrome() {
    let video = videoIndices.contains(index)
    let hidden = video && chromeHidden
    more.isEnabled = canMore && !video && !acting
    share.isEnabled = video && videoPath?.0 == index
    setVisible(more, !video)
    setVisible(share, video && !hidden)
    for control in [close, reply, pageContainer] { setVisible(control, !hidden) }
    // A single page has no indicator, so its strip must not swallow touches.
    pageContainer.isUserInteractionEnabled = !hidden && labels.count > 1
    // The video transport is part of the same chrome, so a tap shows or hides everything together.
    (pager.viewControllers?.first as? NativePhotoPage)?.setTransport(visible: !hidden)
  }
  private func setVisible(_ control: UIView, _ visible: Bool) {
    control.alpha = visible ? 1 : 0
    control.isUserInteractionEnabled = visible
    control.accessibilityElementsHidden = !visible
  }
  private func selected(_ value: Int) {
    guard value != index else { return }
    stopVideos()
    index = value; indicator.currentPage = value
    chromeHidden = false; updateChrome()
    view.setNeedsLayout()
    trimWindow()
    // Move Dart's request window before asking for newly adjacent images.
    syncSelection { [weak self] in self?.loadWindow() }
  }
  @objc private func selectPage() {
    guard !paging, !finishing else { return }
    let target = indicator.currentPage
    guard target != index else { return }
    selectionFeedback.selectionChanged()
    // Scrubbing must not disable its own control or wait for a page animation.
    let direction: UIPageViewController.NavigationDirection = target >= index ? .forward : .reverse
    selected(target)
    pager.setViewControllers([page(target)], direction: direction, animated: false)
  }
  private func syncSelection(completion: (() -> Void)? = nil) {
    let selected = index
    request?("select", selected, nil) { [weak self] result in
      guard let self else { return }
      if self.index == selected {
        self.onSelection?((self.pager.viewControllers?.first as? NativePhotoPage)?.image, result as? [String: Any])
      }
      completion?()
    }
  }
  @objc private func closePhoto() { finish("closed") }
  @objc private func replyToMessage() { if canReply { finish("reply") } }
  private func finish(_ result: String) {
    guard !finishing else { return }; finishing = true; action = result
    stopVideos()
    syncSelection { [weak self] in self?.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) }
  }
  private func confirmDelete() {
    guard canDelete, !acting else { return }
    let alert = UIAlertController(title: "Delete message", message: "This cannot be undone.", preferredStyle: .alert)
    alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
    alert.addAction(UIAlertAction(title: "Delete", style: .destructive) { [weak self, weak alert] _ in
      alert?.dismiss(animated: true) { self?.perform("delete") }
    })
    present(alert, animated: true)
  }
  private func perform(_ action: String) {
    guard canMore, !videoIndices.contains(index), !acting, !finishing else { return }; acting = true; more.isEnabled = false
    request?("action", index, action) { [weak self] result in
      guard let self else { return }; self.acting = false; self.more.isEnabled = self.canMore
      if let error = result as? FlutterError { self.showStatus(error.message ?? "Could not complete action"); return }
      if let message = result as? String { self.showStatus(message) }
      else if action == "delete" { self.finish("closed") }
    }
  }
  private func showStatus(_ message: String) {
    guard !finishing, presentedViewController == nil else { return }
    let alert = UIAlertController(title: nil, message: message, preferredStyle: .alert)
    alert.addAction(UIAlertAction(title: "OK", style: .default)); present(alert, animated: true)
  }
  override func viewDidDisappear(_ animated: Bool) {
    super.viewDidDisappear(animated)
    if isBeingDismissed || presentingViewController == nil {
      finishing = true
      stopVideos()
      pending.removeAll(); previews.removeAll(); images.removeAll()
      request = nil
      onClose?(action); onClose = nil
    }
  }
  override func accessibilityPerformEscape() -> Bool { closePhoto(); return true }
}

@available(iOS 18.0, *)
private final class NativePhotoPage: UIViewController, UIScrollViewDelegate, AVRoutePickerViewDelegate {
  let index: Int
  let isVideo: Bool
  var onRetry: (() -> Void)?
  var onPlayback: ((Bool) -> Void)?
  // Opening a menu or the AirPlay picker holds the fade until it closes.
  var onActivity: (() -> Void)?
  private(set) var isShowingMenu = false
  var chromeInsets = UIEdgeInsets.zero { didSet { if chromeInsets != oldValue { viewIfLoaded?.setNeedsLayout() } } }
  var photoInsets = UIEdgeInsets.zero { didSet { if photoInsets != oldValue && !isVideo { viewIfLoaded?.setNeedsLayout() } } }
  private lazy var video = AVPlayerViewController()
  private let transport = NativeVideoTransport()
  private var timeObserver: Any?
  private var endObserver: NSObjectProtocol?
  private var atEnd = false
  private var resumeAfterScrub = false
  private let retry = UIButton(type: .system)
  private var observation: NSKeyValueObservation?
  private var playbackObservation: NSKeyValueObservation?
  private let imageView = UIImageView()
  private let scroll = UIScrollView()
  private let status = UILabel()
  private let loading = NativeGalleryLoadingIndicator()
  private var previousSize = CGSize.zero
  var image: UIImage? { imageView.image }
  var isPlaying: Bool { isVideo && (video.player.map { $0.timeControlStatus != .paused } ?? false) }
  var isAtRestingZoom: Bool { isVideo || scroll.zoomScale <= scroll.minimumZoomScale + 0.01 }
  func photoFrame(in target: UIView) -> CGRect? {
    if isVideo, video.player != nil, !video.videoBounds.isEmpty { return video.view.convert(video.videoBounds, to: target) }
    guard let image, image.size.width > 0, image.size.height > 0 else { return nil }
    let bounds = imageView.bounds
    let scale = min(bounds.width / image.size.width, bounds.height / image.size.height)
    let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
    return imageView.convert(CGRect(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2, width: size.width, height: size.height), to: target)
  }
  init(index: Int, label: String, image: UIImage?, isVideo: Bool = false) {
    self.index = index; self.isVideo = isVideo; super.init(nibName: nil, bundle: nil)
    imageView.image = image; imageView.accessibilityLabel = label; imageView.isAccessibilityElement = !isVideo
    status.text = !isVideo && image == nil ? "Loading photo…" : nil
  }
  required init?(coder: NSCoder) { nil }
  override func viewDidLoad() {
    super.viewDidLoad(); view.backgroundColor = .black
    scroll.delegate = self; scroll.minimumZoomScale = 1; scroll.maximumZoomScale = 5
    scroll.contentInsetAdjustmentBehavior = .never
    scroll.showsHorizontalScrollIndicator = false; scroll.showsVerticalScrollIndicator = false
    imageView.contentMode = .scaleAspectFit
    scroll.addSubview(imageView); view.addSubview(scroll)
    status.textColor = .white; status.textAlignment = .center; status.numberOfLines = 0
    status.font = .preferredFont(forTextStyle: .body); status.adjustsFontForContentSizeCategory = true
    view.addSubview(status)
    if isVideo {
      video.allowsPictureInPicturePlayback = false
      // AVKit's controls toggle themselves on tap and can't be shown, hidden or observed,
      // so they drift out of step with ours. The gallery draws its own transport instead.
      video.showsPlaybackControls = false
      video.view.backgroundColor = .clear
      video.view.isHidden = true
      addChild(video); view.insertSubview(video.view, belowSubview: status); video.didMove(toParent: self)
      retry.setTitle("Retry video", for: .normal)
      retry.titleLabel?.font = .preferredFont(forTextStyle: .body)
      retry.titleLabel?.adjustsFontForContentSizeCategory = true
      retry.addTarget(self, action: #selector(retryVideo), for: .touchUpInside)
      retry.isHidden = true; view.addSubview(retry)
      transport.isHidden = true; view.addSubview(transport)
      transport.playPause.addTarget(self, action: #selector(togglePlayback), for: .touchUpInside)
      transport.mute.addTarget(self, action: #selector(toggleMute), for: .touchUpInside)
      transport.options.onMenu = { [weak self] open in self?.menuChanged(open: open) }
      transport.airPlay.delegate = self
      transport.slider.addTarget(self, action: #selector(beginScrub), for: .touchDown)
      transport.slider.addTarget(self, action: #selector(scrub), for: .valueChanged)
      transport.slider.addTarget(self, action: #selector(endScrub), for: [.touchUpInside, .touchUpOutside, .touchCancel])
      view.addSubview(loading); loading.setLoading(true)
    }
    let tap = UITapGestureRecognizer(target: self, action: #selector(toggleZoom)); tap.numberOfTapsRequired = 2; if !isVideo { scroll.addGestureRecognizer(tap) }
  }
  func setImage(_ image: UIImage?) {
    imageView.image = image
    if !isVideo { status.text = image == nil ? "Image unavailable" : nil }
  }
  func showVideoLoading() { guard isVideo else { return }; loadViewIfNeeded(); status.text = nil; loading.setLoading(true); retry.isHidden = true }
  func setVideo(_ path: String?, autoplay: Bool) {
    guard isVideo else { return }
    loadViewIfNeeded()
    stopVideo()
    guard let path, FileManager.default.fileExists(atPath: path) else { videoFailed(); return }
    showVideoLoading()
    let item = AVPlayerItem(url: URL(fileURLWithPath: path))
    let player = AVPlayer(playerItem: item)
    video.player = player
    observation = item.observe(\.status, options: [.initial, .new]) { [weak self, weak item] _, _ in
      DispatchQueue.main.async {
        guard let self, let item, self.video.player?.currentItem === item else { return }
        if item.status == .failed { self.videoFailed() }
        else if item.status == .readyToPlay {
          self.loading.setLoading(false); self.status.text = nil; self.video.view.isHidden = false; self.scroll.isHidden = true
          self.transport.isHidden = false; self.updateTransport(); self.rebuildOptions()
        }
      }
    }
    playbackObservation = player.observe(\.timeControlStatus, options: [.new]) { [weak self] player, _ in
      DispatchQueue.main.async {
        guard let self, self.video.player === player else { return }
        self.transport.setPlaying(player.timeControlStatus != .paused)
        self.onPlayback?(player.timeControlStatus != .paused)
      }
    }
    timeObserver = player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 4), queue: .main) { [weak self] _ in
      self?.updateTransport()
    }
    endObserver = NotificationCenter.default.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { [weak self] _ in
      self?.atEnd = true
    }
    atEnd = false
    transport.setMuted(player.isMuted)
    if autoplay { player.play() }
  }
  func setTransport(visible: Bool) {
    guard isVideo else { return }
    loadViewIfNeeded()
    transport.alpha = visible ? 1 : 0
    transport.isUserInteractionEnabled = visible
    transport.accessibilityElementsHidden = !visible
  }
  func routePickerViewWillBeginPresentingRoutes(_ routePickerView: AVRoutePickerView) { menuChanged(open: true) }
  func routePickerViewDidEndPresentingRoutes(_ routePickerView: AVRoutePickerView) { menuChanged(open: false) }
  func menuChanged(open: Bool) { isShowingMenu = open; onActivity?() }
  private func rebuildOptions() {
    guard let player = video.player else { return }
    let rate = player.defaultRate
    transport.setSpeed(rate)
    let speeds = [Float(0.5), 0.75, 1, 1.25, 1.5, 2].map { value in
      UIAction(title: NativeVideoTransport.speed(value), state: abs(rate - value) < 0.01 ? .on : .off) { [weak self] _ in
        self?.setSpeed(value)
      }
    }
    transport.options.menu = UIMenu(title: "Playback Speed", children: speeds)
  }
  func setSpeed(_ rate: Float) {
    guard let player = video.player else { return }
    player.defaultRate = rate
    if player.timeControlStatus != .paused { player.rate = rate }
    rebuildOptions()
  }
  private func updateTransport() {
    guard let item = video.player?.currentItem, !transport.slider.isTracking else { return }
    transport.update(time: item.currentTime().seconds, duration: item.duration.seconds)
  }
  @objc private func togglePlayback() {
    guard let player = video.player else { return }
    if player.timeControlStatus != .paused { player.pause(); return }
    if atEnd { atEnd = false; player.seek(to: .zero) }
    player.play()
  }
  @objc private func toggleMute() {
    guard let player = video.player else { return }
    player.isMuted.toggle(); transport.setMuted(player.isMuted)
  }
  // Pausing while the thumb is held also keeps the actions on screen.
  @objc private func beginScrub() {
    guard let player = video.player else { return }
    resumeAfterScrub = player.timeControlStatus != .paused
    player.pause()
  }
  @objc private func scrub() {
    guard let item = video.player?.currentItem else { return }
    let seconds = Double(transport.slider.value)
    atEnd = false
    transport.update(time: seconds, duration: item.duration.seconds)
    video.player?.seek(to: CMTime(seconds: seconds, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero)
  }
  @objc private func endScrub() {
    if resumeAfterScrub { video.player?.play() }
    resumeAfterScrub = false
  }
  func playVideo() { if isVideo { video.player?.play() } }
  func pauseVideo() { if isVideo { video.player?.pause() } }
  func stopVideo() {
    guard isVideo else { return }
    observation = nil; playbackObservation = nil
    if let timeObserver { video.player?.removeTimeObserver(timeObserver) }
    if let endObserver { NotificationCenter.default.removeObserver(endObserver) }
    timeObserver = nil; endObserver = nil; atEnd = false; resumeAfterScrub = false
    loading.setLoading(false)
    video.player?.pause(); video.player?.replaceCurrentItem(with: nil); video.player = nil
    if isViewLoaded { video.view.isHidden = true; scroll.isHidden = false; transport.isHidden = true }
  }
  private func videoFailed() {
    stopVideo(); status.text = "Video unavailable"; retry.isHidden = false
    UIAccessibility.post(notification: .announcement, argument: status.text)
  }
  @objc private func retryVideo() { showVideoLoading(); onRetry?() }
  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews(); scroll.frame = isVideo ? view.bounds : view.bounds.inset(by: photoInsets)
    status.frame = view.bounds.insetBy(dx: 24, dy: 100)
    if isVideo {
      video.view.frame = view.bounds
      loading.frame = CGRect(x: view.bounds.midX - 22, y: view.bounds.midY - 22, width: 44, height: 44)
      retry.frame = CGRect(x: 24, y: view.bounds.midY + 32, width: max(0, view.bounds.width - 48), height: 48)
      let safe = view.bounds.inset(by: chromeInsets)
      let width = max(0, min(safe.width - 32, 640))
      let height = transport.sizeThatFits(CGSize(width: width, height: 0)).height
      transport.frame = CGRect(x: safe.midX - width / 2, y: safe.maxY - 8 - height, width: width, height: height)
    }
    guard previousSize != scroll.bounds.size else { return }; previousSize = scroll.bounds.size
    scroll.zoomScale = 1; imageView.frame = scroll.bounds; scroll.contentSize = scroll.bounds.size
  }
  func viewForZooming(in scrollView: UIScrollView) -> UIView? { imageView }
  @objc private func toggleZoom() { scroll.setZoomScale(scroll.zoomScale > 1 ? 1 : 2.5, animated: !UIAccessibility.isReduceMotionEnabled) }
}

// Play/pause, position and mute for gallery video. It belongs to the gallery's
// chrome, so it shows and hides with Close, Share and Reply on tap.
private final class NativeVideoTransport: UIView {
  let playPause = UIButton(type: .system)
  let mute = UIButton(type: .system)
  // Playback speed, as AVKit offered. The relay only accepts one video and one audio
  // track, so there are never subtitles or alternate audio to choose.
  let options = MenuButton(type: .system)
  let airPlay = AVRoutePickerView()
  let slider = UISlider()
  private let elapsed = UILabel()
  private let total = UILabel()

  override init(frame: CGRect) {
    super.init(frame: frame)
    backgroundColor = UIColor(white: 0, alpha: 0.5)
    layer.cornerRadius = 22; layer.cornerCurve = .continuous
    // Larger text would squeeze out the scrubber; past this size the timeline gets its own row.
    maximumContentSizeCategory = .accessibilityMedium
    for button in [playPause, mute] {
      button.tintColor = .white
      button.setPreferredSymbolConfiguration(UIImage.SymbolConfiguration(pointSize: 17, weight: .semibold), forImageIn: .normal)
      addSubview(button)
    }
    let font = UIFontMetrics(forTextStyle: .caption1).scaledFont(for: .monospacedDigitSystemFont(ofSize: 12, weight: .medium))
    for label in [elapsed, total] {
      label.textColor = .white; label.font = font; label.adjustsFontForContentSizeCategory = true
      label.isAccessibilityElement = false
      addSubview(label)
    }
    total.textAlignment = .right
    options.tintColor = .white
    options.titleLabel?.font = UIFontMetrics(forTextStyle: .caption1).scaledFont(for: .systemFont(ofSize: 13, weight: .semibold))
    options.titleLabel?.adjustsFontForContentSizeCategory = true
    options.showsMenuAsPrimaryAction = true
    options.accessibilityLabel = "Playback options"
    addSubview(options)
    airPlay.tintColor = .white
    airPlay.prioritizesVideoDevices = true
    addSubview(airPlay)
    slider.minimumTrackTintColor = .white
    slider.maximumTrackTintColor = UIColor(white: 1, alpha: 0.35)
    slider.accessibilityLabel = "Video position"
    addSubview(slider)
    setPlaying(false); setMuted(false); setSpeed(1); update(time: 0, duration: 0)
  }
  required init?(coder: NSCoder) { nil }

  func setSpeed(_ rate: Float) {
    options.setTitle(Self.speed(rate), for: .normal)
    options.accessibilityValue = "\(Self.speed(rate)) speed"
  }
  static func speed(_ rate: Float) -> String {
    let text = String(format: "%.2f", rate).replacingOccurrences(of: #"\.?0+$"#, with: "", options: .regularExpression)
    return "\(text)×"
  }

  func setPlaying(_ playing: Bool) {
    playPause.setImage(UIImage(systemName: playing ? "pause.fill" : "play.fill"), for: .normal)
    playPause.accessibilityLabel = playing ? "Pause" : "Play"
  }
  func setMuted(_ muted: Bool) {
    mute.setImage(UIImage(systemName: muted ? "speaker.slash.fill" : "speaker.wave.2.fill"), for: .normal)
    mute.accessibilityLabel = muted ? "Unmute" : "Mute"
  }
  func update(time: Double, duration: Double) {
    let duration = duration.isFinite ? max(0, duration) : 0
    let time = time.isFinite ? min(max(0, time), duration) : 0
    slider.maximumValue = Float(duration)
    if !slider.isTracking { slider.value = Float(time) }
    elapsed.text = Self.format(time); total.text = Self.format(duration)
    slider.accessibilityValue = "\(Self.format(time)) of \(Self.format(duration))"
    setNeedsLayout()
  }
  static func format(_ seconds: Double) -> String {
    let value = Int(seconds.rounded(.down))
    return value >= 3600
      ? String(format: "%d:%02d:%02d", value / 3600, value / 60 % 60, value % 60)
      : String(format: "%d:%02d", value / 60, value % 60)
  }
  static let rowHeight: CGFloat = 44
  static let minimumScrubber: CGFloat = 88
  private var labelWidth: CGFloat { max(elapsed.intrinsicContentSize.width, total.intrinsicContentSize.width) }
  private var optionsWidth: CGFloat { max(44, options.intrinsicContentSize.width + 8) }
  // Play, options, AirPlay and mute; the timeline shares their row while the scrubber still fits.
  private func fitsOneRow(width: CGFloat) -> Bool {
    width - 44 * 3 - optionsWidth - labelWidth * 2 - 20 >= Self.minimumScrubber
  }
  override func sizeThatFits(_ size: CGSize) -> CGSize {
    CGSize(width: size.width, height: fitsOneRow(width: size.width) ? Self.rowHeight : Self.rowHeight * 2)
  }
  override func layoutSubviews() {
    super.layoutSubviews()
    let oneRow = bounds.height < Self.rowHeight * 2
    let buttons = CGRect(x: 0, y: oneRow ? 0 : Self.rowHeight, width: bounds.width, height: Self.rowHeight)
    playPause.frame = CGRect(x: 0, y: buttons.minY, width: 44, height: buttons.height)
    mute.frame = CGRect(x: buttons.maxX - 44, y: buttons.minY, width: 44, height: buttons.height)
    airPlay.frame = mute.frame.offsetBy(dx: -44, dy: 0)
    options.frame = CGRect(x: airPlay.frame.minX - optionsWidth, y: buttons.minY, width: optionsWidth, height: buttons.height)
    let timeline = oneRow
      ? CGRect(x: playPause.frame.maxX, y: 0, width: options.frame.minX - playPause.frame.maxX, height: Self.rowHeight)
      : CGRect(x: 16, y: 0, width: bounds.width - 32, height: Self.rowHeight)
    layoutTimeline(in: timeline)
  }
  private func layoutTimeline(in row: CGRect) {
    let label = labelWidth
    elapsed.frame = CGRect(x: row.minX, y: row.minY, width: label, height: row.height)
    total.frame = CGRect(x: row.maxX - label, y: row.minY, width: label, height: row.height)
    slider.frame = CGRect(x: elapsed.frame.maxX + 10, y: row.minY,
                          width: max(0, total.frame.minX - elapsed.frame.maxX - 20), height: row.height)
  }
}

// Reports when its menu opens and closes, so the transport can stay up underneath it.
private final class MenuButton: UIButton {
  var onMenu: ((Bool) -> Void)?
  override func contextMenuInteraction(_ interaction: UIContextMenuInteraction, willDisplayMenuFor configuration: UIContextMenuConfiguration,
                                       animator: UIContextMenuInteractionAnimating?) {
    super.contextMenuInteraction(interaction, willDisplayMenuFor: configuration, animator: animator)
    onMenu?(true)
  }
  override func contextMenuInteraction(_ interaction: UIContextMenuInteraction, willEndFor configuration: UIContextMenuConfiguration,
                                       animator: UIContextMenuInteractionAnimating?) {
    super.contextMenuInteraction(interaction, willEndFor: configuration, animator: animator)
    onMenu?(false)
  }
}

// Native counterpart of BuzzLoadingIndicator: 44pt white quarter arc, 4pt
// stroke, 10% track and a linear 500ms turn, as used by the Flutter video viewer.
private final class NativeGalleryLoadingIndicator: UIView {
  private let track = CAShapeLayer()
  private let arc = CAShapeLayer()
  private var loading = false

  override init(frame: CGRect) {
    super.init(frame: frame)
    isUserInteractionEnabled = false
    isAccessibilityElement = true
    accessibilityLabel = "Loading video"
    accessibilityTraits = .updatesFrequently
    for shape in [track, arc] {
      shape.fillColor = UIColor.clear.cgColor
      shape.lineWidth = 4
      layer.addSublayer(shape)
    }
    track.strokeColor = UIColor.white.withAlphaComponent(0.1).cgColor
    arc.strokeColor = UIColor.white.cgColor
    NotificationCenter.default.addObserver(self, selector: #selector(updateAnimation),
      name: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil)
    isHidden = true
  }
  required init?(coder: NSCoder) { nil }
  deinit { NotificationCenter.default.removeObserver(self) }
  func setLoading(_ value: Bool) {
    loading = value; isHidden = !value; updateAnimation()
  }
  override func didMoveToWindow() { super.didMoveToWindow(); updateAnimation() }
  override func layoutSubviews() {
    super.layoutSubviews()
    let center = CGPoint(x: bounds.midX, y: bounds.midY)
    let radius = max(0, (min(bounds.width, bounds.height) - 4) / 2)
    track.path = UIBezierPath(arcCenter: center, radius: radius, startAngle: 0, endAngle: 2 * .pi, clockwise: true).cgPath
    arc.path = UIBezierPath(arcCenter: center, radius: radius, startAngle: -.pi / 2, endAngle: 0, clockwise: true).cgPath
  }
  @objc private func updateAnimation() {
    guard loading, window != nil, !UIAccessibility.isReduceMotionEnabled else {
      layer.removeAnimation(forKey: "loading"); return
    }
    guard layer.animation(forKey: "loading") == nil else { return }
    let rotation = CABasicAnimation(keyPath: "transform.rotation.z")
    rotation.fromValue = 0; rotation.toValue = 2 * Double.pi
    rotation.duration = 0.5; rotation.repeatCount = .infinity
    rotation.timingFunction = CAMediaTimingFunction(name: .linear)
    layer.add(rotation, forKey: "loading")
  }
}
