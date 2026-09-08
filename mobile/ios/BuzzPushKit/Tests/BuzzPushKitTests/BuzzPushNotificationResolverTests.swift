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

  func testDecodeResolutionSenderTitlesCanonicalizeKeysOrFallBackToNeutral() {
    // The sender-identity presentation boundary: a verifiable key renders
    // as the same compact npub whether it arrives as hex or as an npub, and
    // an unverifiable key — including radix impostors like "+a"×32 that
    // parse as hex pairs but are not literal hex keys — gets the neutral
    // identity, never raw key material, while body, subtitle, and internal
    // payload fields pass through untouched. Malformed-input classification
    // itself is pinned at the codec (Bech32Tests); named senders winning
    // over these labels is covered by the cached-profile resolve tests.
    let senders: [(pubkey: String, title: String)] = [
      (Self.unnamedSenderHex, "npub14f8…9nsy"),
      (Self.unnamedSenderNpub, "npub14f8…9nsy"),
      ("author-pubkey", "Someone"),
      (String(repeating: "+a", count: 32), "Someone"),
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
    XCTAssertEqual(result.title, "npub1ccz…mnyd")
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

  func testStaleVerifiedCacheIsUsedWhileOneBoundedRefreshFails() throws {
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
      XCTAssertEqual(request.timeoutInterval, 3)
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
    XCTAssertEqual(URLProtocolStub.requests.count, 2)
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

  func testMissingCacheRefreshesVerifiedProfileAndChannelTogether() throws {
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
    XCTAssertEqual(URLProtocolStub.requests.count, 2)
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

    XCTAssertEqual(result.title, "npub1ccz…mnyd")
    XCTAssertNil(result.conversationDisplayName)
    XCTAssertEqual(result.subtitle, "Community")
    XCTAssertEqual(result.body, "Fallback content")
  }

  func testOversizedPresentationRefreshFallsBackWithoutBlockingMessage() throws {
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

    XCTAssertEqual(result.title, "npub1ccz…mnyd")
    XCTAssertNil(result.conversationDisplayName)
    XCTAssertEqual(result.body, "Bounded fallback")
    XCTAssertEqual(URLProtocolStub.requests.count, 2)
  }

  func testBoundedInlineAvatarProfileRefreshStillResolvesDisplayName() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Inline avatar profile"
    )
    let picture = "data:image/png;base64," + String(repeating: "A", count: 170_000)
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
    let presentationData = try JSONEncoder().encode([profile])
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
          communitiesData: try snapshotData([community()]),
          now: Date(timeIntervalSince1970: TimeInterval(Self.now))
        )
      )
    )

    XCTAssertEqual(result.title, "Fizz")
    XCTAssertNil(result.senderAvatarPNG)
    XCTAssertEqual(result.body, "Inline avatar profile")
    XCTAssertEqual(URLProtocolStub.requests.count, 2)
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
    now: Date = Date()
  ) -> BuzzPushNotificationResolver {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [URLProtocolStub.self]
    return BuzzPushNotificationResolver(
      session: URLSession(configuration: configuration),
      loadCommunitiesData: { communitiesData },
      loadPrivateKey: { privateKeys[$0] },
      loadPresentationCacheData: { presentationCacheData },
      now: { now }
    )
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
        ]
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
        client?.urlProtocolDidFinishLoading(self)
      } catch {
        client?.urlProtocol(self, didFailWithError: error)
      }
    }

    override func stopLoading() {}

    static func reset() {
      lock.lock()
      handler = nil
      requests = []
      lock.unlock()
    }
  }
}
