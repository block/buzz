import Foundation
import XCTest

@testable import BuzzPushKit

extension BuzzPushNotificationResolverTests {
  func testOpenChannelOutsiderUsesEveryVerifiedMemberAsARecipient() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Hello from an open-channel guest"
    )
    let relayPubkey = try Self.pubkey(for: Self.relayPrivateKey)
    let snapshot = BuzzPushPresentationCacheSnapshot(
      channels: [
        BuzzPushCachedChannel(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          channelID: Self.channelID,
          relayMetadataPubkey: relayPubkey,
          displayName: "General",
          channelType: "stream",
          memberCount: 2,
          memberDigests: Self.memberDigests([Self.ownPubkey, relayPubkey]),
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

    XCTAssertEqual(result.conversationDisplayName, "#General")
    XCTAssertEqual(result.conversationRecipientCount, 2)
  }

  func testMissingCurrentUserInVerifiedRosterUsesOrdinaryPresentation() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Do not fabricate a group"
    )
    let relayPubkey = try Self.pubkey(for: Self.relayPrivateKey)
    let snapshot = BuzzPushPresentationCacheSnapshot(
      channels: [
        BuzzPushCachedChannel(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          channelID: Self.channelID,
          relayMetadataPubkey: relayPubkey,
          displayName: "General",
          channelType: "stream",
          memberCount: 1,
          memberDigests: Self.memberDigests([message.pubkey]),
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

    XCTAssertNil(result.conversationDisplayName)
    XCTAssertNil(result.conversationRecipientCount)
    XCTAssertEqual(result.subtitle, "Community")
  }

  func testFreshEvictedMembershipDigestsTriggerOneBoundedRefresh() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Refresh an evicted roster"
    )
    let relayPubkey = try Self.pubkey(for: Self.relayPrivateKey)
    let snapshot = BuzzPushPresentationCacheSnapshot(
      channels: [
        BuzzPushCachedChannel(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          channelID: Self.channelID,
          relayMetadataPubkey: relayPubkey,
          displayName: "General",
          channelType: "stream",
          memberCount: 2,
          memberDigests: nil,
          membershipEventID: "evicted-membership",
          membershipEventCreatedAt: Self.now - 1,
          membershipCachedAt: Self.now,
          eventID: "channel-event",
          eventCreatedAt: Self.now,
          cachedAt: Self.now
        )
      ]
    )
    let refreshedMembership = try Self.signedEvent(
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
      XCTAssertEqual(request.timeoutInterval, 3)
      return Self.response(
        request,
        status: 200,
        data: try JSONEncoder().encode([refreshedMembership])
      )
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

    XCTAssertEqual(result.conversationDisplayName, "#General")
    XCTAssertEqual(result.conversationRecipientCount, 1)
    XCTAssertEqual(URLProtocolStub.requests.count, 2)
  }

  func testTwoPersonDMUsesDirectCommunicationPresentation() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Direct message"
    )
    let relayPubkey = try Self.pubkey(for: Self.relayPrivateKey)
    let snapshot = BuzzPushPresentationCacheSnapshot(
      channels: [
        BuzzPushCachedChannel(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          channelID: Self.channelID,
          relayMetadataPubkey: relayPubkey,
          displayName: "DM",
          channelType: "dm",
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

    XCTAssertNil(result.conversationDisplayName)
    XCTAssertEqual(result.conversationRecipientCount, 1)
  }

  func testGroupDMWithoutVerifiedDisplayLabelUsesOrdinaryPresentation() throws {
    let message = try Self.signedEvent(
      privateKey: Self.profilePrivateKey,
      createdAt: Self.now,
      kind: 9,
      tags: [["h", Self.channelID]],
      content: "Group direct message"
    )
    let relayPubkey = try Self.pubkey(for: Self.relayPrivateKey)
    let snapshot = BuzzPushPresentationCacheSnapshot(
      channels: [
        BuzzPushCachedChannel(
          communityID: "community-id",
          relayOrigin: "https://relay.example",
          channelID: Self.channelID,
          relayMetadataPubkey: relayPubkey,
          displayName: "DM",
          channelType: "dm",
          memberCount: 3,
          memberDigests: Self.memberDigests([
            Self.ownPubkey,
            message.pubkey,
            relayPubkey,
          ]),
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

    XCTAssertNil(result.conversationDisplayName)
    XCTAssertNil(result.conversationRecipientCount)
    XCTAssertEqual(result.subtitle, "Community")
  }
}
