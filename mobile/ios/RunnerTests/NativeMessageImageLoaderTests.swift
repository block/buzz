import UIKit
import XCTest
@testable import Buzz

final class NativeMessageImageLoaderTests: XCTestCase {
  private func loader(limit: Int = 1) -> NativeMessageImageLoader {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [MessageImageProtocol.self]
    return NativeMessageImageLoader(configuration: configuration, maximumConcurrent: limit)
  }

  @MainActor func testRejectsDeclaredOversizeWithoutFinishingBody() async {
    let done = expectation(description: "rejected")
    MessageImageProtocol.start = { transport in
      transport.client?.urlProtocol(transport, didReceive: HTTPURLResponse(
        url: transport.request.url!, statusCode: 200, httpVersion: nil,
        headerFields: ["Content-Length": "\(NativeMessageImageLoader.maximumBytes + 1)", "Content-Type": "image/png"]
      )!, cacheStoragePolicy: .notAllowed)
      // Let URLProtocol deliver its headers without finishing the body. A
      // completion-handler byte check would hang here, rather than reject.
      transport.client?.urlProtocol(transport, didLoad: Data(count: 1024))
    }
    let loader = loader()
    _ = loader.load(URLRequest(url: URL(string: "https://example.com/declared")!)) { image in
      XCTAssertNil(image)
      done.fulfill()
    }
    await fulfillment(of: [done], timeout: 2)
  }

  @MainActor func testRejectsChunkedOversizeWithoutWaitingForCompletion() async {
    let done = expectation(description: "stream cancelled")
    MessageImageProtocol.start = { transport in
      transport.client?.urlProtocol(transport, didReceive: HTTPURLResponse(
        url: transport.request.url!, statusCode: 200, httpVersion: nil, headerFields: nil
      )!, cacheStoragePolicy: .notAllowed)
      transport.client?.urlProtocol(transport, didLoad: Data(count: NativeMessageImageLoader.maximumBytes))
      transport.client?.urlProtocol(transport, didLoad: Data([0]))
      // No finish callback: the streaming byte guard must terminate the load.
    }
    let loader = loader()
    _ = loader.load(URLRequest(url: URL(string: "https://example.com/chunked")!)) { image in
      XCTAssertNil(image)
      done.fulfill()
    }
    await fulfillment(of: [done], timeout: 2)
  }

  @MainActor func testDeduplicatesAndCancellationReleasesAdmission() async {
    let first = expectation(description: "first admitted")
    let second = expectation(description: "second admitted after cancellation")
    var started = [String]()
    MessageImageProtocol.start = { transport in
      DispatchQueue.main.async {
        started.append(transport.request.url!.path)
        if started.count == 1 { first.fulfill() } else { second.fulfill() }
      }
    }
    let loader = loader()
    let request = URLRequest(url: URL(string: "https://example.com/shared")!)
    let cancelFirst = loader.load(request) { _ in XCTFail("cancelled subscriber called") }
    let cancelDuplicate = loader.load(request) { _ in XCTFail("cancelled duplicate called") }
    let cancelQueued = loader.load(URLRequest(url: URL(string: "https://example.com/queued")!)) { _ in
      XCTFail("cancelled queued subscriber called")
    }
    let cancelNext = loader.load(URLRequest(url: URL(string: "https://example.com/next")!)) { _ in }
    await fulfillment(of: [first], timeout: 2)
    XCTAssertEqual(started, ["/shared"])
    cancelFirst()
    cancelQueued()
    // One subscriber remains: cancelling the first must not cancel its peer.
    let turn = expectation(description: "cancellation processed")
    DispatchQueue.main.async { turn.fulfill() }
    await fulfillment(of: [turn], timeout: 2)
    XCTAssertEqual(started, ["/shared"])
    cancelDuplicate()
    await fulfillment(of: [second], timeout: 2)
    XCTAssertEqual(started, ["/shared", "/next"])
    cancelNext()
  }

  @MainActor func testCachesDecodedImages() async {
    let done = expectation(description: "decoded")
    let image = UIGraphicsImageRenderer(size: CGSize(width: 1, height: 1)).pngData { _ in }
    var requests = 0
    MessageImageProtocol.start = { transport in
      requests += 1
      transport.client?.urlProtocol(transport, didReceive: HTTPURLResponse(
        url: transport.request.url!, statusCode: 200, httpVersion: nil, headerFields: nil
      )!, cacheStoragePolicy: .notAllowed)
      transport.client?.urlProtocol(transport, didLoad: image)
      transport.client?.urlProtocolDidFinishLoading(transport)
    }
    let loader = loader()
    let request = URLRequest(url: URL(string: "https://example.com/cached")!)
    _ = loader.load(request) { image in XCTAssertNotNil(image); done.fulfill() }
    await fulfillment(of: [done], timeout: 2)
    var cached = false
    _ = loader.load(request) { image in cached = image != nil }
    XCTAssertTrue(cached)
    XCTAssertEqual(requests, 1)
  }
}

private final class MessageImageProtocol: URLProtocol {
  static var start: ((MessageImageProtocol) -> Void)?
  override class func canInit(with request: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
  override func startLoading() { Self.start?(self) }
  override func stopLoading() {}
}


final class NativeReactionDetailsTests: XCTestCase {
  @MainActor func testInlineEmojiAvatarRendersEmojiAndBackground() {
    let controller = NativeReactionDetailsViewController(data: [
      "reactions": [["emoji": "❤️", "label": "Heart", "users": ["person"]]],
      "profiles": ["person": ["name": "Kenny", "avatarEmoji": "😊", "avatarColor": NSNumber(value: UInt32(0xFFFF6B9A))]],
    ])
    let cell = controller.tableView(UITableView(), cellForRowAt: IndexPath(row: 0, section: 0))
    let row = cell.contentView.subviews.compactMap { $0 as? UIStackView }.first!
    let avatar = row.arrangedSubviews[0] as! NativeMessageGlyph
    XCTAssertEqual(avatar.subviews.compactMap { $0 as? UILabel }.first?.text, "😊")
    XCTAssertEqual(avatar.backgroundColor, UIColor(red: 1, green: 107.0 / 255, blue: 154.0 / 255, alpha: 1))
    controller.updateProfiles(["person": ["name": "Kenny", "avatarEmoji": "🎉", "avatarColor": NSNumber(value: UInt32(0xFFFFE75C))]])
    let updated = controller.tableView(UITableView(), cellForRowAt: IndexPath(row: 0, section: 0))
    let updatedRow = updated.contentView.subviews.compactMap { $0 as? UIStackView }.first!
    XCTAssertEqual(updatedRow.arrangedSubviews[0].subviews.compactMap { $0 as? UILabel }.first?.text, "🎉")
  }

  @MainActor func testFiltersPreserveRowsAndUpdatedAccessibilityNames() {
    let controller = NativeReactionDetailsViewController(data: [
      "initialEmoji": "❤️",
      "reactions": [
        ["emoji": "❤️", "label": "Heart", "count": 2, "users": ["human", "agent"]],
        ["emoji": "🔥", "label": "Fire", "count": 1, "users": ["agent"]],
      ],
      "profiles": ["human": ["name": "Honey"], "agent": ["name": "Honey (agent)"]],
    ])
    controller.loadViewIfNeeded()
    let table = UITableView()
    func labels() -> [String] {
      (0..<controller.tableView(table, numberOfRowsInSection: 0)).map {
        controller.tableView(table, cellForRowAt: IndexPath(row: $0, section: 0)).accessibilityLabel ?? ""
      }
    }
    func descendants(_ view: UIView) -> [UIView] {
      view.subviews.flatMap { [$0] + descendants($0) }
    }
    func select(_ label: String) {
      let button = descendants(controller.view).compactMap { $0 as? UIButton }
        .first { $0.accessibilityLabel == label }
      XCTAssertNotNil(button)
      button?.sendActions(for: .primaryActionTriggered)
    }
    XCTAssertEqual(labels(), ["Honey, Heart", "Honey (agent), Heart"])
    select("All 3")
    XCTAssertEqual(labels(), ["Honey, Heart", "Honey (agent), Heart", "Honey (agent), Fire"])
    select("Fire 1")
    XCTAssertEqual(labels(), ["Honey (agent), Fire"])
    controller.updateProfiles(["agent": ["name": "Helper (agent)"]])
    XCTAssertEqual(labels(), ["Helper (agent), Fire"])
    select("Heart 2")
    XCTAssertEqual(labels(), ["Honey, Heart", "Helper (agent), Heart"])
  }
}


final class NativeMessageLayoutTests: XCTestCase {
  private func descendants(_ view: UIView) -> [UIView] {
    view.subviews.flatMap { [$0] + descendants($0) }
  }

  @MainActor func testReactionFiltersFitAccessibilityText() {
    let parent = UIViewController()
    let controller = NativeReactionDetailsViewController(data: [
      "reactions": [["emoji": "❤️", "label": "Heart", "count": 12345, "users": ["a"]]],
    ])
    parent.addChild(controller)
    parent.setOverrideTraitCollection(UITraitCollection(preferredContentSizeCategory: .accessibilityExtraExtraExtraLarge), forChild: controller)
    parent.view.addSubview(controller.view)
    controller.view.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
    controller.view.setNeedsLayout()
    controller.view.layoutIfNeeded()
    let filter = descendants(controller.view).compactMap { $0 as? UIButton }
      .first { $0.accessibilityLabel == "All 12345" }
    XCTAssertNotNil(filter)
    let viewport = descendants(controller.view).compactMap { $0 as? UIScrollView }
      .first { !($0 is UITableView) }
    XCTAssertNotNil(viewport)
    let needed = filter?.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize).height ?? 0
    XCTAssertGreaterThan(needed, 48)
    XCTAssertGreaterThanOrEqual(viewport?.bounds.height ?? 0, needed)
    XCTAssertGreaterThanOrEqual(filter?.bounds.height ?? 0, needed)
  }

  @MainActor func testOversizedMenuStaysInsideShortViewportAndScrollsToLastAction() {
    let controller = NativeMessageMenuViewController(data: [
      "actions": (0..<12).map { ["id": "action\($0)", "title": "Action \($0)", "symbol": "star"] },
      "reactions": [["emoji": "❤️", "label": "Heart"]],
    ], sourceRect: CGRect(x: 16, y: 180, width: 288, height: 80), preview: UIView())
    controller.onPreviewReady = { _ in }
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
    window.rootViewController = controller
    window.makeKeyAndVisible()
    defer { window.isHidden = true }
    controller.loadViewIfNeeded()
    // Constrain the available viewport as a bottom obstruction would. The
    // production bottom anchor remains the system keyboard layout guide.
    controller.additionalSafeAreaInsets.bottom = 200
    controller.view.setNeedsLayout()
    controller.view.layoutIfNeeded()
    let scroll = controller.view.subviews.compactMap { $0 as? UIScrollView }.first!
    scroll.layoutIfNeeded()
    XCTAssertTrue(scroll.clipsToBounds)
    XCTAssertGreaterThan(scroll.contentSize.height, scroll.bounds.height)
    XCTAssertLessThanOrEqual(scroll.frame.maxY, controller.view.keyboardLayoutGuide.layoutFrame.minY)
    let content = scroll.subviews.compactMap { $0 as? UIStackView }.first!
    XCTAssertEqual(content.frame.minX, 16, accuracy: 0.5)
    XCTAssertGreaterThanOrEqual(content.frame.minY, 16)
    XCTAssertNil(scroll.hitTest(CGPoint(x: 40, y: scroll.bounds.maxY + 1), with: nil))
    scroll.setContentOffset(CGPoint(x: 0, y: scroll.contentSize.height - scroll.bounds.height), animated: false)
    scroll.layoutIfNeeded()
    let last = descendants(content).compactMap { $0 as? UIButton }
      .first { $0.accessibilityLabel == "Action 11" }!
    let lastRect = last.convert(last.bounds, to: scroll)
    XCTAssertTrue(scroll.bounds.contains(lastRect), "viewport=\(scroll.bounds) last=\(lastRect) content=\(scroll.contentSize)")
  }
}
