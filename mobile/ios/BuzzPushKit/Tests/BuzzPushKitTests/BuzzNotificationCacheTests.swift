import CryptoKit
import Foundation
import P256K
import Testing

@testable import BuzzPushKit

@Suite("Extension-owned notification cache")
struct BuzzNotificationCacheTests {
  private let key = String(repeating: "0", count: 63) + "1"

  private func scope(_ session: String = "login-one") throws -> BuzzNotificationCacheScope {
    try #require(
      BuzzNotificationCacheScope(
        community: PushLeaseCommunity(
          id: "community", name: "Test", relayUrl: "wss://relay.example",
          pubkey: "account", policies: [], notificationSessionID: session)))
  }

  private func directory() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
  }

  @Test("Replacement retains the previous avatar; removal rejects a late replacement")
  func replacementAndRemoval() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scope = try scope()
    let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    let first = try signedEvent(
      privateKey: key, createdAt: 1, kind: 0,
      content: #"{"name":"A","picture":"https://image.example/a.png"}"#)
    try cache.save(event: first, scope: scope)
    let firstRequest = try #require(
      try cache.beginAvatarRequest(
        profileID: first.id, pubkey: first.pubkey, scope: scope))
    let bytes = Data([1, 2, 3])
    try cache.saveAvatar(
      bytes, source: "https://image.example/a.png",
      profileID: first.id, requestID: firstRequest, pubkey: first.pubkey, scope: scope)
    let replacement = try signedEvent(
      privateKey: key, createdAt: 2, kind: 0,
      content: #"{"name":"A","picture":"https://image.example/b.png"}"#)
    try cache.save(event: replacement, scope: scope)
    #expect(try cache.avatar(scope: scope, pubkey: first.pubkey)?.data == bytes)
    let replacementRequest = try #require(
      try cache.beginAvatarRequest(
        profileID: replacement.id, pubkey: first.pubkey, scope: scope))
    let removal = try signedEvent(privateKey: key, createdAt: 3, kind: 0, content: #"{"name":"A"}"#)
    try cache.save(event: removal, scope: scope)
    try cache.saveAvatar(
      bytes, source: "https://image.example/b.png",
      profileID: replacement.id, requestID: replacementRequest, pubkey: first.pubkey, scope: scope)
    #expect(try cache.avatar(scope: scope, pubkey: first.pubkey) == nil)
    try cache.save(event: first, scope: scope)
    #expect(try cache.metadata(scope: scope, kind: 0, author: first.pubkey)?.event.id == removal.id)
  }

  @Test("Two cache objects preserve each other's records and data survives reopening")
  func multipleInstances() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scope = try scope()
    let first = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    let second = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    let profile = try signedEvent(privateKey: key, kind: 0, content: #"{"name":"A"}"#)
    let channel = try signedEvent(
      privateKey: key, kind: 39000, tags: [["d", "room"], ["name", "General"]])
    try first.save(event: profile, scope: scope)
    try second.save(event: channel, scope: scope)
    #expect(
      try first.metadata(scope: scope, kind: 39000, author: channel.pubkey, identifier: "room")?
        .event == channel)
    #expect(try second.metadata(scope: scope, kind: 0, author: profile.pubkey)?.event == profile)
  }

  @Test("Concurrent notification writers preserve every independent record")
  func concurrentWriters() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scope = try scope()
    let events = try (0..<24).map { index in
      try signedEvent(
        privateKey: key, kind: 39000,
        tags: [["d", "room-\(index)"], ["name", "Room \(index)"]])
    }
    let errorLock = NSLock()
    var failures: [String] = []
    DispatchQueue.concurrentPerform(iterations: events.count) { index in
      let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
      do {
        try cache.save(event: events[index], scope: scope)
        let record = try cache.metadata(
          scope: scope, kind: 39000,
          author: events[index].pubkey, identifier: "room-\(index)")
        if record?.event != events[index] {
          errorLock.lock()
          failures.append("Missing record \(index)")
          errorLock.unlock()
        }
      } catch {
        errorLock.lock()
        failures.append(String(describing: error))
        errorLock.unlock()
      }
    }
    #expect(failures.isEmpty)
    let reopened = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    for (index, event) in events.enumerated() {
      #expect(
        try reopened.metadata(
          scope: scope, kind: 39000,
          author: event.pubkey, identifier: "room-\(index)")?.event == event)
    }
  }

  @Test("A failed batch cannot publish only its channel metadata")
  func atomicBatchFailure() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scope = try scope()
    let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    let channel = try signedEvent(
      privateKey: key, createdAt: 1, kind: 39000, tags: [["d", "room"]])
    let members = try signedEvent(
      privateKey: key, createdAt: 1, kind: 39002, tags: [["d", "room"]])
    try cache.save(events: [channel, members], scope: scope)
    let newer = try signedEvent(privateKey: key, createdAt: 2, kind: 39000, tags: [["d", "room"]])
    let invalid = try signedEvent(privateKey: key, createdAt: 2, kind: 9)
    #expect(throws: BuzzNotificationCacheError.self) {
      try cache.save(events: [newer, invalid], scope: scope)
    }
    let snapshot = try cache.presentation(
      scope: scope, sender: channel.pubkey,
      channelID: "room", authority: channel.pubkey)
    #expect(snapshot.channel?.event == channel)
    #expect(snapshot.membership?.event == members)
  }

  @Test("Partial channel refreshes cannot reuse an old counterpart")
  func partialChannelRefresh() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scope = try scope()
    let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    for refreshedKind in [39000, 39002] {
      let old = try [39000, 39002].map {
        try signedEvent(privateKey: key, createdAt: 1, kind: $0, tags: [["d", "room-\(refreshedKind)"]])
      }
      try cache.saveChannel(channel: old[0], membership: old[1], scope: scope)
      let newer = try signedEvent(
        privateKey: key, createdAt: 2, kind: refreshedKind,
        tags: [["d", "room-\(refreshedKind)"]])
      try cache.saveChannel(
        channel: refreshedKind == 39000 ? newer : nil,
        membership: refreshedKind == 39002 ? newer : nil, scope: scope)
      let snapshot = try cache.presentation(
        scope: scope, sender: newer.pubkey,
        channelID: "room-\(refreshedKind)", authority: newer.pubkey)
      #expect(snapshot.channel?.event == (refreshedKind == 39000 ? newer : nil))
      #expect(snapshot.membership?.event == (refreshedKind == 39002 ? newer : nil))
    }
  }

  @Test("Older channel responses cannot invalidate or partly replace a newer pair")
  func olderChannelRefresh() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scope = try scope()
    let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    let current = try [39000, 39002].map {
      try signedEvent(privateKey: key, createdAt: 2, kind: $0, tags: [["d", "room"]])
    }
    try cache.saveChannel(channel: current[0], membership: current[1], scope: scope)
    let older = try signedEvent(privateKey: key, createdAt: 1, kind: 39000, tags: [["d", "room"]])
    let newer = try signedEvent(privateKey: key, createdAt: 3, kind: 39002, tags: [["d", "room"]])
    try cache.saveChannel(channel: older, membership: nil, scope: scope)
    try cache.saveChannel(channel: older, membership: newer, scope: scope)
    try cache.saveChannel(channel: nil, membership: nil, scope: scope)
    let snapshot = try cache.presentation(
      scope: scope, sender: current[0].pubkey, channelID: "room", authority: current[0].pubkey)
    #expect(snapshot.channel?.event == current[0])
    #expect(snapshot.membership?.event == current[1])
  }

  @Test("Concurrent channel readers see one complete refresh")
  func atomicBatchReaders() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scope = try scope()
    let pairs = try (1...20).map { version in
      try [39000, 39002].map { kind in
        try signedEvent(privateKey: key, createdAt: version, kind: kind, tags: [["d", "room"]])
      }
    }
    let errorsLock = NSLock()
    var errors: [String] = []
    DispatchQueue.concurrentPerform(iterations: pairs.count) { index in
      let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
      do {
        try cache.save(events: pairs[index], scope: scope)
        let snapshot = try cache.presentation(
          scope: scope, sender: pairs[index][0].pubkey,
          channelID: "room", authority: pairs[index][0].pubkey)
        if snapshot.channel?.event.createdAt != snapshot.membership?.event.createdAt {
          errorsLock.lock()
          errors.append("Mixed channel refresh")
          errorsLock.unlock()
        }
      } catch {
        errorsLock.lock()
        errors.append(String(describing: error))
        errorsLock.unlock()
      }
    }
    #expect(errors.isEmpty)
  }

  @Test("Reads do not extend freshness; a successful recheck does")
  func freshness() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scope = try scope()
    let event = try signedEvent(privateKey: key, kind: 0, content: #"{"name":"A"}"#)
    let first = BuzzNotificationCache(
      directory: root,
      now: { Date(timeIntervalSince1970: 100) }, isCurrent: { $0 == scope })
    try first.save(event: event, scope: scope)
    let cached = try #require(try first.metadata(scope: scope, kind: 0, author: event.pubkey))
    #expect(cached.isFresh(at: Date(timeIntervalSince1970: 3699)))
    #expect(!cached.isFresh(at: Date(timeIntervalSince1970: 3700)))
    let next = BuzzNotificationCache(
      directory: root,
      now: { Date(timeIntervalSince1970: 4000) }, isCurrent: { $0 == scope })
    #expect(
      try next.metadata(scope: scope, kind: 0, author: event.pubkey)?.checkedAt == cached.checkedAt)
    try next.save(event: event, scope: scope)
    #expect(
      try next.metadata(scope: scope, kind: 0, author: event.pubkey)?.checkedAt
        == Date(timeIntervalSince1970: 4000))
  }

  @Test("Removed account is inaccessible and a new sign-in cannot reuse its cache")
  func removedAccount() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let old = try scope()
    let new = try scope("login-two")
    let event = try signedEvent(privateKey: key, kind: 0, content: #"{"name":"A"}"#)
    try BuzzNotificationCache(directory: root, isCurrent: { $0 == old }).save(
      event: event, scope: old)
    let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == new })
    #expect(throws: BuzzNotificationCacheError.self) {
      try cache.metadata(scope: old, kind: 0, author: event.pubkey)
    }
    #expect(throws: BuzzNotificationCacheError.self) { try cache.save(event: event, scope: old) }
    #expect(try cache.metadata(scope: new, kind: 0, author: event.pubkey) == nil)
    try cache.removeRevokedScopes(keeping: [new])
    #expect(
      !FileManager.default.fileExists(atPath: root.appendingPathComponent(old.directoryName).path))
  }

  @Test("Incomplete cache file is reported, removed, and can be fetched again")
  func interruptedWriteRecovery() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scope = try scope()
    let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    let event = try signedEvent(privateKey: key, kind: 0, content: #"{"name":"A"}"#)
    try cache.save(event: event, scope: scope)
    let file = root.appendingPathComponent(scope.directoryName).appendingPathComponent(
      "metadata.json")
    try Data("{broken".utf8).write(to: file)
    #expect(throws: BuzzNotificationCacheError.self) {
      try cache.metadata(scope: scope, kind: 0, author: event.pubkey)
    }
    try cache.save(event: event, scope: scope)
    #expect(try cache.metadata(scope: scope, kind: 0, author: event.pubkey)?.event == event)
  }

  @Test("Bootstrap keeps a session across updates but renews it after removal")
  func bootstrapSession() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let bootstrap = BuzzPushPresentationCacheStore(containerURL: root)
    let community = PushLeaseCommunity(
      id: "community", name: "Test", relayUrl: "wss://relay.example",
      pubkey: "account", policies: [])
    func session() throws -> String? {
      let data = try Data(
        contentsOf: root.appendingPathComponent(BuzzPushPresentationCacheStore.fileName))
      return BuzzPushPresentationCacheSnapshot.decode(data).communities.first?.notificationSessionID
    }
    try bootstrap.replaceCommunities([community])
    let first = try #require(try session())
    try bootstrap.replaceCommunities([community])
    #expect(try session() == first)
    try bootstrap.replaceCommunities([])
    try bootstrap.replaceCommunities([community])
    #expect(try session() != first)
  }

  private func signedEvent(
    privateKey: String,
    createdAt: Int = 1_700_000_000,
    kind: Int,
    tags: [[String]] = [],
    content: String = ""
  ) throws -> VerifiedNostrEvent {
    let privateKeyBytes = try #require(VerifiedNostrEvent.hexBytes(privateKey))
    let key = try P256K.Schnorr.PrivateKey(dataRepresentation: privateKeyBytes)
    let pubkey = VerifiedNostrEvent.hex(key.xonly.bytes)
    let serialization = try VerifiedNostrEvent.canonicalSerialization(
      pubkey: pubkey,
      createdAt: createdAt,
      kind: kind,
      tags: tags,
      content: content
    )
    let digest = Array(SHA256.hash(data: serialization))
    var message = digest
    var randomness = [UInt8](repeating: UInt8(truncatingIfNeeded: createdAt), count: 32)
    let signature = try key.signature(message: &message, auxiliaryRand: &randomness)
    return VerifiedNostrEvent(
      id: VerifiedNostrEvent.hex(digest),
      pubkey: pubkey,
      createdAt: createdAt,
      kind: kind,
      tags: tags,
      content: content,
      sig: VerifiedNostrEvent.hex(signature.dataRepresentation)
    )
  }
}
