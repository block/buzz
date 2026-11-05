import Foundation

/// The organization account session the app shares with the notification extension, so the
/// extension can mint the NIP-FI assertion an enforcing organization relay requires on `/query`.
public struct BuzzOrganizationSession: Codable, Equatable, Sendable {
  /// Keychain item, in the access group the app shares with the extension.
  public static let keychainService = "buzz.push.nse.organization-session"
  public static let keychainAccount = "organization"

  /// The organization's Builderlab HTTPS origin, for example `https://block.builderlab.xyz`.
  public let origin: String
  /// The account session credential, sent only to [origin].
  public let credential: String
  /// Exact relay hosts that trust this organization. Subdomains and other ports never match.
  public let relayHosts: [String]

  public init(origin: String, credential: String, relayHosts: [String]) {
    self.origin = origin
    self.credential = credential
    self.relayHosts = relayHosts
  }

  /// The issuer route, when [origin] is a bare HTTPS origin.
  var assertionURL: URL? {
    guard let components = URLComponents(string: origin),
      components.scheme == "https", components.host?.isEmpty == false,
      components.user == nil, components.password == nil, components.port == nil,
      components.query == nil, components.fragment == nil,
      components.path.isEmpty || components.path == "/"
    else { return nil }
    var route = components
    route.path = "/api/goose/v1/buzz/identity/assertions"
    return route.url
  }

  /// Matches the app's `OrganizationEnvironment.protects` for an HTTPS relay URL.
  func protects(_ relayURL: URL) -> Bool {
    guard relayURL.scheme == "https", relayURL.user == nil, relayURL.password == nil,
      relayURL.query == nil, relayURL.fragment == nil,
      relayURL.path.isEmpty || relayURL.path == "/",
      relayURL.port == nil || relayURL.port == 443,
      let host = relayURL.host?.lowercased()
    else { return false }
    return relayHosts.contains(host)
  }
}

/// A minted `Nostr-Federated-Identity` header and the organization session it was minted from.
public struct BuzzRelayAssertion {
  public let header: String
  let organization: BuzzOrganizationSession
}

/// Mints NIP-FI assertions for organization relays from the shared organization session.
/// Relays outside the organization get no header, exactly as before.
public final class BuzzRelayAssertionIssuer: @unchecked Sendable {
  static let maximumResponseBytes = 32 * 1_024
  static let maximumAssertionBytes = 16_384
  /// A reused assertion must outlive the requests that carry it.
  static let reuseMargin: TimeInterval = 30

  private let loadSession: () -> BuzzOrganizationSession?
  private let session: URLSession
  private let now: () -> Date
  private let lock = NSLock()
  private var cached: [String: (token: String, expiry: Date)] = [:]

  public init(
    session: URLSession,
    loadSession: @escaping () -> BuzzOrganizationSession?,
    now: @escaping () -> Date = Date.init
  ) {
    self.session = session
    self.loadSession = loadSession
    self.now = now
  }

  /// Whether the organization session that issued `assertion` is still the shared one. A
  /// response fetched with an assertion from a signed-out or replaced session must be dropped.
  public func isCurrent(_ assertion: BuzzRelayAssertion) -> Bool {
    loadSession() == assertion.organization
  }

  /// Completes with the `Nostr-Federated-Identity` value for an organization relay, or nil when
  /// the relay is not an organization relay or no assertion could be issued. A nil value for an
  /// enforcing relay leaves the relay to refuse the request.
  public func header(
    relayURL: URL, privateKeyHex: String, completion: @escaping (BuzzRelayAssertion?) -> Void
  ) {
    guard let organization = loadSession(), organization.protects(relayURL),
      let issuerURL = organization.assertionURL,
      let host = relayURL.host?.lowercased(),
      let secret = VerifiedNostrEvent.hexBytes(privateKeyHex), secret.count == 32,
      let pubkeyBytes = try? NostrSchnorr.publicKey(secret: secret)
    else {
      completion(nil)
      return
    }
    let pubkey = VerifiedNostrEvent.hex(pubkeyBytes)
    let relay = "wss://\(host)"
    let cacheKey = "\(organization.origin)|\(relay)|\(pubkey)|\(organization.credential)"
    lock.lock()
    if let entry = cached[cacheKey], entry.expiry.timeIntervalSince(now()) > Self.reuseMargin {
      lock.unlock()
      completion(BuzzRelayAssertion(header: "Bearer \(entry.token)", organization: organization))
      return
    }
    lock.unlock()

    guard
      let body = try? JSONSerialization.data(
        withJSONObject: ["relay_url": relay, "nostr_pubkey": pubkey], options: [.sortedKeys]),
      let auth = try? NostrHTTPAuth.authorizationHeader(
        url: issuerURL, method: "POST", body: body, privateKeyHex: privateKeyHex)
    else {
      completion(nil)
      return
    }
    var request = URLRequest(url: issuerURL)
    request.httpMethod = "POST"
    request.httpBody = body
    request.timeoutInterval = 8
    request.setValue("application/json", forHTTPHeaderField: "Accept")
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    request.setValue(organization.origin, forHTTPHeaderField: "Origin")
    request.setValue(organization.credential, forHTTPHeaderField: "X-BB-Session-Credential")
    request.setValue(auth, forHTTPHeaderField: "Authorization")
    BuzzNotificationDownload.start(
      request: request, configuration: session.configuration,
      maximumBytes: Self.maximumResponseBytes
    ) { [self] result in
      guard case .success(let data) = result,
        let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
        let token = value["assertion"] as? String, !token.isEmpty,
        token.utf8.count <= Self.maximumAssertionBytes,
        !token.contains(where: { $0.isNewline || $0 == "\0" }),
        (value["nostr_pubkey"] as? String)?.lowercased() == pubkey,
        let expiresAt = value["expires_at"] as? Int
      else {
        if case .failure(let error) = result {
          NSLog("Notification assertion issuance failed: %@", String(describing: error))
        }
        completion(nil)
        return
      }
      let expiry = Date(timeIntervalSince1970: TimeInterval(expiresAt))
      // A sign-out or account change during issuance revokes this result.
      guard expiry > now(), loadSession() == organization else {
        completion(nil)
        return
      }
      lock.lock()
      cached[cacheKey] = (token, expiry)
      lock.unlock()
      completion(BuzzRelayAssertion(header: "Bearer \(token)", organization: organization))
    }
  }
}
