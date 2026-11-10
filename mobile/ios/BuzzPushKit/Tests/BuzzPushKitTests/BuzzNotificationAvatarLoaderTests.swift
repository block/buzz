import Foundation
import ImageIO
import Testing

@testable import BuzzPushKit

@Suite("Native notification avatar decoding")
struct BuzzNotificationAvatarLoaderTests {
  @Test("Avatar proofs preserve IPv6 brackets and explicit ports")
  func signedServerAuthority() throws {
    let key = String(repeating: "0", count: 63) + "1"
    let account = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    for authority in ["[::1]:3000", "[2001:db8::1]", "relay.example:443", "relay.example"] {
      let url = try #require(URL(string: "https://\(authority)/media/avatar.png"))
      let header = try BuzzNotificationAvatarLoader.authorization(url: url, key: key, account: account)
      let data = try #require(Data(base64Encoded: String(header.dropFirst(6))))
      let event = try JSONDecoder().decode(VerifiedNostrEvent.self, from: data)
      #expect(event.tags.contains(["server", authority]))
      #expect(event.kind == 24242)
      #expect(event.hasValidIDAndSignature())
    }
  }

  @Test("Relay media uses canonical hosts and default ports without trusting other origins")
  func mediaOrigins() throws {
    let hash = String(repeating: "a", count: 64)
    for (relay, media, expected) in [
      ("wss://relay.example:443", "https://relay.example", true),
      ("wss://relay.example", "https://RELAY.EXAMPLE:443", true),
      ("ws://relay.example:80", "http://RELAY.EXAMPLE", true),
      ("wss://relay.example:8443", "https://relay.example:8443", true),
      ("wss://relay.example", "https://relay.example:8443", false),
      ("wss://relay.example", "http://relay.example", false),
      ("wss://relay.example", "https://other.example", false),
    ] {
      let scope = try #require(
        BuzzNotificationCacheScope(
          community: PushLeaseCommunity(
            id: "community", name: "Test", relayUrl: relay, pubkey: "account", policies: [],
            notificationSessionID: "session")))
      let url = try #require(URL(string: "\(media)/media/\(hash).png"))
      #expect(
        BuzzNotificationAvatarLoader.thumbnailURLs(url, scope: scope).map(\.lastPathComponent)
          == (expected ? ["\(hash).thumb.webp", "\(hash).thumb.jpg"] : []))
    }
  }

  @Test("Animated avatar selects the poster without fetching the animation")
  func animatedPoster() throws {
    let source =
      "https://images.example/poster.png#buzz-anim=https%3A%2F%2Fimages.example%2Fanimation.apng"
    #expect(
      BuzzNotificationAvatarLoader.posterURL(source)?.absoluteString
        == "https://images.example/poster.png")
    #expect(BuzzNotificationAvatarLoader.posterURL("file:///private/avatar.png") == nil)
  }

  @Test("Raster decoder produces a bounded PNG and rejects non-image data")
  func raster() throws {
    let bytes = try #require(
      Data(
        base64Encoded:
          "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII="
      ))
    let png = try BuzzNotificationAvatarLoader.decode(bytes)
    let source = try #require(CGImageSourceCreateWithData(png as CFData, nil))
    let properties = try #require(
      CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
    #expect(properties[kCGImagePropertyPixelWidth] as? Int == 1)
    #expect(properties[kCGImagePropertyPixelHeight] as? Int == 1)
    #expect(png.count <= BuzzNotificationCache.maximumAvatarBytes)
    #expect(throws: BuzzNotificationCacheError.self) {
      try BuzzNotificationAvatarLoader.decode(Data("not an image".utf8))
    }
  }

  @Test("A scope revoked while the assertion is issued never starts the signed download")
  func scopeRevokedDuringAssertionIssuance() async throws {
    let key = String(repeating: "0", count: 63) + "1"
    let account = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    let scope = try #require(
      BuzzNotificationCacheScope(
        community: PushLeaseCommunity(
          id: "community", name: "Test", relayUrl: "wss://relay.org.example", pubkey: account,
          policies: [], notificationSessionID: "session")))
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [AssertionIssuanceStub.self]
    let state = AvatarScopeState()
    AssertionIssuanceStub.onRequest = { state.revoke() }
    defer { AssertionIssuanceStub.onRequest = nil }
    let issuer = BuzzRelayAssertionIssuer(
      session: URLSession(configuration: configuration),
      loadSession: {
        BuzzOrganizationSession(
          origin: "https://identity.org.example", credential: "session-credential",
          relayHosts: ["relay.org.example"])
      })
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let loader = BuzzNotificationAvatarLoader(
      directory: directory, privateKey: { _ in key }, isCurrent: { _ in state.isCurrent },
      assertionIssuer: issuer)
    loader.startDownload = { _, _, _, completion in
      state.recordDownload()
      completion(.failure(URLError(.cancelled)))
    }

    let result = await withCheckedContinuation { continuation in
      loader.load(
        source: "https://relay.org.example/media/\(String(repeating: "a", count: 64)).png",
        scope: scope
      ) { continuation.resume(returning: $0) }
    }

    guard case .failure(BuzzNotificationCacheError.revoked) = result else {
      Issue.record("Expected a revoked failure, got \(result)")
      return
    }
    #expect(state.downloads == 0)
  }

  @Test("An organization relay avatar is fetched with the issued assertion")
  func relayAvatarCarriesAssertion() async throws {
    let key = String(repeating: "0", count: 63) + "1"
    let account = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    let scope = try #require(
      BuzzNotificationCacheScope(
        community: PushLeaseCommunity(
          id: "community", name: "Test", relayUrl: "wss://relay.org.example", pubkey: account,
          policies: [], notificationSessionID: "session")))
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [AssertionIssuanceStub.self]
    let issuer = BuzzRelayAssertionIssuer(
      session: URLSession(configuration: configuration),
      loadSession: {
        BuzzOrganizationSession(
          origin: "https://identity.org.example", credential: "session-credential",
          relayHosts: ["relay.org.example"])
      })
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let loader = BuzzNotificationAvatarLoader(
      directory: directory, privateKey: { _ in key }, isCurrent: { _ in true },
      assertionIssuer: issuer)
    let requests = AvatarRequests()
    loader.startDownload = { request, _, _, completion in
      requests.record(request)
      completion(.failure(URLError(.cancelled)))
    }

    _ = await withCheckedContinuation { continuation in
      loader.load(
        source: "https://relay.org.example/media/\(String(repeating: "a", count: 64)).png",
        scope: scope
      ) { continuation.resume(returning: $0) }
    }

    let request = try #require(requests.first)
    #expect(
      request.value(forHTTPHeaderField: "Nostr-Federated-Identity")
        == "Bearer header.payload.signature")
    #expect(request.value(forHTTPHeaderField: "Authorization") != nil)
  }

  @Test("A hosted relay avatar is fetched without an assertion until the relay challenges")
  func hostedAvatarAnswersChallenge() async throws {
    let key = String(repeating: "0", count: 63) + "1"
    let account = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    let scope = try #require(
      BuzzNotificationCacheScope(
        community: PushLeaseCommunity(
          id: "community", name: "Test", relayUrl: "wss://team.communities.example",
          pubkey: account, policies: [], notificationSessionID: "session")))
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [HostedIssuanceStub.self]
    let issuer = BuzzRelayAssertionIssuer(
      session: URLSession(configuration: configuration), loadSession: { nil },
      loadHostedSession: {
        BuzzOrganizationSession(
          origin: "https://app.example", credential: "hosted-credential", relayHosts: [],
          relayDomains: ["communities.example"], communityDomain: "communities.example")
      })
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let loader = BuzzNotificationAvatarLoader(
      directory: directory, privateKey: { _ in key }, isCurrent: { _ in true },
      assertionIssuer: issuer)
    let requests = AvatarRequests()
    loader.startDownload = { request, _, _, completion in
      requests.record(request)
      let challenged = request.value(forHTTPHeaderField: "Nostr-Federated-Identity") == nil
      completion(.failure(challenged ? BuzzNotificationDownload.Failure.challenged : URLError(.cancelled)))
    }

    _ = await withCheckedContinuation { continuation in
      loader.load(
        source: "https://team.communities.example/media/\(String(repeating: "a", count: 64)).png",
        scope: scope
      ) { continuation.resume(returning: $0) }
    }

    #expect(requests.all.map { $0.value(forHTTPHeaderField: "Nostr-Federated-Identity") }
      == [nil, "Bearer header.payload.signature"])
    #expect(requests.all.allSatisfy { $0.value(forHTTPHeaderField: "Authorization") != nil })
  }

  @Test("An avatar fetched with an assertion from a replaced organization session is refused")
  func replacedOrganizationSessionRefusesAvatar() async throws {
    let key = String(repeating: "0", count: 63) + "1"
    let account = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    let scope = try #require(
      BuzzNotificationCacheScope(
        community: PushLeaseCommunity(
          id: "community", name: "Test", relayUrl: "wss://relay.org.example", pubkey: account,
          policies: [], notificationSessionID: "session")))
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [AssertionIssuanceStub.self]
    let sessions = OrganizationSessions()
    let issuer = BuzzRelayAssertionIssuer(
      session: URLSession(configuration: configuration), loadSession: { sessions.current })
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let loader = BuzzNotificationAvatarLoader(
      directory: directory, privateKey: { _ in key }, isCurrent: { _ in true },
      assertionIssuer: issuer)
    let png = try #require(
      Data(
        base64Encoded:
          "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII="
      ))
    loader.startDownload = { _, _, _, completion in
      // The member signs in again while the avatar is in flight; the community is unchanged.
      sessions.replace()
      completion(.success(png))
    }

    let result = await withCheckedContinuation { continuation in
      loader.load(
        source: "https://relay.org.example/media/\(String(repeating: "a", count: 64)).png",
        scope: scope
      ) { continuation.resume(returning: $0) }
    }

    guard case .failure(BuzzNotificationCacheError.revoked) = result else {
      Issue.record("Expected a revoked failure, got \(result)")
      return
    }
  }
}

private final class AvatarRequests: @unchecked Sendable {
  private let lock = NSLock()
  private var requests: [URLRequest] = []

  var first: URLRequest? { lock.withLock { requests.first } }
  var all: [URLRequest] { lock.withLock { requests } }
  func record(_ request: URLRequest) { lock.withLock { requests.append(request) } }
}

private final class OrganizationSessions: @unchecked Sendable {
  private let lock = NSLock()
  private var credential = "session-credential"

  var current: BuzzOrganizationSession {
    lock.withLock {
      BuzzOrganizationSession(
        origin: "https://identity.org.example", credential: credential,
        relayHosts: ["relay.org.example"])
    }
  }
  func replace() { lock.withLock { credential = "replacement-credential" } }
}

private final class AvatarScopeState: @unchecked Sendable {
  private let lock = NSLock()
  private var current = true
  private var started = 0

  var isCurrent: Bool { lock.withLock { current } }
  var downloads: Int { lock.withLock { started } }
  func revoke() { lock.withLock { current = false } }
  func recordDownload() { lock.withLock { started += 1 } }
}

/// Answers the organization's assertion issuer, running `onRequest` while issuance is in flight.
private final class HostedIssuanceStub: URLProtocol, @unchecked Sendable {
  override class func canInit(with request: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

  override func startLoading() {
    let account = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    let value: [String: Any] =
      request.url?.path == "/api/goose/v1/buzz/nostr-identities/current"
      ? ["identity": ["pubkey_hex": account]]
      : [
        "assertion": "header.payload.signature", "nostr_pubkey": account,
        "expires_at": Int(Date().timeIntervalSince1970) + 300,
      ]
    let body = try! JSONSerialization.data(withJSONObject: value)
    let response = HTTPURLResponse(
      url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
    client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
    client?.urlProtocol(self, didLoad: body)
    client?.urlProtocolDidFinishLoading(self)
  }

  override func stopLoading() {}
}

private final class AssertionIssuanceStub: URLProtocol, @unchecked Sendable {
  nonisolated(unsafe) static var onRequest: (() -> Void)?

  override class func canInit(with request: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

  override func startLoading() {
    Self.onRequest?()
    let body = try! JSONSerialization.data(withJSONObject: [
      "assertion": "header.payload.signature",
      "nostr_pubkey": "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
      "expires_at": Int(Date().timeIntervalSince1970) + 300,
    ])
    let response = HTTPURLResponse(
      url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
    client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
    client?.urlProtocol(self, didLoad: body)
    client?.urlProtocolDidFinishLoading(self)
  }

  override func stopLoading() {}
}
