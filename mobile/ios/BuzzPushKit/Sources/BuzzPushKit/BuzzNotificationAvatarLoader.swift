import CryptoKit
import Foundation
import ImageIO
import P256K
import UniformTypeIdentifiers

/// Fetches an avatar with the extension's private HTTP cache and bounded native decoding.
public final class BuzzNotificationAvatarLoader {
  private let cacheLock = NSLock()
  private var httpCaches: [String: URLCache] = [:]
  private let directory: URL
  private let privateKey: (String) -> String?
  private let isCurrent: (BuzzNotificationCacheScope) -> Bool
  private static let maximumDownloadBytes = 2 * 1024 * 1024

  public init(
    directory: URL, privateKey: @escaping (String) -> String?,
    isCurrent: @escaping (BuzzNotificationCacheScope) -> Bool
  ) {
    self.directory = directory
    self.privateKey = privateKey
    self.isCurrent = isCurrent
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
    let thumbnail = Self.thumbnailURL(original, scope: scope)
    fetch(thumbnail ?? original, scope: scope) { result in
      if thumbnail != nil, case .failure(BuzzNotificationDownload.Failure.status(404)) = result {
        // Older or externally populated media may not have a generated thumbnail.
        self.fetch(original, scope: scope, completion: completion)
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
    if Self.isRelayMedia(url, scope: scope) {
      guard let key = privateKey(scope.communityID) else {
        completion(.failure(BuzzNotificationCacheError.revoked))
        return
      }
      do {
        request.setValue(
          try Self.authorization(url: url, key: key, account: scope.account),
          forHTTPHeaderField: "Authorization")
      } catch {
        completion(.failure(error))
        return
      }
    }
    let configuration = URLSessionConfiguration.default
    configuration.httpCookieStorage = nil
    configuration.urlCredentialStorage = nil
    configuration.urlCache = httpCache(scope)
    BuzzNotificationDownload.start(
      request: request, configuration: configuration, maximumBytes: Self.maximumDownloadBytes
    ) { result in
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

  static func thumbnailURL(_ url: URL, scope: BuzzNotificationCacheScope) -> URL? {
    guard isRelayMedia(url, scope: scope) else { return nil }
    let parts = url.lastPathComponent.split(separator: ".")
    guard let hash = parts.first, hash.count == 64,
      VerifiedNostrEvent.hexBytes(String(hash))?.count == 32,
      parts.count <= 2
    else { return nil }
    return url.deletingLastPathComponent().appendingPathComponent("\(hash).thumb.jpg")
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
    let signingKey = try P256K.Schnorr.PrivateKey(dataRepresentation: bytes)
    let pubkey = VerifiedNostrEvent.hex(signingKey.xonly.bytes)
    guard pubkey == account else { throw BuzzNotificationCacheError.revoked }
    let timestamp = Int(Date().timeIntervalSince1970)
    // URLComponents preserves the brackets required around IPv6 authorities.
    let host = URLComponents(url: url, resolvingAgainstBaseURL: false)?.host ?? ""
    let authority = host + (url.port.map { ":\($0)" } ?? "")
    let tags = [["t", "get"], ["expiration", String(timestamp + 60)], ["server", authority]]
    let content = "Get buzz-media"
    let serialization = try VerifiedNostrEvent.canonicalSerialization(
      pubkey: pubkey, createdAt: timestamp, kind: 24242, tags: tags, content: content)
    var digest = Array(SHA256.hash(data: serialization))
    let id = VerifiedNostrEvent.hex(digest)
    let signature = try signingKey.signature(message: &digest, auxiliaryRand: nil)
    let event = VerifiedNostrEvent(
      id: id, pubkey: pubkey, createdAt: timestamp,
      kind: 24242, tags: tags, content: content,
      sig: VerifiedNostrEvent.hex(signature.dataRepresentation))
    return "Nostr " + (try JSONEncoder().encode(event)).base64EncodedString()
  }
}
