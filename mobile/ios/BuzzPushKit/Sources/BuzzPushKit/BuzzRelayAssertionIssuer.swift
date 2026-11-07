import Foundation

/// An account session the app shares with the notification extension, so the extension can mint
/// the NIP-FI assertion an enforcing relay requires on `/query`. The app shares at most one
/// organization session and one hosted session.
public struct BuzzOrganizationSession: Codable, Equatable, Sendable {
  /// Keychain item, in the access group the app shares with the extension.
  public static let keychainService = "buzz.push.nse.organization-session"
  public static let keychainAccount = "organization"
  /// The hosted account's item, under the same service.
  public static let hostedKeychainAccount = "hosted"

  /// The account's Builderlab HTTPS origin, for example `https://app.builderlab.xyz`.
  public let origin: String
  /// The account session credential, sent only to [origin].
  public let credential: String
  /// Exact relay hosts that trust this organization. Subdomains and other ports never match.
  public let relayHosts: [String]
  /// Hosted only: a relay host must be a subdomain of [communityDomain] and equal to or under
  /// one of these, matching the app's `HostedEnvironment.requiresAuth` and `supportsRelay`.
  public let relayDomains: [String]
  public let communityDomain: String?

  public init(
    origin: String, credential: String, relayHosts: [String], relayDomains: [String] = [],
    communityDomain: String? = nil
  ) {
    self.origin = origin
    self.credential = credential
    self.relayHosts = relayHosts
    self.relayDomains = relayDomains
    self.communityDomain = communityDomain
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    origin = try container.decode(String.self, forKey: .origin)
    credential = try container.decode(String.self, forKey: .credential)
    relayHosts = try container.decodeIfPresent([String].self, forKey: .relayHosts) ?? []
    relayDomains = try container.decodeIfPresent([String].self, forKey: .relayDomains) ?? []
    communityDomain = try container.decodeIfPresent(String.self, forKey: .communityDomain)
  }

  /// Hosted relays enforce NIP-FI per community, so the issuer refuses communities that don't.
  var isHosted: Bool { communityDomain != nil }

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

  /// The account's current identity binding route, beside [assertionURL].
  var currentIdentityURL: URL? {
    guard var route = assertionURL.flatMap({ URLComponents(url: $0, resolvingAgainstBaseURL: false) })
    else { return nil }
    route.path = "/api/goose/v1/buzz/nostr-identities/current"
    return route.url
  }

  /// Matches the app's `OrganizationEnvironment.protects`, or for a hosted session
  /// `HostedRelayAuth`'s hosted rule, for an HTTPS relay URL.
  func protects(_ relayURL: URL) -> Bool {
    guard relayURL.scheme == "https", relayURL.user == nil, relayURL.password == nil,
      relayURL.query == nil, relayURL.fragment == nil,
      relayURL.path.isEmpty || relayURL.path == "/",
      relayURL.port == nil || relayURL.port == 443,
      let host = relayURL.host?.lowercased()
    else { return false }
    if relayHosts.contains(host) { return true }
    guard let communityDomain, host.hasSuffix(".\(communityDomain)") else { return false }
    return relayDomains.contains { host == $0 || host.hasSuffix(".\($0)") }
  }
}

/// A minted `Nostr-Federated-Identity` header and the account session it was minted from.
public struct BuzzRelayAssertion {
  public let header: String
  let organization: BuzzOrganizationSession
}

/// Mints NIP-FI assertions for organization and hosted relays from the shared account sessions.
/// Other relays get no header, exactly as before.
public final class BuzzRelayAssertionIssuer: @unchecked Sendable {
  static let maximumResponseBytes = 32 * 1_024
  static let maximumAssertionBytes = 16_384
  /// A reused assertion must outlive the requests that carry it.
  static let reuseMargin: TimeInterval = 30
  /// How long a hosted issuer's refusal stands. A hosted community that doesn't enforce NIP-FI
  /// is refused, so this keeps its pushes from asking the issuer every time.
  static let refusalLifetime: TimeInterval = 15 * 60

  private let loadSession: () -> BuzzOrganizationSession?
  private let loadHostedSession: () -> BuzzOrganizationSession?
  private let session: URLSession
  private let now: () -> Date
  private let lock = NSLock()
  private var cached: [String: (token: String, expiry: Date)] = [:]
  private var refused: [String: Date] = [:]
  /// Hosted credential and key pairs the account is already bound to.
  private var bound: Set<String> = []
  /// Hosted relays that have challenged for NIP-FI evidence in this process.
  private var challengedRelays: Set<String> = []

  public init(
    session: URLSession,
    loadSession: @escaping () -> BuzzOrganizationSession?,
    loadHostedSession: @escaping () -> BuzzOrganizationSession? = { nil },
    now: @escaping () -> Date = Date.init
  ) {
    self.session = session
    self.loadSession = loadSession
    self.loadHostedSession = loadHostedSession
    self.now = now
  }

  /// Whether the account session that issued `assertion` is still the shared one. A
  /// response fetched with an assertion from a signed-out or replaced session must be dropped.
  public func isCurrent(_ assertion: BuzzRelayAssertion) -> Bool {
    current(assertion.organization.isHosted) == assertion.organization
  }

  private func current(_ hosted: Bool) -> BuzzOrganizationSession? {
    hosted ? loadHostedSession() : loadSession()
  }

  /// The organization session when it protects the relay, as in the app, else the hosted one.
  private func sessionProtecting(_ relayURL: URL) -> BuzzOrganizationSession? {
    if let organization = loadSession(), organization.protects(relayURL) { return organization }
    if let hosted = loadHostedSession(), hosted.isHosted, hosted.protects(relayURL) {
      return hosted
    }
    return nil
  }

  /// Whether `response` is a relay's NIP-FI challenge: 401 with `WWW-Authenticate: Nostr`. A
  /// rejected assertion is a 403 and never carries the challenge.
  public static func isChallenge(_ response: URLResponse?) -> Bool {
    guard let response = response as? HTTPURLResponse, response.statusCode == 401,
      let challenge = response.value(forHTTPHeaderField: "WWW-Authenticate")
    else { return false }
    return challenge.trimmingCharacters(in: .whitespaces).lowercased().hasPrefix("nostr")
  }

  /// Completes with the `Nostr-Federated-Identity` value for an organization or hosted relay, or
  /// nil when no shared session covers the relay or no assertion could be issued. A nil value for
  /// an enforcing relay leaves the relay to refuse the request.
  ///
  /// Hosted communities enforce NIP-FI one at a time and none need Builderlab until they do, so
  /// a hosted relay gets an assertion only once it has `challenged` (answered a request with
  /// [isChallenge]) in this process. Organization relays always get one.
  public func header(
    relayURL: URL, privateKeyHex: String, challenged: Bool = false,
    completion: @escaping (BuzzRelayAssertion?) -> Void
  ) {
    guard let organization = sessionProtecting(relayURL),
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
    if organization.isHosted {
      lock.lock()
      if challenged { challengedRelays.insert(relay) }
      let required = challengedRelays.contains(relay)
      lock.unlock()
      guard required else {
        completion(nil)
        return
      }
    }
    let cacheKey = "\(organization.origin)|\(relay)|\(pubkey)|\(organization.credential)"
    lock.lock()
    if let entry = cached[cacheKey], entry.expiry.timeIntervalSince(now()) > Self.reuseMargin {
      lock.unlock()
      completion(BuzzRelayAssertion(header: "Bearer \(entry.token)", organization: organization))
      return
    }
    if let refusal = refused[cacheKey], refusal > now() {
      lock.unlock()
      completion(nil)
      return
    }
    let needsBinding = organization.isHosted && !bound.contains("\(organization.credential)|\(pubkey)")
    lock.unlock()

    guard needsBinding else {
      mint(
        organization: organization, issuerURL: issuerURL, relay: relay, pubkey: pubkey,
        privateKeyHex: privateKeyHex, cacheKey: cacheKey, completion: completion)
      return
    }
    // Issuance can create the account's first identity binding, and the extension must never
    // choose that identity: the app may still be selecting it, and this key may be one the user
    // discarded. Mint only for a key the hosted account is already bound to, as the app does
    // for background requests.
    guard let identityURL = organization.currentIdentityURL,
      let body = try? JSONSerialization.data(withJSONObject: [String: Any]())
    else {
      completion(nil)
      return
    }
    var request = URLRequest(url: identityURL)
    request.httpMethod = "POST"
    request.httpBody = body
    request.timeoutInterval = 8
    request.setValue("application/json", forHTTPHeaderField: "Accept")
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    request.setValue(organization.origin, forHTTPHeaderField: "Origin")
    request.setValue(organization.credential, forHTTPHeaderField: "X-BB-Session-Credential")
    BuzzNotificationDownload.start(
      request: request, configuration: session.configuration,
      maximumBytes: Self.maximumResponseBytes
    ) { [self] result in
      guard case .success(let data) = result,
        let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
        value["error"] == nil || value["error"] is NSNull
      else {
        if case .failure(let error) = result {
          NSLog("Notification identity check failed: %@", String(describing: error))
        }
        completion(nil)
        return
      }
      guard let identity = value["identity"] as? [String: Any],
        (identity["pubkey_hex"] as? String)?.lowercased() == pubkey
      else {
        // Unbound or bound to another key: ask again only after the refusal lapses.
        lock.lock()
        refused[cacheKey] = now().addingTimeInterval(Self.refusalLifetime)
        lock.unlock()
        completion(nil)
        return
      }
      lock.lock()
      bound.insert("\(organization.credential)|\(pubkey)")
      lock.unlock()
      mint(
        organization: organization, issuerURL: issuerURL, relay: relay, pubkey: pubkey,
        privateKeyHex: privateKeyHex, cacheKey: cacheKey, completion: completion)
    }
  }

  private func mint(
    organization: BuzzOrganizationSession, issuerURL: URL, relay: String, pubkey: String,
    privateKeyHex: String, cacheKey: String, completion: @escaping (BuzzRelayAssertion?) -> Void
  ) {
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
          if organization.isHosted,
            case .status(403)? = error as? BuzzNotificationDownload.Failure
          {
            lock.lock()
            refused[cacheKey] = now().addingTimeInterval(Self.refusalLifetime)
            lock.unlock()
          }
        }
        completion(nil)
        return
      }
      let expiry = Date(timeIntervalSince1970: TimeInterval(expiresAt))
      // A sign-out or account change during issuance revokes this result.
      guard expiry > now(), current(organization.isHosted) == organization else {
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
