import CryptoKit
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Fetches an avatar with the extension's private HTTP cache and bounded native decoding.
public final class BuzzNotificationAvatarLoader {
  private let cacheLock = NSLock()
  private var httpCaches: [String: URLCache] = [:]
  private let directory: URL
  private let privateKey: (String) -> String?
  private let isCurrent: (BuzzNotificationCacheScope) -> Bool
  private let assertionIssuer: BuzzRelayAssertionIssuer?
  private static let maximumDownloadBytes = 2 * 1024 * 1024
  /// The network transfer; tests substitute it to observe whether a request would be sent.
  var startDownload:
    (URLRequest, URLSessionConfiguration, Int, @escaping (Result<Data, Error>) -> Void) -> Void =
      { request, configuration, maximumBytes, completion in
        BuzzNotificationDownload.start(
          request: request, configuration: configuration, maximumBytes: maximumBytes,
          completion: completion)
      }

  /// `assertionIssuer` supplies the NIP-FI assertion an organization relay requires on media
  /// reads, as it does for the resolver's `/query`; without it those avatars are refused.
  public init(
    directory: URL, privateKey: @escaping (String) -> String?,
    isCurrent: @escaping (BuzzNotificationCacheScope) -> Bool,
    assertionIssuer: BuzzRelayAssertionIssuer? = nil
  ) {
    self.directory = directory
    self.privateKey = privateKey
    self.isCurrent = isCurrent
    self.assertionIssuer = assertionIssuer
  }

  /// Returns a bounded PNG suitable for INImage. HTTP caching retains the original response.
  public func load(
    source: String, scope: BuzzNotificationCacheScope,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    guard isCurrent(scope) else {
      completion(.failure(BuzzNotificationCacheError.revoked))
      return
    }
    if source.hasPrefix("data:") {
      do { completion(.success(try Self.decode(Self.inlineData(source)))) } catch {
        completion(.failure(error))
      }
      return
    }
    guard let original = Self.posterURL(source)
    else {
      completion(.failure(BuzzNotificationCacheError.invalidRecord))
      return
    }
    fetchFirst(
      Self.thumbnailURLs(original, scope: scope) + [original], scope: scope, completion: completion)
  }

  /// Fetches the first of `urls` the relay has: older relays, and media uploaded before WebP
  /// thumbnails, have only the JPEG one, and externally populated media may have neither.
  private func fetchFirst(
    _ urls: [URL], scope: BuzzNotificationCacheScope,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    fetch(urls[0], scope: scope) { result in
      if urls.count > 1, case .failure(BuzzNotificationDownload.Failure.status(404)) = result {
        self.fetchFirst(Array(urls.dropFirst()), scope: scope, completion: completion)
      } else {
        completion(result)
      }
    }
  }

  private func fetch(
    _ url: URL, scope: BuzzNotificationCacheScope,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    guard isCurrent(scope) else {
      completion(.failure(BuzzNotificationCacheError.revoked))
      return
    }
    var request = URLRequest(url: url)
    guard Self.isRelayMedia(url, scope: scope) else {
      download(request, scope: scope, completion: completion)
      return
    }
    guard let key = privateKey(scope.communityID) else {
      completion(.failure(BuzzNotificationCacheError.revoked))
      return
    }
    guard let assertionIssuer, let relayURL = URL(string: scope.relayOrigin) else {
      do {
        request.setValue(
          try Self.authorization(url: url, key: key, account: scope.account),
          forHTTPHeaderField: "Authorization")
      } catch {
        completion(.failure(error))
        return
      }
      download(request, scope: scope, completion: completion)
      return
    }
    // Each attempt is signed afresh. The signed request never follows a redirect, so the
    // assertion stays on the relay.
    func fetch(
      _ assertion: BuzzRelayAssertion?, _ completion: @escaping (Result<Data, Error>) -> Void
    ) {
      // The community may have been removed or its key changed while the assertion was issued.
      // Sending the signed request now would present revoked proofs to the relay.
      guard self.isCurrent(scope) else {
        completion(.failure(BuzzNotificationCacheError.revoked))
        return
      }
      var signed = request
      do {
        signed.setValue(
          try Self.authorization(url: url, key: key, account: scope.account),
          forHTTPHeaderField: "Authorization")
      } catch {
        completion(.failure(error))
        return
      }
      if let assertion {
        signed.setValue(assertion.header, forHTTPHeaderField: "Nostr-Federated-Identity")
      }
      self.download(signed, scope: scope, assertion: assertion, completion: completion)
    }
    assertionIssuer.header(relayURL: relayURL, privateKeyHex: key) { assertion in
      fetch(assertion) { result in
        // A hosted relay is tried without an assertion first, and asks for one by challenging.
        guard assertion == nil, case .failure(BuzzNotificationDownload.Failure.challenged) = result
        else {
          completion(result)
          return
        }
        assertionIssuer.header(relayURL: relayURL, privateKeyHex: key, challenged: true) { answer in
          guard let answer else {
            completion(result)
            return
          }
          fetch(answer, completion)
        }
      }
    }
  }

  /// Bytes fetched with `assertion` are accepted only while the organization session that
  /// issued it is still the shared one, so a retired session's avatar never reaches the cache.
  private func download(
    _ request: URLRequest, scope: BuzzNotificationCacheScope,
    assertion: BuzzRelayAssertion? = nil,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    let configuration = URLSessionConfiguration.default
    configuration.httpCookieStorage = nil
    configuration.urlCredentialStorage = nil
    let cache = httpCache(scope)
    configuration.urlCache = cache
    startDownload(request, configuration, Self.maximumDownloadBytes) { result in
      if let assertion, self.assertionIssuer?.isCurrent(assertion) != true {
        cache.removeCachedResponse(for: request)
        completion(.failure(BuzzNotificationCacheError.revoked))
        return
      }
      guard self.isCurrent(scope) else {
        completion(.failure(BuzzNotificationCacheError.revoked))
        return
      }
      completion(result.flatMap { bytes in Result { try Self.decode(bytes) } })
    }
  }

  private func httpCache(_ scope: BuzzNotificationCacheScope) -> URLCache {
    cacheLock.lock()
    defer { cacheLock.unlock() }
    if let cache = httpCaches[scope.directoryName] { return cache }
    let cache = URLCache(
      memoryCapacity: 512 * 1024, diskCapacity: 8 * 1024 * 1024,
      directory: directory.appendingPathComponent(scope.directoryName).appendingPathComponent(
        "http"))
    httpCaches[scope.directoryName] = cache
    return cache
  }

  static func normalizedSource(_ value: String?) -> String? {
    guard let value else { return nil }
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    if trimmed.hasPrefix("data:image/svg+xml"), trimmed.utf8.count <= 256 * 1024,
      let comma = trimmed.firstIndex(of: ","),
      [
        "data:image/svg+xml", "data:image/svg+xml;base64", "data:image/svg+xml;utf8",
        "data:image/svg+xml;charset=utf-8",
      ].contains(String(trimmed[..<comma]))
    {
      return trimmed
    }
    return BuzzPushPresentationCacheStore.normalizedAvatarURL(trimmed)
  }

  /// Animated avatars use the sender-selected poster, never the animation URL.
  static func posterURL(_ source: String) -> URL? {
    guard let normalized = BuzzPushPresentationCacheStore.normalizedAvatarURL(source),
      var components = URLComponents(string: normalized)
    else { return nil }
    components.fragment = nil
    return components.url
  }

  /// The relay's thumbnails, best first: the WebP keeps transparency; every relay writes the JPEG.
  static func thumbnailURLs(_ url: URL, scope: BuzzNotificationCacheScope) -> [URL] {
    guard isRelayMedia(url, scope: scope) else { return [] }
    let parts = url.lastPathComponent.split(separator: ".")
    guard let hash = parts.first, hash.count == 64,
      VerifiedNostrEvent.hexBytes(String(hash))?.count == 32,
      parts.count <= 2
    else { return [] }
    return ["webp", "jpg"].map {
      url.deletingLastPathComponent().appendingPathComponent("\(hash).thumb.\($0)")
    }
  }

  private static func isRelayMedia(_ url: URL, scope: BuzzNotificationCacheScope) -> Bool {
    guard let origin = URL(string: scope.relayOrigin) else { return false }
    func effectivePort(_ value: URL) -> Int? {
      value.port ?? (value.scheme?.lowercased() == "https" ? 443 : 80)
    }
    return url.scheme?.lowercased() == origin.scheme?.lowercased()
      && url.host?.lowercased() == origin.host?.lowercased()
      && effectivePort(url) == effectivePort(origin)
      && url.path.hasPrefix("/media/")
  }

  private static func inlineData(_ source: String) throws -> Data {
    guard normalizedSource(source) != nil, let comma = source.firstIndex(of: ",") else {
      throw BuzzNotificationCacheError.invalidRecord
    }
    let header = source[..<comma]
    let payload = String(source[source.index(after: comma)...])
    let bytes: Data?
    if header.hasSuffix(";base64") {
      bytes = Data(base64Encoded: payload)
    } else {
      bytes = payload.removingPercentEncoding?.data(using: .utf8)
    }
    guard let bytes, bytes.count <= maximumDownloadBytes else {
      throw BuzzNotificationCacheError.invalidRecord
    }
    return bytes
  }

  static func decode(_ data: Data) throws -> Data {
    if let text = String(data: data.prefix(256), encoding: .utf8),
      text.trimmingCharacters(in: .whitespacesAndNewlines).hasPrefix("<")
    {
      return try BuzzNotificationSVG.decode(data)
    }
    guard !data.isEmpty, data.count <= maximumDownloadBytes,
      let imageSource = CGImageSourceCreateWithData(
        data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
      let properties = CGImageSourceCopyPropertiesAtIndex(imageSource, 0, nil) as? [CFString: Any],
      let width = properties[kCGImagePropertyPixelWidth] as? Int,
      let height = properties[kCGImagePropertyPixelHeight] as? Int,
      width > 0, height > 0, width <= 16384, height <= 16384,
      Int64(width) * Int64(height) <= 64_000_000,
      let image = CGImageSourceCreateThumbnailAtIndex(
        imageSource, 0,
        [
          kCGImageSourceCreateThumbnailFromImageAlways: true,
          kCGImageSourceCreateThumbnailWithTransform: true,
          kCGImageSourceThumbnailMaxPixelSize: 320,
          kCGImageSourceShouldCacheImmediately: true,
        ] as CFDictionary)
    else { throw BuzzNotificationCacheError.invalidRecord }
    let output = NSMutableData()
    guard
      let destination = CGImageDestinationCreateWithData(
        output, UTType.png.identifier as CFString, 1, nil)
    else { throw BuzzNotificationCacheError.invalidRecord }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination),
      output.length <= BuzzNotificationCache.maximumAvatarBytes
    else { throw BuzzNotificationCacheError.oversizedRecord }
    return output as Data
  }

  static func authorization(url: URL, key: String, account: String) throws -> String {
    guard let bytes = VerifiedNostrEvent.hexBytes(key), bytes.count == 32 else {
      throw NostrHTTPAuthError.invalidHex
    }
    let signingKey = bytes
    let pubkey = VerifiedNostrEvent.hex(try NostrSchnorr.publicKey(secret: signingKey))
    guard pubkey == account else { throw BuzzNotificationCacheError.revoked }
    let timestamp = Int(Date().timeIntervalSince1970)
    // URLComponents preserves the brackets required around IPv6 authorities.
    let host = URLComponents(url: url, resolvingAgainstBaseURL: false)?.host ?? ""
    let authority = host + (url.port.map { ":\($0)" } ?? "")
    let tags = [["t", "get"], ["expiration", String(timestamp + 60)], ["server", authority]]
    let content = "Get buzz-media"
    let serialization = try VerifiedNostrEvent.canonicalSerialization(
      pubkey: pubkey, createdAt: timestamp, kind: 24242, tags: tags, content: content)
    let digest = Array(SHA256.hash(data: serialization))
    let id = VerifiedNostrEvent.hex(digest)
    let signature = try NostrSchnorr.sign(message: digest, secret: signingKey)
    let event = VerifiedNostrEvent(
      id: id, pubkey: pubkey, createdAt: timestamp,
      kind: 24242, tags: tags, content: content,
      sig: VerifiedNostrEvent.hex(signature))
    return "Nostr " + (try JSONEncoder().encode(event)).base64EncodedString()
  }
}
