import Foundation
import XCTest

@testable import BuzzPushKit

final class BuzzNotificationDownloadTests: XCTestCase {
  override func tearDown() {
    DownloadProtocol.response = nil
    super.tearDown()
  }

  func testRejectsOversizedDeclaredResponse() {
    check(headers: ["Content-Length": "1000"], body: Data([1]), limit: 8, succeeds: false)
  }

  func testRejectsOversizedStreamWithoutLengthHeader() {
    check(headers: [:], body: Data(repeating: 1, count: 9), limit: 8, succeeds: false)
  }

  func testAcceptsBoundedResponse() {
    check(headers: [:], body: Data([1, 2, 3]), limit: 8, succeeds: true)
  }

  private func check(headers: [String: String], body: Data, limit: Int, succeeds: Bool) {
    let done = expectation(description: "Bounded transfer completes once")
    done.assertForOverFulfill = true
    DownloadProtocol.response = (headers, body)
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [DownloadProtocol.self]
    BuzzNotificationDownload.start(
      request: URLRequest(url: URL(string: "https://images.example/test")!),
      configuration: configuration, maximumBytes: limit
    ) { result in
      switch result {
      case .success(let data):
        XCTAssertTrue(succeeds)
        XCTAssertEqual(data, body)
      case .failure(let error):
        XCTAssertFalse(succeeds)
        guard case BuzzNotificationDownload.Failure.tooLarge = error else {
          XCTFail("Expected size limit error, got \(error)")
          done.fulfill()
          return
        }
      }
      done.fulfill()
    }
    wait(for: [done], timeout: 2)
  }
}

private final class DownloadProtocol: URLProtocol {
  static var response: ([String: String], Data)?
  override class func canInit(with request: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
  override func startLoading() {
    guard let (headers, data) = Self.response, let url = request.url,
      let response = HTTPURLResponse(
        url: url, statusCode: 200, httpVersion: nil, headerFields: headers)
    else { return }
    client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
    client?.urlProtocol(self, didLoad: data)
    client?.urlProtocolDidFinishLoading(self)
  }
  override func stopLoading() {}
}
