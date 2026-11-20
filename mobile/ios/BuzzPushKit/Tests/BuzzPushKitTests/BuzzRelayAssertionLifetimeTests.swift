import XCTest

@testable import BuzzPushKit

final class BuzzRelayAssertionLifetimeTests: XCTestCase {
  func testAssertionThatExpiredDuringSleepIsReplaced() throws {
    typealias Stub = BuzzPushNotificationResolverTests.URLProtocolStub
    Stub.reset()
    defer { Stub.reset() }
    let config = URLSessionConfiguration.ephemeral
    config.protocolClasses = [Stub.self]
    let session = URLSession(configuration: config)
    defer { session.invalidateAndCancel() }
    var wall = 1_800_000_000
    var awake: TimeInterval = 1_000
    var mints = 0
    Stub.handler = { request in
      mints += 1
      let claims = try JSONSerialization.data(withJSONObject: ["iat": wall, "exp": wall + 300])
      let payload = claims.base64EncodedString().replacingOccurrences(of: "=", with: "")
      let data = try JSONSerialization.data(withJSONObject: [
        "assertion": "e30.\(payload).c2ln",
        "nostr_pubkey": BuzzPushNotificationResolverTests.ownPubkey,
        "expires_at": wall + 300,
      ])
      return BuzzPushNotificationResolverTests.response(request, status: 200, data: data)
    }
    let issuer = BuzzRelayAssertionIssuer(
      session: session,
      loadSession: {
        BuzzOrganizationSession(origin: "https://identity.org.example", credential: "credential",
          relayHosts: ["relay.org.example"])
      },
      now: { Date(timeIntervalSince1970: TimeInterval(wall)) }, uptime: { awake })
    func acquire() {
      let done = expectation(description: "assertion")
      issuer.header(relayURL: URL(string: "https://relay.org.example")!,
        privateKeyHex: BuzzPushNotificationResolverTests.privateKey) { result in
        XCTAssertNotNil(result)
        done.fulfill()
      }
      wait(for: [done], timeout: 3)
    }
    acquire()
    XCTAssertEqual(mints, 1)
    // Sleep: the wall clock moves on while systemUptime does not. The issuer is process-wide.
    wall += 600
    awake += 1
    acquire()
    XCTAssertEqual(mints, 2, "an assertion that expired during sleep must be replaced")
  }

  /// The issuer refuses any request with a cookie, and its hosts set Cloudflare's `__cf_bm` on
  /// every response, so a cookie set on the identity check must never reach the mint.
  func testIssuerRequestsCarryNoCookieAfterAResponseSetsOne() throws {
    let wall = 1_800_000_000
    let issuer = BuzzRelayAssertionIssuer(
      session: URLSession(configuration: .ephemeral), loadSession: { nil },
      loadHostedSession: {
        BuzzOrganizationSession(
          origin: "https://app.example", credential: "hosted-credential", relayHosts: [],
          relayDomains: ["communities.example"], communityDomain: "communities.example")
      },
      now: { Date(timeIntervalSince1970: TimeInterval(wall)) })
    // A stand-in for the loading system's cookie handling: it stores each response's cookies
    // and sends them back, unless the configuration turns cookies off.
    let jar = HTTPCookieStorage.sharedCookieStorage(forGroupContainerIdentifier: UUID().uuidString)
    var sent: [URLRequest] = []
    issuer.startDownload = { request, configuration, _, completion in
      var request = request
      if configuration.httpCookieStorage != nil, configuration.httpShouldSetCookies {
        let cookies = jar.cookies(for: request.url!) ?? []
        HTTPCookie.requestHeaderFields(with: cookies).forEach {
          request.setValue($0.value, forHTTPHeaderField: $0.key)
        }
      }
      sent.append(request)
      if configuration.httpCookieStorage != nil {
        jar.setCookies(
          HTTPCookie.cookies(
            withResponseHeaderFields: [
              "Set-Cookie": "__cf_bm=token; Domain=app.example; Path=/; Secure; HttpOnly"
            ], for: request.url!),
          for: request.url!, mainDocumentURL: nil)
      }
      let identity = ["identity": ["pubkey_hex": BuzzPushNotificationResolverTests.ownPubkey]]
      let assertion: [String: Any] = [
        "assertion": "e30.eyJpYXQiOjE4MDAwMDAwMDB9.c2ln",
        "nostr_pubkey": BuzzPushNotificationResolverTests.ownPubkey, "expires_at": wall + 300,
      ]
      let isIdentityCheck = request.url?.path == "/api/goose/v1/buzz/nostr-identities/current"
      completion(Result {
        try JSONSerialization.data(withJSONObject: isIdentityCheck ? identity : assertion)
      })
    }
    let done = expectation(description: "assertion")
    issuer.header(
      relayURL: URL(string: "https://team.communities.example")!,
      privateKeyHex: BuzzPushNotificationResolverTests.privateKey, challenged: true
    ) { result in
      XCTAssertNotNil(result)
      done.fulfill()
    }
    wait(for: [done], timeout: 3)
    XCTAssertEqual(sent.map(\.url?.path), [
      "/api/goose/v1/buzz/nostr-identities/current", "/api/goose/v1/buzz/identity/assertions",
    ])
    XCTAssertEqual(sent.map { $0.value(forHTTPHeaderField: "Cookie") }, [nil, nil])
  }

  func testDeadlineWithPhoneClock120SecondsSlowOrFast() {
    // Issuer: iat 10_000, exp 10_180. Request sent at uptime 1_000, answered at once.
    func deadline(wall: Int) -> TimeInterval {
      BuzzRelayAssertionIssuer.localDeadline(
        requestStart: 1_000, receipt: 1_000,
        wallAtReceipt: Date(timeIntervalSince1970: TimeInterval(wall)), exp: 10_180, iat: 10_000)
    }
    XCTAssertEqual(deadline(wall: 9_880), 1_180, "slow: the lifetime caps it")
    XCTAssertEqual(deadline(wall: 10_120), 1_060, "fast: only what exp leaves")
    XCTAssertEqual(deadline(wall: 10_181), 1_000, "none left: refused at receipt")
    // A fractional `iat` is a valid NumericDate; the lifetime rounds down.
    let iat = BuzzRelayAssertionIssuer.issuedAt("e30.eyJpYXQiOjEwMDAwLjUsImV4cCI6MTAxODB9.c2ln")
    XCTAssertEqual(iat, 10_000.5)
    XCTAssertEqual(
      BuzzRelayAssertionIssuer.localDeadline(
        requestStart: 1_000, receipt: 1_000, wallAtReceipt: Date(timeIntervalSince1970: 9_880),
        exp: 10_180, iat: iat ?? 0), 1_179)
    // A sleep: the wall clock jumps 1 hour, uptime doesn't.
    let wall = Date(timeIntervalSince1970: 10_000)
    func reusable(_ now: Date) -> Bool {
      BuzzRelayAssertionIssuer.reusable(
        receipt: 1_000, wall: wall, budget: 30, uptime: 1_000, now: now)
    }
    XCTAssertTrue(reusable(wall))
    XCTAssertFalse(reusable(wall.addingTimeInterval(3_600)), "a sleep ends reuse")
  }
}
