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
      videoJourney(data: data)
      loaderJourney()

    }
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
    let next = viewer.pageViewController(pager, viewControllerAfter: current)!
    viewer.pageViewController(pager, willTransitionTo: [next])
    precondition(feedback.clicks == 4, "Next swipe fires before the new photo is selected")
    pager.setViewControllers([next], direction: .forward, animated: false)
    viewer.pageViewController(pager, didFinishAnimating: true, previousViewControllers: [current], transitionCompleted: true)
    precondition(feedback.clicks == 4, "Completed swipe does not add a delayed haptic")
    viewer.pageViewController(pager, didFinishAnimating: true, previousViewControllers: [current], transitionCompleted: true)
    precondition(feedback.clicks == 4, "Repeated callbacks for the same page do not duplicate haptics")
    viewer.viewDidDisappear(false)
    precondition(active.player == nil, "Closing releases the player before returning to Flutter")
    print("PASS: native video decoding, selected-only loading, paging and teardown")
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
  @MainActor static func sendValueChanged(_ control: UIControl) {
    // A command-line UIKit fixture has no UIApplication action dispatcher.
    for target in control.allTargets {
      for action in control.actions(forTarget: target, forControlEvent: .valueChanged) ?? [] {
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
