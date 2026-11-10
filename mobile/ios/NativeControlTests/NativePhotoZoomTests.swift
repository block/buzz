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

    }
  }
  @available(iOS 18.0, *)
  @MainActor static func videoJourney(data: Data) {
    let path = CommandLine.arguments[1]
    let viewer = NativePhotoZoomViewController(image: UIImage(data: data)!,
      labels: ["Clip", "Photo", "Second clip"], initialIndex: 0,
      canReply: true, canMore: true, canDelete: false, videoIndices: [0, 2])
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
    let first = pager.viewControllers!.first!
    first.loadViewIfNeeded()
    let playerController = first.children.compactMap { $0 as? AVPlayerViewController }.first!
    let player = playerController.player!
    let item = player.currentItem!
    let deadline = Date().addingTimeInterval(5)
    while item.status == .unknown && Date() < deadline {
      RunLoop.current.run(until: Date().addingTimeInterval(0.01))
    }
    precondition(item.status == .readyToPlay, "Real MP4 fixture must become playable")
    precondition(requested == [0], "Only the selected video may download")
    indicator.currentPage = 2; sendValueChanged(indicator)
    precondition(player.currentItem == nil && playerController.player == nil, "Paging releases the old player before Dart deletes its file")
    precondition(requested == [0, 2], "Paging requests the selected video")
    indicator.currentPage = 0; sendValueChanged(indicator)
    lateVideo?(path)
    let current = pager.viewControllers!.first!
    current.loadViewIfNeeded()
    let active = current.children.compactMap { $0 as? AVPlayerViewController }.first!
    precondition(active.player != nil, "Returning to a video restores its player")
    viewer.viewDidDisappear(false)
    precondition(active.player == nil, "Closing releases the player before returning to Flutter")
    print("PASS: native video decoding, selected-only loading, paging and teardown")
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
