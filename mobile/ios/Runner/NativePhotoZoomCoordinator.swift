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
  private var closeCenterConstraint: NSLayoutConstraint?
  private let reply = UIButton(type: .system)
  private let more = UIButton(type: .system)
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
  private var paging = false
  private var acting = false
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
    pager.dataSource = self; pager.delegate = self
    addChild(pager); view.addSubview(pager.view); pager.didMove(toParent: self)
    pager.setViewControllers([page(index)], direction: .forward, animated: false)
    loadWindow()
    configure(close, symbol: "xmark", label: videoIndices.isEmpty ? "Close photo" : "Close media", selector: #selector(closePhoto))
    configure(reply, symbol: "arrowshape.turn.up.left", label: "Reply in thread", selector: #selector(replyToMessage))
    configure(more, symbol: "ellipsis", label: "More image actions", selector: nil)
    reply.isEnabled = canReply; updateMore()
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
    let pageContainer = UIView()
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
    let controls = UIStackView(arrangedSubviews: [reply, pageContainer, more])
    controls.axis = .horizontal; controls.alignment = .center; controls.spacing = 8
    view.addSubview(close); view.addSubview(controls)
    pager.view.translatesAutoresizingMaskIntoConstraints = false
    pager.view.clipsToBounds = true
    close.translatesAutoresizingMaskIntoConstraints = false
    controls.translatesAutoresizingMaskIntoConstraints = false
    let closeCenter = close.centerYAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor)
    closeCenterConstraint = closeCenter
    NSLayoutConstraint.activate([
      // Match the existing gallery: fit photos between the toolbar bands.
      pager.view.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 56),
      pager.view.bottomAnchor.constraint(equalTo: controls.topAnchor, constant: -16),
      pager.view.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
      pager.view.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
      closeCenter,
      close.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
      close.widthAnchor.constraint(equalToConstant: 44), close.heightAnchor.constraint(equalToConstant: 44),
      reply.widthAnchor.constraint(equalToConstant: 44), reply.heightAnchor.constraint(equalToConstant: 44),
      more.widthAnchor.constraint(equalToConstant: 44), more.heightAnchor.constraint(equalToConstant: 44),
      indicator.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
      controls.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 16),
      controls.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
      controls.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -12),
    ])
  }
  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    // Ask UIKit for its compact navigation height, including the current OS metrics.
    closeCenterConstraint?.constant = navigationSizingBar.sizeThatFits(view.bounds.size).height / 2
  }
  func shouldBeginDismissal(location: CGPoint, velocity: CGVector) -> Bool {
    !paging && !finishing && !videoIndices.contains(index) && pager.view.frame.contains(location)
      && velocity.dy > abs(velocity.dx) * 1.5
      && ((pager.viewControllers?.first as? NativePhotoPage)?.isAtRestingZoom ?? false)
  }
  func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
    guard gestureRecognizer === downwardDismiss else { return true }
    let velocity = downwardDismiss.velocity(in: view)
    return shouldBeginDismissal(location: downwardDismiss.location(in: view), velocity: CGVector(dx: velocity.x, dy: velocity.y))
  }
  func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer) -> Bool {
    gestureRecognizer === downwardDismiss || otherGestureRecognizer === downwardDismiss
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
    guard !finishing, !videoIndices.contains(value), value == index, let image = UIImage(data: data) else { return }
    images[value] = image
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) where page.index == value {
      page.setImage(image)
    }
  }
  private func page(_ value: Int) -> NativePhotoPage {
    let page = NativePhotoPage(index: value, label: labels[value], image: images[value] ?? previews[value], isVideo: videoIndices.contains(value))
    page.onRetry = { [weak self] in self?.videoPath = nil; self?.loadVideo() }
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
      for page in self.pager.children.compactMap({ $0 as? NativePhotoPage }) where page.index == value {
        page.setVideo(path, autoplay: self.appeared)
      }
    }
  }
  private func stopVideos() {
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) { page.stopVideo() }
    videoPath = nil
    pending = pending.filter { !$0.key.hasPrefix("video:") }
  }
  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated); appeared = true
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
    paging = true; indicator.isEnabled = false
    for page in pager.children.compactMap({ $0 as? NativePhotoPage }) { page.pauseVideo() }
  }
  func pageViewController(_ pageViewController: UIPageViewController, didFinishAnimating finished: Bool, previousViewControllers: [UIViewController], transitionCompleted completed: Bool) {
    paging = false; indicator.isEnabled = true
    if completed, let page = pager.viewControllers?.first as? NativePhotoPage { selected(page.index) }
    else if appeared { (pager.viewControllers?.first as? NativePhotoPage)?.playVideo() }
  }
  private func updateMore() {
    let video = videoIndices.contains(index)
    more.isEnabled = canMore && !video
    more.alpha = video ? 0 : 1
    more.accessibilityElementsHidden = video
  }
  private func selected(_ value: Int) {
    stopVideos()
    index = value; indicator.currentPage = value
    updateMore()
    trimWindow()
    // Move Dart's request window before asking for newly adjacent images.
    syncSelection { [weak self] in self?.loadWindow() }
  }
  @objc private func selectPage() {
    guard !paging, !finishing else { return }
    let target = indicator.currentPage
    guard target != index else { return }
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
private final class NativePhotoPage: UIViewController, UIScrollViewDelegate {
  let index: Int
  let isVideo: Bool
  var onRetry: (() -> Void)?
  private lazy var video = AVPlayerViewController()
  private let retry = UIButton(type: .system)
  private var observation: NSKeyValueObservation?
  private let imageView = UIImageView()
  private let scroll = UIScrollView()
  private let status = UILabel()
  private var previousSize = CGSize.zero
  var image: UIImage? { imageView.image }
  var isAtRestingZoom: Bool { !isVideo && scroll.zoomScale <= scroll.minimumZoomScale + 0.01 }
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
    status.text = isVideo ? "Loading video…" : image == nil ? "Loading photo…" : nil
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
      video.view.backgroundColor = .clear
      video.view.isHidden = true
      addChild(video); view.insertSubview(video.view, belowSubview: status); video.didMove(toParent: self)
      retry.setTitle("Retry video", for: .normal)
      retry.titleLabel?.font = .preferredFont(forTextStyle: .body)
      retry.titleLabel?.adjustsFontForContentSizeCategory = true
      retry.addTarget(self, action: #selector(retryVideo), for: .touchUpInside)
      retry.isHidden = true; view.addSubview(retry)
    }
    let tap = UITapGestureRecognizer(target: self, action: #selector(toggleZoom)); tap.numberOfTapsRequired = 2; if !isVideo { scroll.addGestureRecognizer(tap) }
  }
  func setImage(_ image: UIImage?) {
    imageView.image = image
    if !isVideo { status.text = image == nil ? "Image unavailable" : nil }
  }
  func showVideoLoading() { guard isVideo else { return }; status.text = "Loading video…"; retry.isHidden = true }
  func setVideo(_ path: String?, autoplay: Bool) {
    guard isVideo else { return }
    loadViewIfNeeded()
    stopVideo()
    guard let path, FileManager.default.fileExists(atPath: path) else { videoFailed(); return }
    let item = AVPlayerItem(url: URL(fileURLWithPath: path))
    let player = AVPlayer(playerItem: item)
    video.player = player
    observation = item.observe(\.status, options: [.initial, .new]) { [weak self, weak item] _, _ in
      DispatchQueue.main.async {
        guard let self, let item, self.video.player?.currentItem === item else { return }
        if item.status == .failed { self.videoFailed() }
        else if item.status == .readyToPlay { self.status.text = nil; self.video.view.isHidden = false; self.scroll.isHidden = true }
      }
    }
    if autoplay { player.play() }
  }
  func playVideo() { if isVideo { video.player?.play() } }
  func pauseVideo() { if isVideo { video.player?.pause() } }
  func stopVideo() {
    guard isVideo else { return }
    observation = nil
    video.player?.pause(); video.player?.replaceCurrentItem(with: nil); video.player = nil
    if isViewLoaded { video.view.isHidden = true; scroll.isHidden = false }
  }
  private func videoFailed() {
    stopVideo(); status.text = "Video unavailable"; retry.isHidden = false
    UIAccessibility.post(notification: .announcement, argument: status.text)
  }
  @objc private func retryVideo() { showVideoLoading(); onRetry?() }
  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews(); scroll.frame = view.bounds; status.frame = view.bounds.insetBy(dx: 24, dy: 100)
    if isVideo {
      video.view.frame = view.bounds
      retry.frame = CGRect(x: 24, y: view.bounds.midY + 32, width: max(0, view.bounds.width - 48), height: 48)
    }
    guard previousSize != scroll.bounds.size else { return }; previousSize = scroll.bounds.size
    scroll.zoomScale = 1; imageView.frame = scroll.bounds; scroll.contentSize = scroll.bounds.size
  }
  func viewForZooming(in scrollView: UIScrollView) -> UIView? { imageView }
  @objc private func toggleZoom() { scroll.setZoomScale(scroll.zoomScale > 1 ? 1 : 2.5, animated: !UIAccessibility.isReduceMotionEnabled) }
}
