import CryptoKit
import Foundation

/// Account and relay identity for one local sign-in's notification cache.
public struct BuzzNotificationCacheScope: Codable, Equatable, Sendable {
  public let communityID: String
  public let relayOrigin: String
  public let account: String
  public let sessionID: String

  public init?(community: PushLeaseCommunity) {
    guard let origin = BuzzPushPresentationCacheStore.canonicalRelayOrigin(community.relayUrl),
      let account = community.pubkey, !account.isEmpty,
      let session = community.notificationSessionID, !session.isEmpty
    else { return nil }
    communityID = community.id
    relayOrigin = origin
    self.account = account.lowercased()
    sessionID = session
  }

  /// A filename-safe identity that contains no credentials.
  public var directoryName: String {
    let fields = [communityID, relayOrigin, account, sessionID]
    let bytes = fields.flatMap { Array("\($0.utf8.count):\($0)".utf8) }
    return VerifiedNostrEvent.hex(SHA256.hash(data: bytes))
  }
}

/// Verified metadata plus the time it was last checked with the relay.
public struct BuzzNotificationMetadata: Codable, Equatable, Sendable {
  public let event: VerifiedNostrEvent
  public let checkedAt: Date

  /// One shared freshness threshold for profile and channel presentation metadata.
  public static let freshness: TimeInterval = 60 * 60

  public func isFresh(at date: Date) -> Bool {
    let age = date.timeIntervalSince(checkedAt)
    return age >= 0 && age < Self.freshness
  }
}

/// Last successfully decoded image for a sender, retained during replacement fetches.
public struct BuzzNotificationAvatar: Codable, Equatable, Sendable {
  public let pubkey: String
  public let source: String
  public let data: Data
  public let savedAt: Date
}

/// Failures callers must report; revoked accounts must never use stale cache results.
public enum BuzzNotificationCacheError: Error {
  case revoked, invalidRecord, oversizedRecord, corruptStore
}

/// Extension-owned metadata storage. Each operation reads current disk state under
/// a process lock and publishes one complete snapshot with an atomic replacement.
/// Image HTTP response storage remains URLCache's responsibility.
public final class BuzzNotificationCache: @unchecked Sendable {
  public static let maximumBytes = 8 * 1024 * 1024
  public static let maximumAvatarBytes = 256 * 1024
  private static let processLock = NSLock()
  private let root: URL
  private let now: () -> Date
  private let isCurrent: (BuzzNotificationCacheScope) -> Bool

  private struct State: Codable {
    var version = 1
    var metadata: [BuzzNotificationMetadata] = []
    var avatars: [BuzzNotificationAvatar] = []
  }

  public init(
    directory: URL,
    now: @escaping () -> Date = Date.init,
    isCurrent: @escaping (BuzzNotificationCacheScope) -> Bool
  ) {
    root = directory
    self.now = now
    self.isCurrent = isCurrent
  }

  /// Returns a complete cached record, including stale records for presentation fallback.
  public func metadata(
    scope: BuzzNotificationCacheScope, kind: Int, author: String, identifier: String? = nil
  ) throws -> BuzzNotificationMetadata? {
    try access(scope) { state in
      state.metadata.first {
        $0.event.kind == kind && $0.event.pubkey.lowercased() == author.lowercased()
          && Self.identifier($0.event) == identifier
      }
    }
  }

  /// Saves verified replaceable metadata without allowing an older response to win.
  public func save(event: VerifiedNostrEvent, scope: BuzzNotificationCacheScope) throws {
    guard [0, 39_000, 39_002].contains(event.kind), event.hasValidIDAndSignature(),
      event.content.utf8.count <= 256 * 1024
    else { throw BuzzNotificationCacheError.invalidRecord }
    if event.kind == 0 {
      guard let data = event.content.data(using: .utf8),
        (try? JSONSerialization.jsonObject(with: data)) is [String: Any]
      else { throw BuzzNotificationCacheError.invalidRecord }
    }
    try access(scope, write: true) { state in
      let index = state.metadata.firstIndex {
        $0.event.kind == event.kind && $0.event.pubkey == event.pubkey
          && Self.identifier($0.event) == Self.identifier(event)
      }
      let existing = index.map { state.metadata[$0].event }
      guard
        BuzzPushPresentationCacheStore.shouldReplace(
          existingCreatedAt: existing?.createdAt, existingID: existing?.id,
          candidateCreatedAt: event.createdAt, candidateID: event.id
        )
      else { return }
      let record = BuzzNotificationMetadata(event: event, checkedAt: now())
      if let index { state.metadata[index] = record } else { state.metadata.append(record) }
      if event.kind == 0, Self.removesPicture(event) {
        state.avatars.removeAll { $0.pubkey == event.pubkey }
      }
    }
  }

  /// Returns the previous image even when the current profile references a replacement.
  public func avatar(scope: BuzzNotificationCacheScope, pubkey: String) throws
    -> BuzzNotificationAvatar?
  {
    try access(scope) { state in state.avatars.first { $0.pubkey == pubkey } }
  }

  /// Publishes an image only if the current profile still identifies this source.
  public func saveAvatar(
    _ data: Data, source: String, profileID: String, pubkey: String,
    scope: BuzzNotificationCacheScope
  ) throws {
    guard !data.isEmpty, data.count <= Self.maximumAvatarBytes else {
      throw BuzzNotificationCacheError.oversizedRecord
    }
    try access(scope, write: true) { state in
      guard
        let profile = state.metadata.first(where: {
          $0.event.kind == 0 && $0.event.pubkey == pubkey
        }), profile.event.id == profileID, Self.picture(profile.event) == source
      else { return }
      state.avatars.removeAll { $0.pubkey == pubkey }
      state.avatars.append(
        BuzzNotificationAvatar(
          pubkey: pubkey, source: source, data: data, savedAt: now()))
    }
  }

  /// Deletes abandoned sign-ins on the extension's next execution.
  public func removeRevokedScopes(keeping scopes: [BuzzNotificationCacheScope]) throws {
    try locked {
      let retained = Set(scopes.map(\.directoryName))
      for url in try FileManager.default.contentsOfDirectory(
        at: root, includingPropertiesForKeys: [.isDirectoryKey]
      ) where url.lastPathComponent != "cache.lock" {
        guard !retained.contains(url.lastPathComponent) else { continue }
        try FileManager.default.removeItem(at: url)
      }
    }
  }

  /// Normalized source from a verified profile. Missing or empty picture means removal.
  static func picture(_ event: VerifiedNostrEvent) -> String? {
    guard let data = event.content.data(using: .utf8),
      let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    else { return nil }
    return BuzzNotificationAvatarLoader.normalizedSource(object["picture"] as? String)
  }

  private static func removesPicture(_ event: VerifiedNostrEvent) -> Bool {
    guard let data = event.content.data(using: .utf8),
      let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    else { return false }
    guard let value = object["picture"], !(value is NSNull) else { return true }
    guard let value = value as? String else { return false }
    return value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }

  private static func identifier(_ event: VerifiedNostrEvent) -> String? {
    event.kind == 0 ? nil : event.tags.first { $0.count > 1 && $0[0] == "d" }?[1]
  }

  private func access<T>(
    _ scope: BuzzNotificationCacheScope, write: Bool = false,
    _ operation: (inout State) throws -> T
  ) throws -> T {
    try locked {
      guard isCurrent(scope) else { throw BuzzNotificationCacheError.revoked }
      let directory = root.appendingPathComponent(scope.directoryName, isDirectory: true)
      let file = directory.appendingPathComponent("metadata.json")
      var state = State()
      if FileManager.default.fileExists(atPath: file.path) {
        let attributes = try file.resourceValues(forKeys: [.fileSizeKey])
        guard let size = attributes.fileSize, size <= Self.maximumBytes else {
          try FileManager.default.removeItem(at: file)
          throw BuzzNotificationCacheError.corruptStore
        }
        do {
          state = try JSONDecoder().decode(State.self, from: Data(contentsOf: file))
        } catch {
          try FileManager.default.removeItem(at: file)
          throw BuzzNotificationCacheError.corruptStore
        }
        guard state.version == 1 else {
          try FileManager.default.removeItem(at: file)
          throw BuzzNotificationCacheError.corruptStore
        }
      }
      let result = try operation(&state)
      guard isCurrent(scope) else { throw BuzzNotificationCacheError.revoked }
      if write {
        state.metadata = Array(state.metadata.sorted { $0.checkedAt > $1.checkedAt }.prefix(512))
        state.avatars = Array(state.avatars.sorted { $0.savedAt > $1.savedAt }.prefix(128))
        let encoder = JSONEncoder()
        var data = try encoder.encode(state)
        while data.count > Self.maximumBytes {
          if !state.avatars.isEmpty {
            state.avatars.removeLast()
          } else if !state.metadata.isEmpty {
            state.metadata.removeLast()
          } else {
            throw BuzzNotificationCacheError.oversizedRecord
          }
          data = try encoder.encode(state)
        }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try data.write(to: file, options: .atomic)
      }
      return result
    }
  }

  private func locked<T>(_ operation: () throws -> T) throws -> T {
    Self.processLock.lock()
    defer { Self.processLock.unlock() }
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let descriptor = open(root.appendingPathComponent("cache.lock").path, O_CREAT | O_RDWR, 0o600)
    guard descriptor >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
    defer { close(descriptor) }
    guard flock(descriptor, LOCK_EX) == 0 else {
      throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
    }
    defer { flock(descriptor, LOCK_UN) }
    return try operation()
  }
}
