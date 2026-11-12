import XCTest
@testable import Buzz

final class NativeEmojiRedirectPolicyTests: XCTestCase {
  private let session = URLSession(configuration: .ephemeral)

  override func tearDown() {
    session.invalidateAndCancel()
    super.tearDown()
  }

  /// Runs the production delegate for a redirect of `original` and returns
  /// the request it chose to follow (nil means the redirect is refused).
  private func followedRequest(for original: URLRequest) -> URLRequest? {
    // A task that is never resumed still exposes `originalRequest`.
    let task = session.dataTask(with: original)
    let redirectTarget = URL(string: "https://elsewhere.example/avatar.png")!
    let response = HTTPURLResponse(
      url: original.url!, statusCode: 302, httpVersion: nil,
      headerFields: ["Location": redirectTarget.absoluteString]
    )!
    var followed: URLRequest?
    var called = false
    NativeEmojiRedirectPolicy.shared.urlSession(
      session,
      task: task,
      willPerformHTTPRedirection: response,
      newRequest: URLRequest(url: redirectTarget)
    ) { request in
      called = true
      followed = request
    }
    XCTAssertTrue(called, "the delegate must always call its completion handler")
    return followed
  }

  func testRefusesRedirectForRequestCarryingRelayAssertion() {
    var request = URLRequest(url: URL(string: "https://relay.example/media/emoji.png")!)
    request.setValue("token", forHTTPHeaderField: NativeEmojiRedirectPolicy.relayAssertionHeader)

    XCTAssertNil(followedRequest(for: request))
  }

  func testFollowsRedirectForOrdinaryRequest() {
    let request = URLRequest(url: URL(string: "https://cdn.example/emoji.png")!)

    XCTAssertEqual(
      followedRequest(for: request)?.url,
      URL(string: "https://elsewhere.example/avatar.png")
    )
  }

  func testAssertionHeaderNameMatchesDartBridge() {
    XCTAssertEqual(NativeEmojiRedirectPolicy.relayAssertionHeader, "Nostr-Federated-Identity")
  }
}
