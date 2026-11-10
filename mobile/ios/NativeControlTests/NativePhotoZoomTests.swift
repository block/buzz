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
    }
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
