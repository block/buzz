import CryptoKit
import Foundation
import P256K
import XCTest

@testable import BuzzPushKit

final class BuzzNotificationEnrichmentOperationTests: XCTestCase {
  func testLateProfileStartsAvatarFetchButDoesNotDeliverAgain() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let message = try signedEvent(kind: 9, content: "Hello")
    let profile = try signedEvent(
      kind: 0,
      content: #"{"name":"Alice","picture":"https://images.example/avatar.png"}"#)
    let community = PushLeaseCommunity(
      id: "community", name: "Test",
      relayUrl: "wss://relay.example", pubkey: message.pubkey, policies: [],
      notificationSessionID: "session")
    let scope = try XCTUnwrap(BuzzNotificationCacheScope(community: community))
    let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    let delivered = expectation(description: "Delivered at deadline")
    delivered.assertForOverFulfill = true
    let warmed = expectation(description: "Late image saved")
    let image = Data([1, 2, 3])
    var deliveryCount = 0
    let operation = BuzzNotificationEnrichmentOperation(
      event: message, community: community, scope: scope, cache: cache, avatarLoader: nil,
      now: Date.init, isCurrent: { true },
      loadAvatar: { source, receivedScope, completion in
        XCTAssertEqual(source, "https://images.example/avatar.png")
        XCTAssertEqual(receivedScope, scope)
        XCTAssertEqual(deliveryCount, 1)
        completion(.success(image))
      },
      scheduleDeadline: { queue, deadline in
        queue.async(execute: DispatchWorkItem(block: deadline))
      },
      refresh: { needsProfile, _, completion in
        XCTAssertTrue(needsProfile)
        // The deadline is already queued. This callback queues processing after it.
        completion(profile, nil, nil)
      },
      completion: { resolution in
        deliveryCount += 1
        XCTAssertEqual(resolution?.title, "")
        delivered.fulfill()
      })
    operation.start()
    wait(for: [delivered], timeout: 2)
    // Observe persistent state, rather than requiring the process to stay alive
    // for a specific delay after the content handler has run.
    DispatchQueue.global().async {
      for _ in 0..<200 {
        if (try? cache.avatar(scope: scope, pubkey: message.pubkey))?.data == image {
          warmed.fulfill()
          return
        }
        Thread.sleep(forTimeInterval: 0.005)
      }
    }
    wait(for: [warmed], timeout: 2)
    XCTAssertEqual(deliveryCount, 1)
  }

  func testRevocationBeforeDeadlineSuppressesPresentationAndLateWrites() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let message = try signedEvent(kind: 9, content: "Hello")
    let profile = try signedEvent(kind: 0, content: #"{"name":"Alice"}"#)
    let community = PushLeaseCommunity(
      id: "community", name: "Test",
      relayUrl: "wss://relay.example", pubkey: message.pubkey, policies: [],
      notificationSessionID: "session")
    let scope = try XCTUnwrap(BuzzNotificationCacheScope(community: community))
    let accessLock = NSLock()
    var current = true
    func isCurrent() -> Bool {
      accessLock.lock()
      defer { accessLock.unlock() }
      return current
    }
    let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope && isCurrent() })
    let delivered = expectation(description: "Revoked account has no presentation")
    delivered.assertForOverFulfill = true
    let finished = expectation(description: "Late callback processed")
    var operationQueue: DispatchQueue?
    var deadlineAction: (() -> Void)?
    let operation = BuzzNotificationEnrichmentOperation(
      event: message, community: community, scope: scope, cache: cache, avatarLoader: nil,
      now: Date.init, isCurrent: isCurrent,
      scheduleDeadline: { queue, deadline in
        operationQueue = queue
        deadlineAction = deadline
      },
      refresh: { _, _, completion in
        accessLock.lock()
        current = false
        accessLock.unlock()
        if let deadlineAction {
          operationQueue?.async(execute: DispatchWorkItem(block: deadlineAction))
        }
        completion(profile, nil, nil)
        operationQueue?.async { finished.fulfill() }
      },
      completion: { resolution in
        XCTAssertNil(resolution)
        delivered.fulfill()
      })
    operation.start()
    wait(for: [delivered, finished], timeout: 2)
    // Inspect the old scope's disk contents with an independent reader. Revocation
    // must have prevented the late result from being persisted at all.
    let reader = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    XCTAssertNil(try reader.metadata(scope: scope, kind: 0, author: message.pubkey))
  }

  func testExpiredAvatarAtSameURLIsCheckedAndOldImageSurvivesFailure() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let message = try signedEvent(kind: 9, content: "Hello")
    let source = "https://images.example/avatar.png"
    let profile = try signedEvent(
      kind: 0,
      content: #"{"name":"Alice","picture":"https://images.example/avatar.png"}"#)
    let community = PushLeaseCommunity(
      id: "community", name: "Test",
      relayUrl: "wss://relay.example", pubkey: message.pubkey, policies: [],
      notificationSessionID: "session")
    let scope = try XCTUnwrap(BuzzNotificationCacheScope(community: community))
    let oldCache = BuzzNotificationCache(
      directory: root,
      now: { Date(timeIntervalSince1970: 100) }, isCurrent: { $0 == scope })
    try oldCache.save(event: profile, scope: scope)
    let oldImage = Data([1, 2, 3])
    try oldCache.saveAvatar(
      oldImage, source: source, profileID: profile.id,
      pubkey: message.pubkey, scope: scope)
    let now = Date(timeIntervalSince1970: 3700)
    let cache = BuzzNotificationCache(directory: root, now: { now }, isCurrent: { $0 == scope })
    // The profile itself is fresh, but its unchanged URL must not make the old
    // decoded image permanently fresh.
    try cache.save(event: profile, scope: scope)
    let checked = expectation(description: "Same URL checked again")
    let delivered = expectation(description: "Old image retained")
    let operation = BuzzNotificationEnrichmentOperation(
      event: message, community: community, scope: scope, cache: cache, avatarLoader: nil,
      now: { now }, isCurrent: { true },
      loadAvatar: { url, _, completion in
        XCTAssertEqual(url, source)
        checked.fulfill()
        completion(.failure(URLError(.notConnectedToInternet)))
      },
      refresh: { _, _, _ in XCTFail("Fresh profile must not trigger a metadata request") },
      completion: { result in
        XCTAssertEqual(result?.senderAvatarPNG, oldImage)
        delivered.fulfill()
      })
    operation.start()
    wait(for: [checked, delivered], timeout: 2)
    XCTAssertEqual(
      try cache.avatar(scope: scope, pubkey: message.pubkey)?.savedAt,
      Date(timeIntervalSince1970: 100))
  }

  private func signedEvent(kind: Int, content: String) throws -> VerifiedNostrEvent {
    let key = try P256K.Schnorr.PrivateKey(
      dataRepresentation: Array(repeating: UInt8(1), count: 32))
    let pubkey = VerifiedNostrEvent.hex(key.xonly.bytes)
    let serialization = try VerifiedNostrEvent.canonicalSerialization(
      pubkey: pubkey, createdAt: 1, kind: kind, tags: [], content: content)
    let digest = Array(SHA256.hash(data: serialization))
    var message = digest
    var randomness = Array(repeating: UInt8(2), count: 32)
    let signature = try key.signature(message: &message, auxiliaryRand: &randomness)
    return VerifiedNostrEvent(
      id: VerifiedNostrEvent.hex(digest), pubkey: pubkey,
      createdAt: 1, kind: kind, tags: [], content: content,
      sig: VerifiedNostrEvent.hex(signature.dataRepresentation))
  }
}
