import CryptoKit
import Foundation
import P256K
import XCTest

@testable import BuzzPushKit

#if canImport(FoundationNetworking)
  import FoundationNetworking
#endif

final class BuzzPushNotificationResolverTests: XCTestCase {
  static let privateKey = String(repeating: "0", count: 63) + "1"
  static let ownPubkey =
    "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
  static let now = Int(Date().timeIntervalSince1970)
  static let profilePrivateKey = String(repeating: "0", count: 63) + "2"
  static let relayPrivateKey = String(repeating: "0", count: 63) + "3"
  static let gatewayBody = "Reconnect to your relay now"
  static let channelID = "123e4567-e89b-42d3-a456-426614174000"
  static let unnamedSenderHex =
    "aa4fc8665f5696e33db7e1a572e3b0f5b3d615837b0f362dcb1c8068b098c7b4"
  static let unnamedSenderNpub =
    "npub14f8usejl26twx0dhuxjh9cas7keav9vr0v8nvtwtrjqx3vycc76qqh9nsy"

  override func setUp() {
    super.setUp()
    URLProtocolStub.reset()
  }

  override func tearDown() {
    URLProtocolStub.reset()
    super.tearDown()
  }

  func testResolveReturnsNilWhenCommunitiesDataIsMissing() {
    let result = resolve(makeResolver(communitiesData: nil))

    XCTAssertNil(result)
    XCTAssertTrue(URLProtocolStub.requests.isEmpty)
  }

  func testResolveReturnsNilWhenCommunitiesDataIsUndecodable() {
    let result = resolve(makeResolver(communitiesData: Data("not json".utf8)))

    XCTAssertNil(result)
    XCTAssertTrue(URLProtocolStub.requests.isEmpty)
  }

  func testResolveReturnsNilOnKeychainMiss() throws {
    let result = resolve(
      makeResolver(
        communitiesData: try snapshotData([community()]),
        privateKeys: [:]
      ))

    XCTAssertNil(result)
    XCTAssertTrue(URLProtocolStub.requests.isEmpty)
  }

  func testInitialAccountFanoutReadsSnapshotOnce() throws {
    let communities = (0..<64).map { community(id: "community-\($0)") }
    let snapshot = try snapshotData(communities)
    let lock = NSLock()
    var reads = 0
    let started = expectation(description: "All community requests started")
    started.expectedFulfillmentCount = communities.count
    let finished = expectation(description: "Cancelled requests complete resolution")
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [URLProtocolStub.self]
    let session = URLSession(configuration: configuration)
    URLProtocolStub.handler = { request in
      started.fulfill()
      return Self.response(request, status: 200, data: Data("[]".utf8))
    }
    URLProtocolStub.finishLoading = { _ in false }
    let resolver = BuzzPushNotificationResolver(session: session,
      loadCommunitiesData: {
        lock.lock()
        reads += 1
        lock.unlock()
        return snapshot
      }, loadPrivateKey: { _ in Self.privateKey })
    resolver.resolve { result in
      XCTAssertNil(result)
      finished.fulfill()
    }
    wait(for: [started], timeout: 3)
    lock.lock()
    let initialReads = reads
    lock.unlock()
    XCTAssertEqual(initialReads, 1)
    session.invalidateAndCancel()
    wait(for: [finished], timeout: 3)
  }

  func testSnapshotAccountMustMatchKeychainBeforeAnyRequest() throws {
    let configured = community(pubkey: try Self.pubkey(for: Self.profilePrivateKey))
    URLProtocolStub.handler = { request in
      XCTFail("Mismatched account key must not issue a request")
      return Self.response(request, status: 200, data: Data("[]".utf8))
    }
    XCTAssertNil(resolve(makeResolver(communitiesData: try snapshotData([configured]))))
    XCTAssertTrue(URLProtocolStub.requests.isEmpty)
  }

  func testKeyReplacementWhileRequestIsInFlightRejectsItsResult() throws {
    let data = try snapshotData([community()])
    let lock = NSLock()
    var key = Self.privateKey
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [URLProtocolStub.self]
    let message = try Self.signedEvent(privateKey: Self.profilePrivateKey,
      createdAt: Self.now, kind: 9, tags: [["h", Self.channelID]], content: "Old account result")
    URLProtocolStub.handler = { request in
      lock.lock()
      key = Self.profilePrivateKey
      lock.unlock()
      return Self.response(request, status: 200, data: try JSONEncoder().encode([message]))
    }
    let resolver = BuzzPushNotificationResolver(
      session: URLSession(configuration: configuration), loadCommunitiesData: { data },
      loadPrivateKey: { _ in
        lock.lock()
        defer { lock.unlock() }
        return key
      })
    XCTAssertNil(resolve(resolver))
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testResolveReturnsNilForNon2xxRelayResponse() throws {
    URLProtocolStub.handler = { request in
      Self.response(request, status: 503, data: Data())
    }
    let result = resolve(makeResolver(communitiesData: try snapshotData([community()])))

    XCTAssertNil(result)
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testResolveReturnsNilForUndecodableRelayResponse() throws {
    URLProtocolStub.handler = { request in
      Self.response(request, status: 200, data: Data("not events".utf8))
    }
    let result = resolve(makeResolver(communitiesData: try snapshotData([community()])))

    XCTAssertNil(result)
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testDecodeResolutionFiltersOwnPubkeyEvent() {
    let result = BuzzPushNotificationResolver.decodeResolution(
      events: [event(pubkey: Self.ownPubkey, content: "This should be filtered")],
      community: community()
    )

    XCTAssertNil(result)
  }

  func testDecodeResolutionReturnsNilWhenSanitizedPreviewIsEmpty() {
    let event = event(content: "  \n\t  ")

    let result = BuzzPushNotificationResolver.decodeResolution(
      events: [event],
      community: community()
    )

    XCTAssertNil(result)
  }

  func testPreviewBodySanitizesCodeLinksAndWhitespace() {
    let content = """
      Before   ```swift
      print("secret")
      ``` `inline` [docs](https://example.com/docs)
      ![image](https://example.com/image.png) https://example.com/raw
      After
      """

    XCTAssertEqual(
      BuzzPushNotificationResolver.previewBody(content),
      "Before [code] inline docs image [link] After"
    )
  }

  func testPreviewBodyTruncatesTo178CharactersIncludingEllipsis() {
    let preview = BuzzPushNotificationResolver.previewBody(String(repeating: "x", count: 200))

    XCTAssertEqual(preview.count, 178)
    XCTAssertEqual(preview, String(repeating: "x", count: 177) + "…")
  }

  func testDecodeResolutionUsesLowestIDWhenCreatedAtTies() {
    let result = BuzzPushNotificationResolver.decodeResolution(
      events: [
        event(id: "a", content: "lower ID", createdAt: Self.now),
        event(id: "b", content: "higher ID", createdAt: Self.now),
      ],
      community: community()
    )

    XCTAssertEqual(result?.1.id, "a")
    XCTAssertEqual(result?.0.body, "lower ID")
  }

  func testMissingSenderNameOmitsLabelWithoutExposingKey() {
    // Keep identity in the routing payload, but do not use it as a display name.
    let senders: [(pubkey: String, title: String)] = [
      (Self.unnamedSenderHex, ""),
      (Self.unnamedSenderNpub, ""),
      ("author-pubkey", ""),
      (String(repeating: "+a", count: 32), ""),
    ]
    for sender in senders {
      let result = BuzzPushNotificationResolver.decodeResolution(
        events: [event(pubkey: sender.pubkey, content: "Preview")],
        community: community()
      )

      XCTAssertEqual(result?.0.title, sender.title, "pubkey: \(sender.pubkey)")
      XCTAssertEqual(result?.0.body, "Preview", "pubkey: \(sender.pubkey)")
      XCTAssertEqual(result?.0.subtitle, "Community", "pubkey: \(sender.pubkey)")
      XCTAssertEqual(result?.0.senderPubkey, sender.pubkey, "pubkey: \(sender.pubkey)")
      XCTAssertEqual(result?.0.threadIdentifier, "community-id", "pubkey: \(sender.pubkey)")
    }
  }

  func testReplyNavigationUsesDirectParentAndKeepsExactEvent() throws {
    let parent = String(repeating: "a", count: 64)
    let reply = event(id: "reply-id", content: "Reply preview", tags: [
      ["h", Self.channelID],
      ["e", "outer-root", "", "root"],
      ["e", parent, "", "reply"],
      ["e", "bad", "", "reply"],
      ["e", "", "", "reply"],
    ])
    let resolution = try XCTUnwrap(BuzzPushNotificationResolver.decodeResolution(
      events: [reply], community: community()
    )?.0)
    let target = try XCTUnwrap(resolution.navigationTarget)
    XCTAssertEqual(target.eventID, "reply-id")
    XCTAssertEqual(target.userInfoValue["thread_root_id"], parent)
  }

  func testNavigationKeepsTopLevelAndBroadcastMessagesInChannel() throws {
    for tags in [
      [["h", Self.channelID]],
      [["h", Self.channelID], ["e", "mention", "", "mention"]],
      [["h", Self.channelID], ["e", "root", "", "root"]],
      [["h", Self.channelID], ["e", "parent", "", "reply"], ["broadcast", "1"]],
    ] {
      let resolution = try XCTUnwrap(BuzzPushNotificationResolver.decodeResolution(
        events: [event(content: "Preview", tags: tags)], community: community()
      )?.0)
      XCTAssertNil(resolution.navigationTarget?.threadRootID)
    }
  }

  func testResolveSucceedsAndMutatesGatewayContent() throws {
    let event = try JSONDecoder().decode(
      VerifiedNostrEvent.self,
      from: Data(Self.fixtureEvent.utf8)
    )
    URLProtocolStub.handler = { request in
      Self.response(request, status: 200, data: try JSONEncoder().encode([event]))
    }

    let result = try XCTUnwrap(
      resolve(
        makeResolver(
          communitiesData: try snapshotData([community()])
        )))

    XCTAssertNotEqual(result.title, Self.gatewayBody)
    XCTAssertNotEqual(result.body, Self.gatewayBody)
    XCTAssertEqual(result.title, "")
    XCTAssertEqual(result.body, "Hello Buzz")
    XCTAssertEqual(result.subtitle, "Community")
    XCTAssertEqual(
      result.threadIdentifier,
      BuzzPushPresentationIdentity.conversation(
        communityID: "community-id",
        channelID: Self.channelID
      )
    )
    XCTAssertEqual(result.senderPubkey, event.pubkey)
    XCTAssertEqual(
      result.navigationTarget,
      BuzzPushNavigationTarget(
        eventID: event.id,
        communityID: "community-id",
        channelID: Self.channelID
      )
    )
  }

  func testFreshVerifiedCacheResolvesSenderAvatarAndChannelWithoutRefresh() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Hello from Alice"
    )
    let relayPubkey = try Self.pubkey(for: Self.relayPrivateKey)
    let avatar = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    let snapshot = BuzzPushPresentationCacheSnapshot(
      profiles: [
        BuzzPushCachedProfile(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          pubkey: message.pubkey,
          displayName: "Alice",
          pictureHash: "picture-hash",
          avatarPNG: avatar,
          eventID: "profile-event",
          eventCreatedAt: Self.now,
          cachedAt: Self.now
        )
      ],
      channels: [
        BuzzPushCachedChannel(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          channelID: Self.channelID,
          relayMetadataPubkey: relayPubkey,
          displayName: "General",
          channelType: "stream",
          memberCount: 2,
          memberDigests: Self.memberDigests([Self.ownPubkey, message.pubkey]),
          membershipEventID: "membership-event",
          membershipEventCreatedAt: Self.now,
          membershipCachedAt: Self.now,
          eventID: "channel-event",
          eventCreatedAt: Self.now,
          cachedAt: Self.now
        )
      ]
    )
    URLProtocolStub.handler = { request in
      Self.response(request, status: 200, data: try JSONEncoder().encode([message]))
    }

    let result = try XCTUnwrap(
      resolve(
        makeResolver(
          communitiesData: try snapshotData([community(relayMetadataPubkey: relayPubkey)]),
          presentationCacheData: try JSONEncoder().encode(snapshot),
          now: Date(timeIntervalSince1970: TimeInterval(Self.now))
        )
      )
    )

    XCTAssertEqual(result.title, "Alice")
    XCTAssertEqual(result.senderAvatarPNG, avatar)
    XCTAssertEqual(result.conversationDisplayName, "#General")
    XCTAssertEqual(result.conversationRecipientCount, 1)
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testStaleVerifiedCacheIsUsedWhileBoundedRefreshesFail() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Stale cache still presents"
    )
    let relayPubkey = try Self.pubkey(for: Self.relayPrivateKey)
    let staleAt = Self.now - Int(BuzzPushPresentationCacheStore.freshnessLifetime) - 1
    let snapshot = BuzzPushPresentationCacheSnapshot(
      profiles: [
        BuzzPushCachedProfile(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          pubkey: message.pubkey,
          displayName: "Stale Alice",
          pictureHash: nil,
          avatarPNG: nil,
          eventID: "profile-event",
          eventCreatedAt: staleAt,
          cachedAt: staleAt
        )
      ],
      channels: [
        BuzzPushCachedChannel(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          channelID: Self.channelID,
          relayMetadataPubkey: relayPubkey,
          displayName: "Stale General",
          channelType: "stream",
          memberCount: 2,
          memberDigests: Self.memberDigests([Self.ownPubkey, message.pubkey]),
          membershipEventID: "membership-event",
          membershipEventCreatedAt: staleAt,
          membershipCachedAt: staleAt,
          eventID: "channel-event",
          eventCreatedAt: staleAt,
          cachedAt: staleAt
        )
      ]
    )
    URLProtocolStub.handler = { request in
      if URLProtocolStub.requests.count == 1 {
        return Self.response(request, status: 200, data: try JSONEncoder().encode([message]))
      }
      XCTAssertEqual(request.timeoutInterval, 60)
      return Self.response(request, status: 503, data: Data())
    }

    let result = try XCTUnwrap(
      resolve(
        makeResolver(
          communitiesData: try snapshotData([community(relayMetadataPubkey: relayPubkey)]),
          presentationCacheData: try JSONEncoder().encode(snapshot),
          now: Date(timeIntervalSince1970: TimeInterval(Self.now))
        )
      )
    )

    XCTAssertEqual(result.title, "Stale Alice")
    XCTAssertEqual(result.conversationDisplayName, "#Stale General")
    XCTAssertEqual(result.conversationRecipientCount, 1)
    XCTAssertEqual(URLProtocolStub.requests.count, 4)
  }

  func testOlderVerifiedRefreshCannotReplaceNewerStaleCache() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Keep newer cached metadata"
    )
    let olderProfile = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now - 20,
      kind: 0,
      content: #"{"display_name":"Older Alice"}"#
    )
    let olderChannel = try Self.signedEvent(
      privateKey: Self.relayPrivateKey,
      createdAt: Self.now - 20,
      kind: 39_000,
      tags: [["d", Self.channelID], ["name", "Older General"]]
    )
    let staleAt = Self.now - Int(BuzzPushPresentationCacheStore.freshnessLifetime) - 1
    let snapshot = BuzzPushPresentationCacheSnapshot(
      profiles: [
        BuzzPushCachedProfile(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          pubkey: message.pubkey,
          displayName: "Newer Cached Alice",
          pictureHash: nil,
          avatarPNG: nil,
          eventID: String(repeating: "f", count: 64),
          eventCreatedAt: Self.now - 10,
          cachedAt: staleAt
        )
      ],
      channels: [
        BuzzPushCachedChannel(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          channelID: Self.channelID,
          relayMetadataPubkey: olderChannel.pubkey,
          displayName: "Newer Cached General",
          channelType: "stream",
          memberCount: 2,
          memberDigests: Self.memberDigests([Self.ownPubkey, message.pubkey]),
          membershipEventID: "cached-membership",
          membershipEventCreatedAt: Self.now - 10,
          membershipCachedAt: staleAt,
          eventID: String(repeating: "f", count: 64),
          eventCreatedAt: Self.now - 10,
          cachedAt: staleAt
        )
      ]
    )
    URLProtocolStub.handler = { request in
      if URLProtocolStub.requests.count == 1 {
        return Self.response(request, status: 200, data: try JSONEncoder().encode([message]))
      }
      return Self.response(
        request,
        status: 200,
        data: try JSONEncoder().encode([olderProfile, olderChannel])
      )
    }

    let result = try XCTUnwrap(
      resolve(
        makeResolver(
          communitiesData: try snapshotData([
            community(relayMetadataPubkey: olderChannel.pubkey)
          ]),
          presentationCacheData: try JSONEncoder().encode(snapshot),
          now: Date(timeIntervalSince1970: TimeInterval(Self.now))
        )
      )
    )

    XCTAssertEqual(result.title, "Newer Cached Alice")
    XCTAssertEqual(result.conversationDisplayName, "#Newer Cached General")
  }

  func testChannelOnlyRefreshIgnoresUnrequestedProfileEvent() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Ignore unrelated enrichment"
    )
    let unexpectedProfile = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now + 1,
      kind: 0,
      content: #"{"display_name":"Unexpected Alice"}"#
    )
    let relayMetadataPubkey = try Self.pubkey(for: Self.relayPrivateKey)
    let staleAt = Self.now - Int(BuzzPushPresentationCacheStore.freshnessLifetime) - 1
    let snapshot = BuzzPushPresentationCacheSnapshot(
      profiles: [
        BuzzPushCachedProfile(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          pubkey: message.pubkey,
          displayName: "Cached Alice",
          pictureHash: nil,
          avatarPNG: nil,
          eventID: "cached-profile",
          eventCreatedAt: Self.now,
          cachedAt: Self.now
        )
      ],
      channels: [
        BuzzPushCachedChannel(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          channelID: Self.channelID,
          relayMetadataPubkey: relayMetadataPubkey,
          displayName: "Stale General",
          channelType: "stream",
          memberCount: 2,
          memberDigests: Self.memberDigests([Self.ownPubkey, message.pubkey]),
          membershipEventID: "cached-membership",
          membershipEventCreatedAt: Self.now,
          membershipCachedAt: Self.now,
          eventID: "cached-channel",
          eventCreatedAt: Self.now,
          cachedAt: staleAt
        )
      ]
    )
    URLProtocolStub.handler = { request in
      if URLProtocolStub.requests.count == 1 {
        return Self.response(request, status: 200, data: try JSONEncoder().encode([message]))
      }
      return Self.response(
        request,
        status: 200,
        data: try JSONEncoder().encode([unexpectedProfile])
      )
    }

    let result = try XCTUnwrap(
      resolve(
        makeResolver(
          communitiesData: try snapshotData([
            community(relayMetadataPubkey: relayMetadataPubkey)
          ]),
          presentationCacheData: try JSONEncoder().encode(snapshot),
          now: Date(timeIntervalSince1970: TimeInterval(Self.now))
        )
      )
    )

    XCTAssertEqual(result.title, "Cached Alice")
    XCTAssertEqual(result.conversationDisplayName, "#Stale General")
  }

  func testLegacyAccountSnapshotEnrichesBeforeFirstAppLaunchAfterUpgrade() throws {
    let message = try Self.signedEvent(privateKey: Self.profilePrivateKey,
      createdAt: Self.now, kind: 9, tags: [["h", Self.channelID]], content: "Hello before app launch")
    let profile = try Self.signedEvent(privateKey: Self.profilePrivateKey,
      createdAt: Self.now, kind: 0, content: #"{"name":"Alice"}"#)
    var legacy = community()
    legacy.notificationSessionID = nil
    URLProtocolStub.handler = { request in
      Self.response(request, status: 200,
        data: try JSONEncoder().encode(URLProtocolStub.requests.count == 1 ? [message] : [profile]))
    }
    let result = try XCTUnwrap(resolve(makeResolver(communitiesData: try snapshotData([legacy]))))
    XCTAssertEqual(result.title, "Alice")
    XCTAssertEqual(result.body, "Hello before app launch")
    XCTAssertNotNil(result.notificationScope)
    var newSignIn = legacy
    newSignIn.notificationSessionID = UUID().uuidString
    XCTAssertNotEqual(result.notificationScope, BuzzNotificationCacheScope(community: newSignIn))
  }

  func testMissingCacheRefreshesVerifiedProfileAndChannelIndependently() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Fresh metadata"
    )
    let profile = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 0,
      content: #"{"display_name":"Fresh Alice"}"#
    )
    let channel = try Self.signedEvent(
      privateKey: Self.relayPrivateKey,
      createdAt: Self.now,
      kind: 39_000,
      tags: [["d", Self.channelID], ["name", "Fresh General"], ["t", "stream"]]
    )
    let membership = try Self.signedEvent(
      privateKey: Self.relayPrivateKey,
      createdAt: Self.now,
      kind: 39_002,
      tags: [
        ["d", Self.channelID],
        ["p", Self.ownPubkey],
        ["p", message.pubkey],
      ]
    )
    URLProtocolStub.handler = { request in
      if URLProtocolStub.requests.count == 1 {
        return Self.response(request, status: 200, data: try JSONEncoder().encode([message]))
      }
      return Self.response(
        request,
        status: 200,
        data: try JSONEncoder().encode([profile, channel, membership])
      )
    }

    let result = try XCTUnwrap(
      resolve(
        makeResolver(
          communitiesData: try snapshotData([
            community(relayMetadataPubkey: channel.pubkey)
          ]),
          now: Date(timeIntervalSince1970: TimeInterval(Self.now))
        )
      )
    )

    XCTAssertEqual(result.title, "Fresh Alice")
    XCTAssertEqual(result.conversationDisplayName, "#Fresh General")
    XCTAssertEqual(result.conversationRecipientCount, 1)
    XCTAssertNil(result.senderAvatarPNG)
    XCTAssertEqual(URLProtocolStub.requests.count, 4)
  }

  func testMalformedAndUnverifiedRefreshFallsBackWithoutBlockingMessage() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Fallback content"
    )
    let validProfile = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 0,
      content: #"{"display_name":"Tampered"}"#
    )
    let tamperedProfile = VerifiedNostrEvent(
      id: validProfile.id,
      pubkey: validProfile.pubkey,
      createdAt: validProfile.createdAt,
      kind: validProfile.kind,
      tags: validProfile.tags,
      content: #"{"display_name":"Mallory"}"#,
      sig: validProfile.sig
    )
    URLProtocolStub.handler = { request in
      if URLProtocolStub.requests.count == 1 {
        return Self.response(request, status: 200, data: try JSONEncoder().encode([message]))
      }
      return Self.response(
        request,
        status: 200,
        data: try JSONEncoder().encode([tamperedProfile])
      )
    }

    let result = try XCTUnwrap(
      resolve(
        makeResolver(
          communitiesData: try snapshotData([
            community(relayMetadataPubkey: try Self.pubkey(for: Self.relayPrivateKey))
          ]),
          now: Date(timeIntervalSince1970: TimeInterval(Self.now))
        )
      )
    )

    XCTAssertEqual(result.title, "")
    XCTAssertNil(result.conversationDisplayName)
    XCTAssertEqual(result.subtitle, "Community")
    XCTAssertEqual(result.body, "Fallback content")
  }

  func testOversizedPresentationRefreshFallsBackWithoutBlockingMessage() throws {
    let cancelled = expectation(description: "Oversized streaming responses are cancelled")
    cancelled.expectedFulfillmentCount = 3
    URLProtocolStub.finishLoading = { _ in URLProtocolStub.requests.count == 1 }
    URLProtocolStub.onStopLoading = { _ in
      if URLProtocolStub.requests.count > 1 { cancelled.fulfill() }
    }
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Bounded fallback"
    )
    let profile = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 0,
      content: #"{"display_name":"Must Not Be Used"}"#
    )
    URLProtocolStub.handler = { request in
      if URLProtocolStub.requests.count == 1 {
        return Self.response(request, status: 200, data: try JSONEncoder().encode([message]))
      }
      var oversized = Data(
        repeating: 0x20,
        count: BuzzPushNotificationResolver.maximumPresentationResponseBytes
      )
      oversized.append(try JSONEncoder().encode([profile]))
      return Self.response(request, status: 200, data: oversized)
    }

    let result = try XCTUnwrap(
      resolve(
        makeResolver(
          communitiesData: try snapshotData([
            community(relayMetadataPubkey: try Self.pubkey(for: Self.relayPrivateKey))
          ]),
          now: Date(timeIntervalSince1970: TimeInterval(Self.now))
        )
      )
    )

    XCTAssertEqual(result.title, "")
    XCTAssertNil(result.conversationDisplayName)
    XCTAssertEqual(result.body, "Bounded fallback")
    XCTAssertEqual(URLProtocolStub.requests.count, 4)
    wait(for: [cancelled], timeout: 1)
  }

  func testLargeRosterCannotDiscardSenderProfileOrChannelMetadata() throws {
    let message = try Self.signedEvent(privateKey: Self.profilePrivateKey,
      createdAt: Self.now, kind: 9, tags: [["h", Self.channelID]], content: "Large channel")
    let profile = try Self.signedEvent(privateKey: Self.profilePrivateKey,
      createdAt: Self.now, kind: 0, content: #"{"display_name":"Fizz"}"#)
    let members = try Self.signedEvent(privateKey: Self.relayPrivateKey,
      createdAt: Self.now, kind: 39002,
      tags: [["d", Self.channelID]] + (0..<30000).map { ["p", String(format: "%064x", $0)] })
    let channel = try Self.signedEvent(privateKey: Self.relayPrivateKey,
      createdAt: Self.now, kind: 39000,
      tags: [["d", Self.channelID], ["name", "Large Channel"], ["t", "stream"]])
    let roster = try JSONEncoder().encode([profile, channel, members])
    XCTAssertGreaterThan(roster.count, BuzzPushNotificationResolver.maximumPresentationResponseBytes)
    URLProtocolStub.handler = { request in
      if URLProtocolStub.requests.count == 1 {
        return Self.response(request, status: 200, data: try JSONEncoder().encode([message]))
      }
      var body = request.httpBody ?? Data()
      if let stream = request.httpBodyStream {
        stream.open()
        defer { stream.close() }
        var buffer = [UInt8](repeating: 0, count: 4096)
        while stream.hasBytesAvailable {
          let count = stream.read(&buffer, maxLength: buffer.count)
          if count <= 0 { break }
          body.append(contentsOf: buffer.prefix(count))
        }
      }
      let filters = try XCTUnwrap(JSONSerialization.jsonObject(with: body) as? [[String: Any]])
      let kinds = filters.flatMap { $0["kinds"] as? [Int] ?? [] }
      return Self.response(request, status: 200,
        data: kinds.contains(39002) ? roster : try JSONEncoder().encode(kinds.contains(0) ? [profile] : [channel]))
    }
    let configured = community(relayMetadataPubkey: members.pubkey)
    let scope = try XCTUnwrap(BuzzNotificationCacheScope(community: configured))
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let cache = BuzzNotificationCache(directory: root, isCurrent: { $0 == scope })
    let result = try XCTUnwrap(resolve(makeResolver(
      communitiesData: try snapshotData([configured]), cacheOverride: cache,
      now: Date(timeIntervalSince1970: TimeInterval(Self.now)))))
    XCTAssertEqual(result.title, "Fizz")
    XCTAssertEqual(result.body, "Large channel")
    XCTAssertEqual(try cache.metadata(scope: scope, kind: 39000,
      author: channel.pubkey, identifier: Self.channelID)?.event, channel)
    // Group specialization still requires a verified complete recipient count.
    XCTAssertNil(result.conversationRecipientCount)
  }

  func testBoundedInlineAvatarProfileRefreshStillResolvesDisplayName() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Inline avatar profile"
    )
    let picture = "data:image/png;base64," + String(repeating: "A", count: 262_050)
    let profileContent = try XCTUnwrap(
      String(
        data: JSONSerialization.data(withJSONObject: [
          "display_name": "Fizz",
          "picture": picture,
        ]),
        encoding: .utf8
      )
    )
    let profile = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 0,
      content: profileContent
    )
    let channel = try Self.signedEvent(
      privateKey: Self.relayPrivateKey, createdAt: Self.now, kind: 39000,
      tags: [["d", Self.channelID], ["name", "Large Channel"], ["t", "stream"]],
      content: String(repeating: "\"", count: 256 * 1024)
    )
    let membership = try Self.signedEvent(
      privateKey: Self.relayPrivateKey, createdAt: Self.now, kind: 39002,
      tags: [["d", Self.channelID], ["p", Self.ownPubkey]],
      content: String(repeating: "\"", count: 256 * 1024)
    )
    let presentationData = try JSONEncoder().encode([profile, channel, membership])
    XCTAssertLessThanOrEqual(profileContent.utf8.count, 256 * 1024)
    XCTAssertGreaterThan(presentationData.count, 256 * 1024)
    XCTAssertLessThan(
      presentationData.count,
      BuzzPushNotificationResolver.maximumPresentationResponseBytes
    )
    URLProtocolStub.handler = { request in
      Self.response(
        request,
        status: 200,
        data: URLProtocolStub.requests.count == 1
          ? try JSONEncoder().encode([message]) : presentationData
      )
    }

    let result = try XCTUnwrap(
      resolve(
        makeResolver(
          communitiesData: try snapshotData([community(relayMetadataPubkey: channel.pubkey)]),
          now: Date(timeIntervalSince1970: TimeInterval(Self.now))
        )
      )
    )

    XCTAssertEqual(result.title, "Fizz")
    XCTAssertEqual(result.conversationDisplayName, "#Large Channel")
    XCTAssertEqual(result.conversationRecipientCount, 1)
    XCTAssertNil(result.senderAvatarPNG)
    XCTAssertEqual(result.body, "Inline avatar profile")
    XCTAssertEqual(URLProtocolStub.requests.count, 4)
  }

  func testResolveCanonicalizesWebSocketRelayOriginForQuery() throws {
    URLProtocolStub.handler = { request in
      XCTAssertEqual(request.url?.absoluteString, "https://relay.example/query")
      return Self.response(request, status: 200, data: Data("[]".utf8))
    }

    let result = resolve(
      makeResolver(
        communitiesData: try snapshotData([community(relayUrl: "wss://relay.example")])
      ))

    XCTAssertNil(result)
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func makeResolver(
    communitiesData: Data?,
    privateKeys: [String: String] = ["community-id": privateKey],
    presentationCacheData: Data? = nil,
    cacheOverride: BuzzNotificationCache? = nil,
    now: Date = Date()
  ) -> BuzzPushNotificationResolver {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [URLProtocolStub.self]
    return BuzzPushNotificationResolver(
      session: URLSession(configuration: configuration),
      loadCommunitiesData: { communitiesData },
      loadPrivateKey: { privateKeys[$0] },
      notificationCache: cacheOverride ?? makeNotificationCache(communitiesData: communitiesData, fixture: presentationCacheData, now: now),
      now: { now }
    )
  }

  private func makeNotificationCache(communitiesData: Data?, fixture: Data?, now: Date) -> BuzzNotificationCache {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    addTeardownBlock { try? FileManager.default.removeItem(at: root) }
    let communities = communitiesData.flatMap { try? JSONDecoder().decode(PushLeaseSnapshot.self, from: $0) }?.communities ?? []
    let scopes = communities.compactMap(BuzzNotificationCacheScope.init)
    let cache = BuzzNotificationCache(directory: root, now: { now }, isCurrent: { scopes.contains($0) })
    let snapshot = BuzzPushPresentationCacheSnapshot.decode(fixture)
    do {
      for profile in snapshot.profiles {
        let scope = try XCTUnwrap(scopes.first { $0.communityID == profile.communityID })
        var content = ["display_name": profile.displayName ?? ""]
        if profile.avatarPNG != nil { content["picture"] = "https://images.example/avatar.png" }
        let event = try Self.signedEvent(privateKey: Self.profilePrivateKey,
          createdAt: profile.eventCreatedAt, kind: 0,
          content: String(decoding: JSONEncoder().encode(content), as: UTF8.self))
        let writer = BuzzNotificationCache(directory: root,
          now: { Date(timeIntervalSince1970: Double(profile.cachedAt)) }, isCurrent: { scopes.contains($0) })
        try writer.save(event: event, scope: scope)
        if let image = profile.avatarPNG {
          let requestID = try XCTUnwrap(writer.beginAvatarRequest(
            profileID: event.id, pubkey: event.pubkey, scope: scope))
          try writer.saveAvatar(image, source: "https://images.example/avatar.png",
            profileID: event.id, requestID: requestID, pubkey: event.pubkey, scope: scope)
        }
      }
      for channel in snapshot.channels {
        let scope = try XCTUnwrap(scopes.first { $0.communityID == channel.communityID })
        let event = try Self.signedEvent(privateKey: Self.relayPrivateKey,
          createdAt: channel.eventCreatedAt, kind: 39000,
          tags: [["d", channel.channelID], ["name", channel.displayName ?? ""], ["t", channel.channelType ?? "stream"]])
        let writer = BuzzNotificationCache(directory: root,
          now: { Date(timeIntervalSince1970: Double(channel.cachedAt)) }, isCurrent: { scopes.contains($0) })
        try writer.save(event: event, scope: scope)
        guard let digests = channel.memberDigests else { continue }
        let candidates = [Self.ownPubkey, try Self.pubkey(for: Self.profilePrivateKey), try Self.pubkey(for: Self.relayPrivateKey)]
        let members = candidates.filter { candidate in
          Self.memberDigests([candidate]).contains { digests.contains($0) }
        }
        let membership = try Self.signedEvent(privateKey: Self.relayPrivateKey,
          createdAt: channel.membershipEventCreatedAt ?? channel.eventCreatedAt, kind: 39002,
          tags: [["d", channel.channelID]] + members.map { ["p", $0] })
        let membershipWriter = BuzzNotificationCache(directory: root,
          now: { Date(timeIntervalSince1970: Double(channel.membershipCachedAt ?? channel.cachedAt)) }, isCurrent: { scopes.contains($0) })
        try membershipWriter.save(event: membership, scope: scope)
      }
    } catch { XCTFail("Cannot seed verified cache: \(error)") }
    return cache
  }

  func resolve(_ resolver: BuzzPushNotificationResolver) -> BuzzPushResolution? {
    let completed = expectation(description: "resolver completed")
    var result: BuzzPushResolution?
    resolver.resolve {
      result = $0
      completed.fulfill()
    }
    wait(for: [completed], timeout: 2)
    return result
  }

  func community(
    id: String = "community-id",
    name: String = "Community",
    relayUrl: String = "https://relay.example",
    relayMetadataPubkey: String? = nil,
    pubkey: String? = ownPubkey
  ) -> PushLeaseCommunity {
    PushLeaseCommunity(
      id: id,
      name: name,
      relayUrl: relayUrl,
      relayMetadataPubkey: relayMetadataPubkey,
      pubkey: pubkey,
      policies: [
          PushResolutionPolicy(
            filter: PushLeaseFilter(
              kinds: [9, 40002, 45001, 45003],
              hTags: [Self.channelID]
            )
          )
        ],
      notificationSessionID: "test-session"
    )
  }

  func snapshotData(_ communities: [PushLeaseCommunity]) throws -> Data {
    try JSONEncoder().encode(PushLeaseSnapshot(communities: communities))
  }

  private func event(
    id: String = "event-id",
    pubkey: String = "author-pubkey",
    content: String,
    createdAt: Int = now,
    kind: Int = 9,
    tags: [[String]] = []
  ) -> VerifiedNostrEvent {
    VerifiedNostrEvent(
      id: id,
      pubkey: pubkey,
      createdAt: createdAt,
      kind: kind,
      tags: tags,
      content: content,
      sig: "signature"
    )
  }

  private static let fixtureEvent = #"""
    {"kind":9,"created_at":1785551670,"tags":[["h","123e4567-e89b-42d3-a456-426614174000"]],"content":"  Hello   [Buzz](https://buzz.block.xyz)  ","pubkey":"c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5","id":"233ccf24ec7c94808f9ef08b0c986b6df1bc3843ff72a9f8d016e2a77c77429b","sig":"d39dcd413839b872ed75a979b2c1542247fde636709966905c9e424e227a43897dc67b71ec84178a3faad0634f9bcdf0b48a56ebac84a2ac6e58124b8b6476e6"}
    """#

  static func response(
    _ request: URLRequest,
    status: Int,
    data: Data
  ) -> (HTTPURLResponse, Data) {
    let response = HTTPURLResponse(
      url: request.url!,
      statusCode: status,
      httpVersion: "HTTP/1.1",
      headerFields: ["Content-Type": "application/json"]
    )!
    return (response, data)
  }

  static func pubkey(for privateKey: String) throws -> String {
    let bytes = try XCTUnwrap(VerifiedNostrEvent.hexBytes(privateKey))
    let key = try P256K.Schnorr.PrivateKey(dataRepresentation: bytes)
    return VerifiedNostrEvent.hex(key.xonly.bytes)
  }

  static func memberDigests(_ pubkeys: [String]) -> [String] {
    pubkeys.map {
      BuzzPushPresentationIdentity.channelMember(
        communityID: "community-id",
        channelID: channelID,
        pubkey: $0
      )
    }.sorted()
  }

  static func signedEvent(
    privateKey: String,
    createdAt: Int,
    kind: Int,
    tags: [[String]] = [],
    content: String = ""
  ) throws -> VerifiedNostrEvent {
    let privateKeyBytes = try XCTUnwrap(VerifiedNostrEvent.hexBytes(privateKey))
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

  final class URLProtocolStub: URLProtocol, @unchecked Sendable {
    static let lock = NSLock()
    static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?
    static var finishLoading: ((URLRequest) -> Bool)?
    static var onStopLoading: ((URLRequest) -> Void)?
    static var requests: [URLRequest] = []

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
      Self.lock.lock()
      Self.requests.append(request)
      let handler = Self.handler
      Self.lock.unlock()
      do {
        let (response, data) = try handler?(request) ?? { throw URLError(.unsupportedURL) }()
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        if Self.finishLoading?(request) ?? true {
          client?.urlProtocolDidFinishLoading(self)
        }
      } catch {
        client?.urlProtocol(self, didFailWithError: error)
      }
    }

    override func stopLoading() { Self.onStopLoading?(request) }

    static func reset() {
      lock.lock()
      handler = nil
      finishLoading = nil
      onStopLoading = nil
      requests = []
      lock.unlock()
    }
  }
}
