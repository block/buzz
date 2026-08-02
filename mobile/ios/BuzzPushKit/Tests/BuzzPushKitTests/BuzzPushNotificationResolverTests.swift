import Foundation
#if canImport(FoundationNetworking)
  import FoundationNetworking
#endif
import XCTest

@testable import BuzzPushKit

final class BuzzPushNotificationResolverTests: XCTestCase {
  private static let privateKey = String(repeating: "0", count: 63) + "1"
  private static let ownPubkey =
    "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
  private static let now = Int(Date().timeIntervalSince1970)
  private static let gatewayBody = "Reconnect to your relay now"

  override func setUp() {
    super.setUp()
    URLProtocolStub.reset()
  }

  override func tearDown() {
    URLProtocolStub.reset()
    super.tearDown()
  }

  func testResolveReturnsNilWhenCommunitiesDataIsMissing() {
    let result = resolve(makeResolver(communitiesData: nil))

    XCTAssertNil(result)
    XCTAssertTrue(URLProtocolStub.requests.isEmpty)
  }

  func testResolveReturnsNilWhenCommunitiesDataIsUndecodable() {
    let result = resolve(makeResolver(communitiesData: Data("not json".utf8)))

    XCTAssertNil(result)
    XCTAssertTrue(URLProtocolStub.requests.isEmpty)
  }

  func testResolveReturnsNilOnKeychainMiss() throws {
    let result = resolve(makeResolver(
      communitiesData: try snapshotData([community()]),
      privateKeys: [:]
    ))

    XCTAssertNil(result)
    XCTAssertTrue(URLProtocolStub.requests.isEmpty)
  }

  func testResolveReturnsNilForNon2xxRelayResponse() throws {
    URLProtocolStub.handler = { request in
      Self.response(request, status: 503, data: Data())
    }
    let result = resolve(makeResolver(communitiesData: try snapshotData([community()])))

    XCTAssertNil(result)
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testResolveReturnsNilForUndecodableRelayResponse() throws {
    URLProtocolStub.handler = { request in
      Self.response(request, status: 200, data: Data("not events".utf8))
    }
    let result = resolve(makeResolver(communitiesData: try snapshotData([community()])))

    XCTAssertNil(result)
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testDecodeResolutionFiltersOwnPubkeyEvent() {
    let result = BuzzPushNotificationResolver.decodeResolution(
      events: [event(pubkey: Self.ownPubkey, content: "This should be filtered")],
      community: community()
    )

    XCTAssertNil(result)
  }

  func testDecodeResolutionReturnsNilWhenSanitizedPreviewIsEmpty() {
    let event = event(content: "  \n\t  ")

    let result = BuzzPushNotificationResolver.decodeResolution(
      events: [event],
      community: community()
    )

    XCTAssertNil(result)
  }

  func testPreviewBodySanitizesCodeLinksAndWhitespace() {
    let content = """
          Before   ```swift
          print("secret")
          ``` `inline` [docs](https://example.com/docs)
          ![image](https://example.com/image.png) https://example.com/raw
          After
          """

    XCTAssertEqual(
      BuzzPushNotificationResolver.previewBody(content),
      "Before [code] inline docs image [link] After"
    )
  }

  func testPreviewBodyTruncatesTo178CharactersIncludingEllipsis() {
    let preview = BuzzPushNotificationResolver.previewBody(String(repeating: "x", count: 200))

    XCTAssertEqual(preview.count, 178)
    XCTAssertEqual(preview, String(repeating: "x", count: 177) + "…")
  }

  func testDecodeResolutionUsesLowestIDWhenCreatedAtTies() {
    let result = BuzzPushNotificationResolver.decodeResolution(
      events: [
        event(id: "a", content: "lower ID", createdAt: Self.now),
        event(id: "b", content: "higher ID", createdAt: Self.now),
      ],
      community: community()
    )

    XCTAssertEqual(result?.1.id, "a")
    XCTAssertEqual(result?.0.body, "lower ID")
  }

  func testResolveSucceedsAndMutatesGatewayContent() throws {
    let event = try JSONDecoder().decode(
      VerifiedNostrEvent.self,
      from: Data(Self.fixtureEvent.utf8)
    )
    URLProtocolStub.handler = { request in
      Self.response(request, status: 200, data: try JSONEncoder().encode([event]))
    }

    let result = try XCTUnwrap(resolve(makeResolver(
      communitiesData: try snapshotData([community()])
    )))

    XCTAssertNotEqual(result.title, Self.gatewayBody)
    XCTAssertNotEqual(result.body, Self.gatewayBody)
    XCTAssertEqual(result.title, String(event.pubkey.prefix(8)) + "…")
    XCTAssertEqual(result.body, "Hello Buzz")
    XCTAssertEqual(result.subtitle, "Community")
    XCTAssertEqual(result.threadIdentifier, "channel-id")
  }

  private func makeResolver(
    communitiesData: Data?,
    privateKeys: [String: String] = ["community-id": privateKey]
  ) -> BuzzPushNotificationResolver {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [URLProtocolStub.self]
    return BuzzPushNotificationResolver(
      session: URLSession(configuration: configuration),
      defaults: nil,
      loadCommunitiesData: { communitiesData },
      loadPrivateKey: { privateKeys[$0] },
      now: { Self.now }
    )
  }

  private func resolve(_ resolver: BuzzPushNotificationResolver) -> BuzzPushResolution? {
    let completed = expectation(description: "resolver completed")
    var result: BuzzPushResolution?
    resolver.resolve {
      result = $0
      completed.fulfill()
    }
    wait(for: [completed], timeout: 2)
    return result
  }

  private func community(
    id: String = "community-id",
    name: String = "Community",
    relayUrl: String = "https://relay.example",
    pubkey: String? = ownPubkey
  ) -> BuzzPushCommunity {
    BuzzPushCommunity(id: id, name: name, relayUrl: relayUrl, pubkey: pubkey)
  }

  private func snapshotData(_ communities: [BuzzPushCommunity]) throws -> Data {
    let dictionaries: [[String: Any]] = communities.map {
      [
        "id": $0.id,
        "name": $0.name,
        "relayUrl": $0.relayUrl,
        "pubkey": $0.pubkey as Any,
      ]
    }
    return try JSONSerialization.data(withJSONObject: ["communities": dictionaries])
  }

  private func event(
    id: String = "event-id",
    pubkey: String = "author-pubkey",
    content: String,
    createdAt: Int = now,
    kind: Int = 9,
    tags: [[String]] = []
  ) -> VerifiedNostrEvent {
    VerifiedNostrEvent(
      id: id,
      pubkey: pubkey,
      createdAt: createdAt,
      kind: kind,
      tags: tags,
      content: content,
      sig: "signature"
    )
  }

  private static let fixtureEvent = #"""
  {"id":"2d827228c07234a9c30ac19e0d2f01bcc09ca3b1004e95a429b4771a8b42ebb8","sig":"44085c838c0afc8328ce6fb95d4c851021d6b43f15b7f1206ce225a433c2337336837af3df26cdd9301111d16a72508d47130bb2c9206ae4d7dbea336ede1d4d","content":"  Hello   [Buzz](https://buzz.block.xyz)  ","kind":9,"pubkey":"c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5","created_at":1785551670,"tags":[["h","channel-id"]]}
  """#

  private static func response(
    _ request: URLRequest,
    status: Int,
    data: Data
  ) -> (HTTPURLResponse, Data) {
    let response = HTTPURLResponse(
      url: request.url!,
      statusCode: status,
      httpVersion: "HTTP/1.1",
      headerFields: ["Content-Type": "application/json"]
    )!
    return (response, data)
  }
}

private final class URLProtocolStub: URLProtocol, @unchecked Sendable {
  static let lock = NSLock()
  static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?
  static var requests: [URLRequest] = []

  override class func canInit(with request: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

  override func startLoading() {
    Self.lock.lock()
    Self.requests.append(request)
    let handler = Self.handler
    Self.lock.unlock()
    do {
      let (response, data) = try handler?(request) ?? { throw URLError(.unsupportedURL) }()
      client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
      client?.urlProtocol(self, didLoad: data)
      client?.urlProtocolDidFinishLoading(self)
    } catch {
      client?.urlProtocol(self, didFailWithError: error)
    }
  }

  override func stopLoading() {}

  static func reset() {
    lock.lock()
    handler = nil
    requests = []
    lock.unlock()
  }
}
