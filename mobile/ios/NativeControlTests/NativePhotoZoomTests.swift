// Concatenated with the production source by test-mobile-photo-zoom.sh so this
// fixture can exercise the private controller without widening its app API.
@main
struct NativePhotoZoomTests {
  @MainActor static func main() {
    if #available(iOS 18.0, *) {
      let data = UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2)).pngData { context in
        UIColor.red.setFill(); context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
      }
      let viewer = NativePhotoZoomViewController(
        image: UIImage(data: data)!, labels: (0..<100).map { "Photo \($0)" },
        initialIndex: 0, canReply: false, canMore: false, canDelete: false
      )
      var requests: [(String, Int, FlutterResult)] = []
      viewer.request = { method, index, _, completion in
        if method == "select" { completion(nil) }
        else { requests.append((method, index, completion)) }
      }
      viewer.loadViewIfNeeded()
      viewer.view.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
      viewer.view.layoutIfNeeded()
      precondition(!requests.isEmpty && requests.allSatisfy { $0.1 <= 2 }, "Initial requests must stay adjacent")
      let pager = viewer.children.compactMap { $0 as? UIPageViewController }.first!
      let location = CGPoint(x: 195, y: 400)
      precondition(viewer.shouldBeginDismissal(location: location, velocity: CGVector(dx: 0, dy: 300)), "Resting photos dismiss downward")
      precondition(!viewer.shouldBeginDismissal(location: location, velocity: CGVector(dx: 300, dy: 10)), "Horizontal swipes must page")
      let scroll = descendants(pager.viewControllers!.first!.view).compactMap { $0 as? UIScrollView }.first!
      scroll.zoomScale = 2
      precondition(!viewer.shouldBeginDismissal(location: location, velocity: CGVector(dx: 0, dy: 300)), "Zoomed photos must pan instead of dismiss")
      scroll.zoomScale = 1

      let indicator = descendants(viewer.view).compactMap { $0 as? UIPageControl }.first!
      let stale = requests
      requests.removeAll()
      indicator.currentPage = 50; sendValueChanged(indicator)
      precondition(!requests.isEmpty && requests.allSatisfy { (48...52).contains($0.1) }, "Scrubbing must move the request window")
      let page = pager.viewControllers!.first!
      page.loadViewIfNeeded()
      let photo = descendants(page.view).compactMap { $0 as? UIImageView }.first!
      for request in stale { request.2(FlutterStandardTypedData(bytes: data)) }
      precondition(photo.image == nil, "Late old-window responses cannot paint the selected photo")
      let current = requests
      requests.removeAll()
      let blueData = UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2)).pngData { context in
        UIColor.blue.setFill(); context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
      }
      let previous = photo.image
      viewer.updateFrame(blueData, at: 0)
      precondition(photo.image === previous, "Offscreen animation frames cannot paint the selected photo")
      viewer.updateFrame(blueData, at: 50)
      precondition(photo.image !== previous, "Continuing animation frames must update the selected photo")
      let updated = photo.image
      precondition(current.contains { $0.0 == "image" && $0.1 == 50 }, "Hold the selected initial-image reply until after a continuing frame")
      for request in current { request.2(FlutterStandardTypedData(bytes: data)) }
      precondition(photo.image === updated, "Delayed initial image and preview replies must not rewind a continuing frame")
      viewer.updateFrame(Data([0, 1]), at: 50)
      precondition(photo.image === updated, "Malformed frames must preserve the last displayed image")
      indicator.currentPage = 0; sendValueChanged(indicator)
      precondition(requests.contains { $0.1 == 0 }, "Leaving the window evicts old decoded images")
      let closing = requests
      requests.removeAll()
      var closed = false
      viewer.onClose = { _ in closed = true }
      viewer.viewDidDisappear(false)
      for request in closing { request.2(FlutterStandardTypedData(bytes: data)) }
      precondition(closed && requests.isEmpty, "Teardown must ignore late loads and stop requesting")
      let closedPage = pager.viewControllers!.first!
      closedPage.loadViewIfNeeded()
      let closedPhoto = descendants(closedPage.view).compactMap { $0 as? UIImageView }.first!
      let closedImage = closedPhoto.image
      viewer.updateFrame(data, at: 0)
      precondition(closedPhoto.image === closedImage, "Closed viewers must ignore continuing frames")
      print("PASS: native photo window, stale responses, eviction and teardown")
      railJourney(data: data)
      videoJourney(data: data)
      textSizeJourney()
      loaderJourney()

    }
  }
  @available(iOS 18.0, *)
  @MainActor static func railJourney(data: Data) {
    let viewer = NativePhotoZoomViewController(image: UIImage(data: data)!,
      labels: ["One", "Two"], initialIndex: 0, canReply: true, canMore: true, canDelete: false)
    viewer.loadViewIfNeeded()
    viewer.view.frame = CGRect(x: 0, y: 0, width: 951, height: 669)
    let pager = viewer.children.compactMap { $0 as? UIPageViewController }.first!
    let buttons = ["Close photo", "Reply in thread", "More image actions"].map { label in
      descendants(viewer.view).compactMap { $0 as? UIButton }.first { $0.accessibilityLabel == label }!
    }
    let indicator = descendants(viewer.view).compactMap { $0 as? UIPageControl }.first!
    let pageContainer = indicator.superview!.superview!.superview!
    let page = pager.viewControllers!.first!
    for left in [false, true] {
      let centerX: CGFloat = left ? 48 : 903
      viewer.sourceRailGeometry = (viewer.view.bounds.size, centerX)
      viewer.layoutGallery(insets: UIEdgeInsets(top: 0, left: left ? 76 : 0, bottom: 0, right: left ? 0 : 76))
      precondition(pager.view.frame == viewer.view.bounds, "Native gallery must span the full Duo window")
      pageContainer.layoutIfNeeded()
      let indicatorFrame = indicator.convert(indicator.bounds, to: viewer.view)
      precondition(abs(indicatorFrame.midX - pager.view.frame.midX) < 0.5, "Photo indicator must share the full-width photo center on either rail side")
      for (slot, button) in buttons.enumerated() {
        precondition(left ? button.center.x <= 76 : button.center.x >= 875, "Every action is centered inside the rail")
        precondition(button.center.x == (left ? 54 : 897), "Native actions must match the channel rail's inward-aligned centerline")
        precondition(button.bounds.width >= 44 && button.bounds.height >= 44, "Retain accessible hit targets")
        if slot > 0 { precondition(button.frame.minY > buttons[slot - 1].frame.maxY, "Close, Reply, More must not overlap") }
      }
    }
    precondition(buttons[1].frame.minY > 669 - 140 && buttons[2].frame.maxY == 669 - 12, "Reply and More must anchor to the bottom of the rail")
    viewer.view.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
    viewer.layoutGallery(insets: UIEdgeInsets(top: 59, left: 0, bottom: 34, right: 0))
    precondition(buttons[0].frame.maxY < buttons[1].frame.minY, "Ordinary phones retain top Close and bottom actions")
    pager.view.layoutIfNeeded(); page.view.layoutIfNeeded()
    let photo = descendants(page.view).compactMap { $0 as? UIScrollView }.first!.convert(page.view.bounds, to: viewer.view)
    let band = descendants(page.view).compactMap { $0 as? UIScrollView }.first!.frame
    precondition(pager.view.frame == viewer.view.bounds, "One pager frame for every page, so paging never resizes a page")
    precondition(band.minY == 59 + 56 && band.maxY < buttons[1].frame.minY && photo.width > 0, "Photos keep clear of the actions inside their page")
    precondition(pager.viewControllers!.first === page, "Folding must retain the selected native page")
    print("PASS: native gallery rail sides, full viewport, targets and compact transition")
  }
  @available(iOS 18.0, *)
  @MainActor static func videoJourney(data: Data) {
    let path = CommandLine.arguments[1]
    let viewer = NativePhotoZoomViewController(image: UIImage(data: data)!,
      labels: ["Clip", "Photo", "Second clip"], initialIndex: 0,
      canReply: true, canMore: true, canDelete: false, videoIndices: [0, 2])
    let feedback = GalleryFeedbackSpy()
    viewer.selectionFeedback = feedback
    var requested: [Int] = []
    var lateVideo: FlutterResult?
    viewer.request = { method, index, _, completion in
      if method == "video" {
        requested.append(index)
        if index == 0 { completion(path) } else { lateVideo = completion }
      } else { completion(method == "select" ? nil : FlutterStandardTypedData(bytes: data)) }
    }
    viewer.loadViewIfNeeded()
    viewer.view.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
    viewer.view.layoutIfNeeded()
    viewer.viewDidAppear(false)
    let pager = viewer.children.compactMap { $0 as? UIPageViewController }.first!
    let indicator = descendants(viewer.view).compactMap { $0 as? UIPageControl }.first!
    precondition(viewer.shouldBeginDismissal(location: CGPoint(x: 195, y: 400), velocity: CGVector(dx: 0, dy: 300)), "Videos must dismiss downward too")
    precondition(!viewer.shouldBeginDismissal(location: CGPoint(x: 195, y: 400), velocity: CGVector(dx: 300, dy: 0)), "Video horizontal scrubbing must not dismiss")
    let first = pager.viewControllers!.first!
    first.loadViewIfNeeded()
    let posterView = descendants(first.view).compactMap { $0 as? UIImageView }.first!
    let oldPoster = posterView.image
    viewer.updateFrame(data, at: 0)
    precondition(posterView.image !== oldPoster, "A late poster updates the selected video")
    let selectedPoster = posterView.image
    viewer.updateFrame(data, at: 2)
    precondition(posterView.image === selectedPoster, "A stale poster cannot replace the selected video")
    let playerController = first.children.compactMap { $0 as? AVPlayerViewController }.first!
    let player = playerController.player!
    let item = player.currentItem!
    let firstSpinner = descendants(first.view).compactMap { $0 as? NativeGalleryLoadingIndicator }.first!
    let deadline = Date().addingTimeInterval(5)
    while (item.status == .unknown || !firstSpinner.isHidden) && Date() < deadline {
      RunLoop.current.run(until: Date().addingTimeInterval(0.01))
    }
    precondition(item.status == .readyToPlay, "Real MP4 fixture must become playable")
    precondition(firstSpinner.isHidden, "Playable video removes the loader")
    precondition(requested == [0], "Only the selected video may download")
    let videoButtons = ["Close media", "Share video", "Reply in thread"].map { label in
      descendants(viewer.view).compactMap { $0 as? UIButton }.first { $0.accessibilityLabel == label }!
    }
    checkVideoLayout(viewer: viewer, pager: pager, page: first, buttons: videoButtons)
    checkVideoChrome(viewer: viewer, page: first as! NativePhotoPage, buttons: videoButtons)
    checkMenuHoldsFade(viewer: viewer, page: first as! NativePhotoPage)
    checkSpeedAndRoutes(page: first as! NativePhotoPage, player: player)
    checkPlayback(page: first as! NativePhotoPage, player: player)
    precondition(feedback.clicks == 0, "Opening must not trigger selection haptics")
    viewer.pageViewController(pager, willTransitionTo: [first])
    precondition(feedback.clicks == 1, "Swipe feedback fires before the page changes")
    viewer.pageViewController(pager, willTransitionTo: [first])
    precondition(feedback.clicks == 1, "Repeated start callbacks do not duplicate swipe feedback")
    viewer.pageViewController(pager, didFinishAnimating: true, previousViewControllers: [first], transitionCompleted: false)
    precondition(feedback.clicks == 1, "Cancellation does not add a second haptic")
    indicator.currentPage = 2; sendValueChanged(indicator)
    precondition(player.currentItem == nil && playerController.player == nil, "Paging releases the old player before Dart deletes its file")
    precondition(requested == [0, 2], "Paging requests the selected video")
    precondition(feedback.clicks == 2, "Changing to a video triggers one selection click")
    let loadingPage = pager.viewControllers!.first!
    loadingPage.loadViewIfNeeded()
    let spinner = descendants(loadingPage.view).compactMap { $0 as? NativeGalleryLoadingIndicator }.first!
    precondition(!spinner.isHidden && spinner.accessibilityLabel == "Loading video", "Show the shared arc treatment while loading")
    precondition(!descendants(loadingPage.view).compactMap { $0 as? UILabel }.contains { $0.text == "Loading video…" }, "No visible loading copy")
    lateVideo?(nil)
    precondition(spinner.isHidden, "Failure stops the loader")
    let retry = descendants(loadingPage.view).compactMap { $0 as? UIButton }.first { $0.currentTitle == "Retry video" }!
    precondition(!retry.isHidden, "Failure retains retry")
    for target in retry.allTargets {
      for action in retry.actions(forTarget: target, forControlEvent: .touchUpInside) ?? [] {
        _ = (target as AnyObject).perform(NSSelectorFromString(action), with: retry)
      }
    }
    precondition(!spinner.isHidden && requested == [0, 2, 2], "Retry restarts the loader")
    indicator.currentPage = 0; sendValueChanged(indicator)
    lateVideo?(path)
    let current = pager.viewControllers!.first!
    current.loadViewIfNeeded()
    let active = current.children.compactMap { $0 as? AVPlayerViewController }.first!
    precondition(active.player != nil, "Returning to a video restores its player")
    precondition(spinner.isHidden, "Leaving a loading video stops its loader")
    let phone = UIEdgeInsets(top: 59, left: 0, bottom: 34, right: 0)
    viewer.layoutGallery(insets: phone)
    let pagerWhileVideo = pager.view.frame
    let next = viewer.pageViewController(pager, viewControllerAfter: current)!
    viewer.pageViewController(pager, willTransitionTo: [next])
    next.view.frame = pager.view.bounds; next.view.layoutIfNeeded()
    let photoWhileSwiping = photoFrame(next)
    precondition(feedback.clicks == 4, "Next swipe fires before the new photo is selected")
    pager.setViewControllers([next], direction: .forward, animated: false)
    viewer.pageViewController(pager, didFinishAnimating: true, previousViewControllers: [current], transitionCompleted: true)
    precondition(feedback.clicks == 4, "Completed swipe does not add a delayed haptic")
    viewer.layoutGallery(insets: phone); pager.view.setNeedsLayout(); pager.view.layoutIfNeeded(); next.view.layoutIfNeeded()
    precondition(pager.view.frame == pagerWhileVideo, "Settling on a photo keeps the pager frame it had on the video")
    precondition(photoFrame(next) == photoWhileSwiping && photoWhileSwiping.minY == 59 + 56, "A photo keeps its inset frame from mid-swipe to settled")
    viewer.pageViewController(pager, didFinishAnimating: true, previousViewControllers: [current], transitionCompleted: true)
    precondition(feedback.clicks == 4, "Repeated callbacks for the same page do not duplicate haptics")
    viewer.viewDidDisappear(false)
    precondition(active.player == nil, "Closing releases the player before returning to Flutter")
    print("PASS: native video decoding, selected-only loading, paging and teardown")
  }
  @available(iOS 18.0, *)
  @MainActor fileprivate static func checkVideoLayout(viewer: NativePhotoZoomViewController, pager: UIPageViewController,
                                                      page first: UIViewController, buttons videoButtons: [UIButton]) {
    let transport = descendants(first.view).compactMap { $0 as? NativeVideoTransport }.first!
    let slider = transport.slider
    for (size, insets) in [(CGSize(width: 390, height: 844), UIEdgeInsets(top: 59, left: 0, bottom: 34, right: 0)),
                           (CGSize(width: 844, height: 390), UIEdgeInsets(top: 0, left: 59, bottom: 21, right: 59))] {
      viewer.view.frame = CGRect(origin: .zero, size: size)
      viewer.layoutGallery(insets: insets)
      precondition(pager.view.frame == viewer.view.bounds, "Video fills the screen in portrait and landscape")
      for button in videoButtons {
        precondition(viewer.view.bounds.inset(by: insets).contains(button.frame), "Overlaid video actions stay in the safe area")
        precondition(button.frame.maxY < viewer.view.bounds.midY, "Video actions sit in the top row, clear of the transport")
      }
      pager.view.layoutIfNeeded(); first.view.layoutIfNeeded()
      let bar = transport.convert(transport.bounds, to: viewer.view)
      precondition(viewer.view.bounds.inset(by: insets).contains(bar) && bar.minY > viewer.view.bounds.midY, "The transport sits at the bottom of the safe area")
      precondition(bar.height >= 44 && transport.playPause.bounds.width >= 44 && transport.mute.bounds.width >= 44, "Transport keeps 44pt targets")
      precondition(slider.frame.width > 100, "The scrubber has room in portrait and landscape")
      for (slot, button) in videoButtons.enumerated() where slot > 0 {
        precondition(!button.frame.intersects(videoButtons[slot - 1].frame), "Video actions must not overlap")
      }
    }
    viewer.view.frame = CGRect(x: 0, y: 0, width: 951, height: 669)
    viewer.layoutGallery(insets: UIEdgeInsets(top: 0, left: 0, bottom: 0, right: 76))
    precondition(pager.view.frame == viewer.view.bounds, "Video spans the full Duo window beside the rail")
    precondition(videoButtons.allSatisfy { $0.center.x >= 875 }, "Duo video actions stay in the rail")
    first.view.layoutIfNeeded()
    precondition(transport.convert(transport.bounds, to: viewer.view).maxX <= 875, "The Duo transport stays clear of the rail")
    viewer.view.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
    viewer.layoutGallery(insets: .zero)
    precondition(videoButtons[1].isEnabled && videoButtons[1].alpha == 1, "A downloaded video can be shared")
  }
  @available(iOS 18.0, *)
  @MainActor fileprivate static func checkVideoChrome(viewer: NativePhotoZoomViewController, page: NativePhotoPage, buttons videoButtons: [UIButton]) {
    let transport = descendants(page.view).compactMap { $0 as? NativeVideoTransport }.first!
    let chrome: [UIView] = videoButtons + [transport]
    viewer.setChrome(hidden: true)
    precondition(chrome.allSatisfy { !isShown($0) && $0.alpha == 0 }, "Tapping the video hides its actions and transport together")
    viewer.setChrome(hidden: false)
    precondition(chrome.allSatisfy(isShown), "Tapping again restores them")
    viewer.setChrome(hidden: true)
    viewer.revealForVoiceOver(running: false)
    precondition(!isShown(transport), "Turning VoiceOver off leaves the chrome alone")
    viewer.revealForVoiceOver(running: true)
    precondition(chrome.allSatisfy(isShown), "Turning VoiceOver on after the fade reveals the actions and transport")
  }
  @available(iOS 18.0, *)
  @MainActor fileprivate static func checkMenuHoldsFade(viewer: NativePhotoZoomViewController, page: NativePhotoPage) {
    let transport = descendants(page.view).compactMap { $0 as? NativeVideoTransport }.first!
    // An open speed menu or AirPlay picker holds the 3s fade until it closes.
    let player = page.children.compactMap { $0 as? AVPlayerViewController }.first!.player!
    page.menuChanged(open: true)
    player.seek(to: .zero); player.play(); waitFor(transport.playPause, label: "Pause")
    precondition(page.isPlaying, "The fixture video plays")
    viewer.setChrome(hidden: false)
    precondition(!viewer.isChromeFadeScheduled, "The transport stays up under an open menu")
    page.menuChanged(open: false)
    precondition(viewer.isChromeFadeScheduled, "The fade resumes once the menu closes")
    player.pause(); waitFor(transport.playPause, label: "Play")
    precondition(transport.alpha == 1, "Pausing brings the transport back")
  }
  @available(iOS 18.0, *)
  @MainActor fileprivate static func checkSpeedAndRoutes(page: NativePhotoPage, player: AVPlayer) {
    let transport = descendants(page.view).compactMap { $0 as? NativeVideoTransport }.first!
    let menu = transport.options.menu!
    precondition(menu.title == "Playback Speed" && menu.children.count == 6, "The options menu offers AVKit's playback speeds")
    precondition(transport.options.accessibilityLabel == "Playback options" && transport.options.title(for: .normal) == "1×", "Speed shows like the Flutter viewer")
    page.setSpeed(1.5)
    precondition(player.defaultRate == 1.5 && transport.options.title(for: .normal) == "1.5×", "Choosing a speed sets the player's rate")
    page.setSpeed(1)
    precondition(NativeVideoTransport.speed(0.75) == "0.75×" && NativeVideoTransport.speed(2) == "2×", "Speed labels drop trailing zeros")
    precondition(transport.airPlay.superview === transport && transport.airPlay.bounds.width >= 44, "AirPlay stays available")
  }
  @available(iOS 18.0, *)
  @MainActor fileprivate static func checkPlayback(page: NativePhotoPage, player: AVPlayer) {
    let transport = descendants(page.view).compactMap { $0 as? NativeVideoTransport }.first!
    precondition(!page.children.compactMap { $0 as? AVPlayerViewController }.first!.showsPlaybackControls,
                 "AVKit's self-toggling controls stay off so they can't fall out of step")
    precondition(!transport.isHidden && transport.slider.accessibilityLabel == "Video position", "A playable video shows the gallery's own transport")
    player.pause(); waitFor(transport.playPause, label: "Play")
    precondition(transport.playPause.accessibilityLabel == "Play", "A paused video offers Play")
    send(.touchUpInside, to: transport.playPause); waitFor(transport.playPause, label: "Pause")
    precondition(player.rate > 0 && transport.playPause.accessibilityLabel == "Pause", "Play starts the video")
    send(.touchUpInside, to: transport.playPause); waitFor(transport.playPause, label: "Play")
    precondition(player.rate == 0 && transport.playPause.accessibilityLabel == "Play", "Pause stops it")
    send(.touchUpInside, to: transport.mute)
    precondition(player.isMuted && transport.mute.accessibilityLabel == "Unmute", "Mute toggles the player")
    send(.touchUpInside, to: transport.mute)
    precondition(NativeVideoTransport.format(65) == "1:05" && NativeVideoTransport.format(3725) == "1:02:05", "Times read like the Flutter viewer")
  }
  // At the largest text sizes the timeline moves to its own row instead of losing the scrubber.
  @MainActor static func textSizeJourney() {
    let host = UIView(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
    let transport = NativeVideoTransport()
    host.addSubview(transport)
    for (category, duration, rows) in [(UIContentSizeCategory.large, 30.0, 1), (.large, 4000, 2),
                                       (.accessibilityExtraExtraExtraLarge, 30, 2), (.accessibilityExtraExtraExtraLarge, 4000, 2)] {
      host.traitOverrides.preferredContentSizeCategory = category
      host.updateTraitsIfNeeded(); transport.updateTraitsIfNeeded()
      transport.setSpeed(1.25); transport.update(time: duration - 1, duration: duration)
      let size = transport.sizeThatFits(CGSize(width: 358, height: 0))
      transport.frame = CGRect(origin: .zero, size: size); transport.layoutIfNeeded()
      precondition(transport.traitCollection.preferredContentSizeCategory <= .accessibilityMedium, "Transport text is capped")
      precondition(size.height == NativeVideoTransport.rowHeight * CGFloat(rows), "\(category) \(duration)s uses \(rows) row(s)")
      precondition(transport.slider.frame.width >= NativeVideoTransport.minimumScrubber, "The scrubber keeps room to seek")
      let parts = [transport.playPause, transport.options, transport.airPlay, transport.mute, transport.slider]
        + descendants(transport).compactMap { $0 as? UILabel }.filter { $0.superview === transport }
      for (slot, part) in parts.enumerated() {
        precondition(transport.bounds.contains(part.frame), "Transport parts stay inside it")
        precondition(parts[..<slot].allSatisfy { !$0.frame.intersects(part.frame) }, "Transport parts never overlap")
      }
    }
    print("PASS: native video transport fits every text size")
  }
  @MainActor static func isShown(_ view: UIView) -> Bool {
    view.alpha == 1 && view.isUserInteractionEnabled && !view.accessibilityElementsHidden
  }
  @MainActor static func photoFrame(_ page: UIViewController) -> CGRect {
    descendants(page.view).compactMap { $0 as? UIScrollView }.first!.frame
  }
  @MainActor static func waitFor(_ button: UIButton, label: String) {
    let deadline = Date().addingTimeInterval(2)
    while button.accessibilityLabel != label && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
  }
  @MainActor static func loaderJourney() {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
    let root = UIViewController(); window.rootViewController = root
    let loader = NativeGalleryLoadingIndicator(frame: CGRect(x: 28, y: 28, width: 44, height: 44))
    root.view.addSubview(loader); window.isHidden = false
    loader.setLoading(true); loader.layoutIfNeeded()
    if !UIAccessibility.isReduceMotionEnabled {
      precondition(loader.layer.animation(forKey: "loading")?.duration == 0.5, "Match the shared spinner cadence")
    }
    loader.setLoading(false)
    precondition(loader.layer.animation(forKey: "loading") == nil && loader.isHidden)
    loader.setLoading(true); loader.removeFromSuperview()
    precondition(loader.layer.animation(forKey: "loading") == nil, "Detached loaders must stop animating")
    window.isHidden = true
  }
  @MainActor static func sendValueChanged(_ control: UIControl) { send(.valueChanged, to: control) }
  @MainActor static func send(_ event: UIControl.Event, to control: UIControl) {
    // A command-line UIKit fixture has no UIApplication action dispatcher.
    for target in control.allTargets {
      for action in control.actions(forTarget: target, forControlEvent: event) ?? [] {
        _ = (target as AnyObject).perform(NSSelectorFromString(action), with: control)
      }
    }
  }
  @MainActor static func descendants(_ view: UIView) -> [UIView] {
    [view] + view.subviews.flatMap { descendants($0) }
  }
}

@MainActor
private final class GalleryFeedbackSpy: UISelectionFeedbackGenerator {
  var clicks = 0
  override func selectionChanged() { clicks += 1 }
}
