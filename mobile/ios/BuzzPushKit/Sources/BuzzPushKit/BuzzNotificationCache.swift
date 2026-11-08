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
      community.notificationSessionID?.isEmpty != true
    else { return nil }
    communityID = community.id
    relayOrigin = origin
    self.account = account.lowercased()
    // Before the first launch after upgrade, the account snapshot has no session
    // field. Its scope is still isolated by account, relay and community. The new
    // app replaces this marker with a UUID on its first sync and every re-add.
    sessionID = community.notificationSessionID ?? "legacy-before-first-sync-v1"
  }

  /// Confirms the loaded credential belongs to this scope's account.
  public func matchesAccountKey(_ privateKey: String?) -> Bool {
    guard let privateKey, let bytes = VerifiedNostrEvent.hexBytes(privateKey), bytes.count == 32,
      let pubkey = try? NostrSchnorr.publicKey(secret: bytes)
    else { return false }
    return VerifiedNostrEvent.hex(pubkey) == account
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
    var avatarRequests: [String: String]?
    /// Messages already shown in a notification, newest first. Optional so stores
    /// written before it existed still decode.
    var presented: [PresentedMessage]?
  }

  private struct PresentedMessage: Codable {
    let id: String
    /// The event's own `created_at`. Retention follows it, so an entry stays while the
    /// relay still ranks the event among the newest results.
    let createdAt: Int
    /// `createdAt` capped at the time it was shown, so a future-dated event can't hold
    /// back later messages.
    let watermark: Int
  }

  /// How far before the newest shown message a message may be and still be shown.
  /// Pushes can resolve out of order, and the relay accepts a `created_at` up to 15
  /// minutes off (`MAX_TIMESTAMP_DRIFT_SECS` in the relay's ingest handler), so a new
  /// message can carry an earlier time than one already shown. Anything older is
  /// history, not the message a push is for.
  public static let presentationReorderWindow = 15 * 60

  /// The oldest time still within ``presentationReorderWindow`` of `newest`. Saturates,
  /// because a signed event can carry any `created_at` and an overflow would crash the
  /// extension on every push.
  static func presentationCutoff(after newest: Int) -> Int {
    let (cutoff, overflow) = newest.subtractingReportingOverflow(presentationReorderWindow)
    return overflow ? .min : cutoff
  }
  /// Retention keeps up to this many entries by `watermark` and as many by `createdAt`,
  /// so neither future-dated entries nor a burst of recent ones can evict the other.
  static let maximumPresentedMessages = 256

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
    try save(events: [event], scope: scope)
  }

  /// Commits records returned together in a single atomic snapshot.
  func save(events: [VerifiedNostrEvent], scope: BuzzNotificationCacheScope) throws {
    try save(events: events, scope: scope, channelRefresh: false)
  }

  /// Keeps a partial channel response without combining it with an old counterpart.
  func saveChannel(
    channel: VerifiedNostrEvent?, membership: VerifiedNostrEvent?, scope: BuzzNotificationCacheScope
  ) throws {
    try save(events: [channel, membership].compactMap { $0 }, scope: scope, channelRefresh: true)
  }

  private func save(
    events: [VerifiedNostrEvent], scope: BuzzNotificationCacheScope, channelRefresh: Bool
  ) throws {
    guard !events.isEmpty else { return }
    for event in events {
      guard [0, 39_000, 39_002].contains(event.kind), event.hasValidIDAndSignature(),
        event.content.utf8.count <= 256 * 1024
      else { throw BuzzNotificationCacheError.invalidRecord }
      if event.kind == 0 {
        guard let data = event.content.data(using: .utf8),
          (try? JSONSerialization.jsonObject(with: data)) is [String: Any]
        else { throw BuzzNotificationCacheError.invalidRecord }
      }
    }
    try access(scope, write: true) { state in
      if channelRefresh {
        // Reject the whole refresh if any returned record predates its cached
        // version. Accepting only the other record would mix refreshes again.
        for event in events {
          let existing = state.metadata.first {
            $0.event.kind == event.kind && $0.event.pubkey == event.pubkey
              && Self.identifier($0.event) == Self.identifier(event)
          }?.event
          guard BuzzPushPresentationCacheStore.shouldReplace(
            existingCreatedAt: existing?.createdAt, existingID: existing?.id,
            candidateCreatedAt: event.createdAt, candidateID: event.id
          ) else { return }
        }
      }
      for event in events {
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
        else { continue }
        let record = BuzzNotificationMetadata(event: event, checkedAt: now())
        if let index { state.metadata[index] = record } else { state.metadata.append(record) }
        if channelRefresh, events.count == 1, [39000, 39002].contains(event.kind) {
          let counterpartKind = event.kind == 39000 ? 39002 : 39000
          state.metadata.removeAll {
            $0.event.kind == counterpartKind && $0.event.pubkey == event.pubkey
              && Self.identifier($0.event) == Self.identifier(event)
          }
        }
        if event.kind == 0, Self.removesPicture(event) {
          state.avatars.removeAll { $0.pubkey == event.pubkey }
        }
      }
    }
  }

  struct Presentation {
    var profile: BuzzNotificationMetadata?
    var avatar: BuzzNotificationAvatar?
    var channel: BuzzNotificationMetadata?
    var membership: BuzzNotificationMetadata?
  }

  /// Reads related fields from one snapshot, so a concurrent update cannot mix generations.
  func presentation(
    scope: BuzzNotificationCacheScope, sender: String,
    channelID: String?, authority: String?
  ) throws -> Presentation {
    try access(scope) { state in
      let records = state.metadata
      return Presentation(
        profile: records.first { $0.event.kind == 0 && $0.event.pubkey == sender },
        avatar: state.avatars.first { $0.pubkey == sender },
        channel: records.first {
          channelID != nil && authority != nil && $0.event.kind == 39000
            && $0.event.pubkey == authority && Self.identifier($0.event) == channelID
        },
        membership: records.first {
          channelID != nil && authority != nil && $0.event.kind == 39002
            && $0.event.pubkey == authority && Self.identifier($0.event) == channelID
        })
    }
  }

  /// Returns the previous image even when the current profile references a replacement.
  public func avatar(scope: BuzzNotificationCacheScope, pubkey: String) throws
    -> BuzzNotificationAvatar?
  {
    try access(scope) { state in state.avatars.first { $0.pubkey == pubkey } }
  }

  /// Registers the newest image request under the same lock used by all cache writers.
  public func beginAvatarRequest(
    profileID: String, pubkey: String, scope: BuzzNotificationCacheScope
  ) throws -> String? {
    try access(scope, write: true) { state in
      guard
        state.metadata.contains(where: {
          $0.event.kind == 0 && $0.event.pubkey == pubkey && $0.event.id == profileID
        })
      else { return nil }
      let requestID = UUID().uuidString
      if state.avatarRequests == nil { state.avatarRequests = [:] }
      state.avatarRequests?[pubkey] = requestID
      return requestID
    }
  }

  /// Publishes only the latest request while the current profile still identifies its source.
  public func saveAvatar(
    _ data: Data, source: String, profileID: String, requestID: String, pubkey: String,
    scope: BuzzNotificationCacheScope
  ) throws {
    guard !data.isEmpty, data.count <= Self.maximumAvatarBytes else {
      throw BuzzNotificationCacheError.oversizedRecord
    }
    try access(scope, write: true) { state in
      guard
        let profile = state.metadata.first(where: {
          $0.event.kind == 0 && $0.event.pubkey == pubkey
        }), profile.event.id == profileID, Self.picture(profile.event) == source,
        state.avatarRequests?[pubkey] == requestID
      else { return }
      state.avatars.removeAll { $0.pubkey == pubkey }
      state.avatars.append(
        BuzzNotificationAvatar(
          pubkey: pubkey, source: source, data: data, savedAt: now()))
    }
  }

  /// Claims the first of `candidates`, newest first, that hasn't been shown and isn't
  /// older than the newest message shown in any of `scopes` by more than
  /// ``presentationReorderWindow``, and returns its index. A push carries no event
  /// reference, so without this every push whose own event isn't the newest match would
  /// repeat the same message. One lock covers the whole selection, so concurrent pushes
  /// can't claim against a stale view, and only a claim writes. A revoked scope is
  /// skipped. Broken storage must not hide messages, so a candidate whose store can't
  /// be read or written is returned without being recorded.
  public func claimFirstUnpresented(
    _ candidates: [(event: VerifiedNostrEvent, scope: BuzzNotificationCacheScope)],
    scopes: [BuzzNotificationCacheScope]
  ) throws -> Int? {
    try locked {
      var shown: [String: Result<[PresentedMessage], Error>] = [:]
      func presented(_ scope: BuzzNotificationCacheScope) -> Result<[PresentedMessage], Error> {
        if let cached = shown[scope.directoryName] { return cached }
        let result = Result { try unlockedAccess(scope) { $0.presented ?? [] } }
        shown[scope.directoryName] = result
        return result
      }
      let newestShown = (scopes + candidates.map(\.scope))
        .compactMap { try? presented($0).get().map(\.watermark).max() }.max()
      let now = Int(now().timeIntervalSince1970)
      for (index, candidate) in candidates.enumerated() {
        let watermark = min(candidate.event.createdAt, now)
        switch presented(candidate.scope) {
        case .failure(BuzzNotificationCacheError.revoked):
          continue
        case .failure(let error):
          NSLog("Notification dedupe unavailable: %@", String(describing: error))
          return index
        case .success(let entries):
          guard !entries.contains(where: { $0.id == candidate.event.id }) else { continue }
          if let newestShown, watermark < Self.presentationCutoff(after: newestShown) {
            continue
          }
        }
        do {
          try unlockedAccess(candidate.scope, write: true) { state in
            state.presented =
              [PresentedMessage(
                id: candidate.event.id, createdAt: candidate.event.createdAt,
                watermark: watermark)] + (state.presented ?? [])
          }
        } catch BuzzNotificationCacheError.revoked {
          continue
        } catch {
          NSLog("Notification dedupe not recorded: %@", String(describing: error))
        }
        return index
      }
      return nil
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
    try locked { try unlockedAccess(scope, write: write, operation) }
  }

  /// ``access(_:write:_:)`` for a caller that already holds the lock.
  private func unlockedAccess<T>(
    _ scope: BuzzNotificationCacheScope, write: Bool = false,
    _ operation: (inout State) throws -> T
  ) throws -> T {
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
      let profileAuthors = Set(
        state.metadata.filter { $0.event.kind == 0 }.map { $0.event.pubkey })
      state.avatarRequests = state.avatarRequests?.filter { profileAuthors.contains($0.key) }
      if let presented = state.presented, let newest = presented.map(\.watermark).max() {
        // The age check rejects anything older than the window, so those entries can
        // go. A future-dated entry stays while the relay may still rank it first.
        let cutoff = Self.presentationCutoff(after: newest)
        // Entries are newest claim first, so ties keep the most recent claims.
        let current = Array(presented.filter { $0.createdAt >= cutoff }.enumerated())
        func newestIDs(by key: (PresentedMessage) -> Int) -> [String] {
          current.sorted {
            key($0.element) == key($1.element)
              ? $0.offset < $1.offset : key($0.element) > key($1.element)
          }.prefix(Self.maximumPresentedMessages).map(\.element.id)
        }
        let retained = Set(newestIDs(by: \.watermark)).union(newestIDs(by: \.createdAt))
        state.presented = current.map(\.element).filter { retained.contains($0.id) }
      }
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
