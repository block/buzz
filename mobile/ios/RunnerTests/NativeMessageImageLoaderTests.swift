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
