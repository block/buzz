import BuzzPushKit
import CryptoKit
import Foundation
import Security

/// Talks to the last-used community's relay over its HTTP bridge, signing as
/// the account the app shares with its extensions.
final class ShareRelayClient {
  let community: PushLeaseCommunity
  let pubkey: String
  let baseURL: URL
  private let privateKey: String
  private let session: URLSession
  private let assertionIssuer: BuzzRelayAssertionIssuer

  private static var info: [String: Any] { Bundle.main.infoDictionary ?? [:] }
  private static var keychainAccessGroup: String? { info["BuzzKeychainAccessGroup"] as? String }
  static var appGroupURL: URL? {
    (info["BuzzAppGroupIdentifier"] as? String).flatMap {
      FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: $0)
    }
  }

  private init(community: PushLeaseCommunity, pubkey: String, baseURL: URL, privateKey: String) {
    self.community = community
    self.pubkey = pubkey
    self.baseURL = baseURL
    self.privateKey = privateKey
    let configuration = URLSessionConfiguration.ephemeral
    configuration.timeoutIntervalForRequest = 30
    session = URLSession(configuration: configuration)
    let group = Self.keychainAccessGroup
    assertionIssuer = BuzzRelayAssertionIssuer(
      session: URLSession(configuration: .ephemeral),
      loadSession: {
        Self.keychainData(service: BuzzOrganizationSession.keychainService,
          account: BuzzOrganizationSession.keychainAccount, group: group)
          .flatMap { try? JSONDecoder().decode(BuzzOrganizationSession.self, from: $0) }
      },
      loadHostedSession: {
        Self.keychainData(service: BuzzOrganizationSession.keychainService,
          account: BuzzOrganizationSession.hostedKeychainAccount, group: group)
          .flatMap { try? JSONDecoder().decode(BuzzOrganizationSession.self, from: $0) }
      })
  }

  /// The last-used community and its signing key.
  static func current() throws -> ShareRelayClient {
    guard let container = appGroupURL,
      let data = try? Data(contentsOf: container.appendingPathComponent(BuzzPushPresentationCacheStore.fileName)),
      let snapshot = try? JSONDecoder().decode(PushLeaseSnapshot.self, from: data),
      !snapshot.communities.isEmpty
    else { throw ShareError.notSignedIn }
    let defaults = (info["BuzzAppGroupIdentifier"] as? String).flatMap { UserDefaults(suiteName: $0) }
    let selected = BuzzSelectedCommunity.id(in: defaults)
    let community = snapshot.communities.first { $0.id == selected } ?? snapshot.communities[0]
    guard let keyData = keychainData(service: "buzz.push.nse.signing", account: community.id, group: keychainAccessGroup),
      let key = String(data: keyData, encoding: .utf8),
      let pubkey = NostrEventSigner.publicKey(privateKeyHex: key),
      community.pubkey == nil || community.pubkey?.lowercased() == pubkey,
      let baseURL = httpURL(community.relayUrl)
    else { throw ShareError.notSignedIn }
    return ShareRelayClient(community: community, pubkey: pubkey, baseURL: baseURL, privateKey: key)
  }

  private static func httpURL(_ relay: String) -> URL? {
    guard var components = URLComponents(string: relay), components.host?.isEmpty == false else { return nil }
    switch components.scheme?.lowercased() {
    case "wss", "https": components.scheme = "https"
    case "ws", "http": components.scheme = "http"
    default: return nil
    }
    components.path = ""
    components.query = nil
    return components.url
  }

  private static func keychainData(service: String, account: String, group: String?) -> Data? {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
      kSecReturnData as String: true,
      kSecMatchLimit as String: kSecMatchLimitOne,
    ]
    if let group, !group.isEmpty { query[kSecAttrAccessGroup as String] = group }
    var item: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess else { return nil }
    return item as? Data
  }

  // MARK: Requests

  /// Events matching `filters`, as JSON maps.
  func query(_ filters: [[String: Any]]) async throws -> [[String: Any]] {
    let body = try JSONSerialization.data(withJSONObject: filters)
    let (data, response) = try await post("/query", body: body)
    try Self.check(response, data)
    return (try JSONSerialization.jsonObject(with: data) as? [[String: Any]]) ?? []
  }

  /// Signs and submits one event; returns its id once the relay accepts it.
  func publish(kind: Int, content: String, tags: [[String]], createdAt: Int) async throws -> String {
    let event = try NostrEventSigner.sign(
      kind: kind, content: content, tags: tags, privateKeyHex: privateKey, createdAt: createdAt)
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.withoutEscapingSlashes]
    let body = try encoder.encode(event)
    let (data, response) = try await post("/events", body: body)
    try Self.check(response, data)
    if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      json["accepted"] as? Bool == false
    {
      throw ShareError.message("The relay refused the message: \(json["message"] as? String ?? "unknown reason")")
    }
    return event.id
  }

  /// Uploads the image, streaming it from disk; returns the blob descriptor.
  func upload(_ image: SharedContent.Image) async throws -> [String: Any] {
    let sha256 = try Self.sha256(of: image.url)
    var (data, response) = try await put("/upload", file: image.url, mime: image.mime, sha256: sha256)
    if let status = (response as? HTTPURLResponse)?.statusCode, status == 404 || status == 405 {
      (data, response) = try await put("/media/upload", file: image.url, mime: image.mime, sha256: sha256)
    }
    if let status = (response as? HTTPURLResponse)?.statusCode, status == 413 {
      throw ShareError.message("This photo is too large to send.")
    }
    try Self.check(response, data)
    guard let descriptor = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
      throw ShareError.message("The relay sent an unreadable upload response.")
    }
    return descriptor
  }

  private func post(_ path: String, body: Data) async throws -> (Data, URLResponse) {
    let url = URL(string: path, relativeTo: baseURL)!.absoluteURL
    return try await withFederatedIdentity { assertion in
      var request = URLRequest(url: url)
      request.httpMethod = "POST"
      request.httpBody = body
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      request.setValue(
        try NostrHTTPAuth.authorizationHeader(url: url, method: "POST", body: body, privateKeyHex: self.privateKey),
        forHTTPHeaderField: "Authorization")
      if let assertion { request.setValue(assertion, forHTTPHeaderField: "Nostr-Federated-Identity") }
      return try await self.session.data(for: request)
    }
  }

  private func put(_ path: String, file: URL, mime: String, sha256: String) async throws -> (Data, URLResponse) {
    let url = URL(string: path, relativeTo: baseURL)!.absoluteURL
    return try await withFederatedIdentity { assertion in
      var request = URLRequest(url: url)
      request.httpMethod = "PUT"
      request.timeoutInterval = 120
      request.setValue(mime, forHTTPHeaderField: "Content-Type")
      request.setValue(sha256, forHTTPHeaderField: "X-SHA-256")
      request.setValue(try self.uploadAuthorization(sha256: sha256), forHTTPHeaderField: "Authorization")
      if let assertion { request.setValue(assertion, forHTTPHeaderField: "Nostr-Federated-Identity") }
      return try await self.session.upload(for: request, fromFile: file)
    }
  }

  /// Blossom upload authorization, as the app signs it.
  private func uploadAuthorization(sha256: String) throws -> String {
    let expiration = Int(Date().timeIntervalSince1970) + 60
    var authority = baseURL.host?.lowercased() ?? ""
    if let port = baseURL.port, port != 443 { authority += ":\(port)" }
    let event = try NostrEventSigner.sign(
      kind: 24242, content: "Upload buzz-media",
      tags: [["t", "upload"], ["x", sha256], ["expiration", "\(expiration)"], ["server", authority]],
      privateKeyHex: privateKey)
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.withoutEscapingSlashes]
    let encoded = try encoder.encode(event).base64EncodedString()
      .replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
      .replacingOccurrences(of: "=", with: "")
    return "Nostr \(encoded)"
  }

  /// Runs `attempt` with the relay's NIP-FI assertion, if it needs one. A relay
  /// that challenges a request without one gets a single retry with one.
  private func withFederatedIdentity(
    _ attempt: @escaping (String?) async throws -> (Data, URLResponse)
  ) async throws -> (Data, URLResponse) {
    let first = await assertion(challenged: false)
    let result = try await attempt(first)
    guard first == nil, BuzzRelayAssertionIssuer.isChallenge(result.1),
      let second = await assertion(challenged: true)
    else { return result }
    return try await attempt(second)
  }

  private func assertion(challenged: Bool) async -> String? {
    await withCheckedContinuation { continuation in
      assertionIssuer.header(relayURL: baseURL, privateKeyHex: privateKey, challenged: challenged) {
        continuation.resume(returning: $0?.header)
      }
    }
  }

  private static func check(_ response: URLResponse, _ data: Data) throws {
    guard let status = (response as? HTTPURLResponse)?.statusCode else {
      throw ShareError.message("The relay didn't answer.")
    }
    guard (200..<300).contains(status) else {
      let text = String(data: data.prefix(300), encoding: .utf8) ?? ""
      throw ShareError.message("The relay answered \(status). \(text)")
    }
  }

  /// SHA-256 of a file, read in chunks so a large photo never sits in memory.
  private static func sha256(of url: URL) throws -> String {
    let handle = try FileHandle(forReadingFrom: url)
    defer { try? handle.close() }
    var hasher = SHA256()
    while let chunk = try handle.read(upToCount: 1 << 20), !chunk.isEmpty { hasher.update(data: chunk) }
    return hasher.finalize().map { String(format: "%02x", $0) }.joined()
  }
}
