import CryptoKit
import Foundation
import P256K
import Testing

@testable import BuzzPushKit

@Suite("Push presentation cache")
struct BuzzPushPresentationCacheTests {
  private let profileKey = String(repeating: "0", count: 63) + "1"
  private let relayKey = String(repeating: "0", count: 63) + "2"
  private let otherRelayKey = String(repeating: "0", count: 63) + "3"

  @Test("Verified profile uses display_name, then name, and attaches a bounded local avatar")
  func verifiedProfilePrecedenceAndAvatar() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(
      containerURL: directory,
      now: { Date(timeIntervalSince1970: 1_700_000_100) }
    )
    let event = try signedEvent(
      privateKey: profileKey,
      createdAt: 1_700_000_000,
      kind: 0,
      content:
        #"{"display_name":"  Alice   Example ","name":"alice","picture":"https://images.example/alice.png"}"#
    )

    let needsAvatar = try store.updateProfiles(
      communityID: "community-a",
      relayOrigin: "wss://relay.example/",
      updates: [BuzzPushProfileCacheUpdate(event: event)]
    )
    try store.updateProfiles(
      communityID: "community-b",
      relayOrigin: "wss://relay.example/",
      updates: [BuzzPushProfileCacheUpdate(event: event)]
    )

    #expect(needsAvatar == Set([event.id]))
    var snapshot = try loadSnapshot(directory)
    var cached = try #require(
      snapshot.profile(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        pubkey: event.pubkey
      )
    )
    #expect(cached.displayName == "Alice Example")
    #expect(cached.avatarPNG == nil)
    #expect(
      snapshot.profile(
        communityID: "community-a",
        relayOrigin: "https://other.example",
        pubkey: event.pubkey
      ) == nil
    )

    let png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01])
    #expect(
      try store.updateAvatar(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        sourceURL: "https://images.example/alice.png",
        avatarPNG: png
      )
    )
    snapshot = try loadSnapshot(directory)
    cached = try #require(
      snapshot.profile(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        pubkey: event.pubkey
      )
    )
    #expect(cached.avatarPNG == png)
    #expect(
      snapshot.profile(
        communityID: "community-b",
        relayOrigin: "https://relay.example",
        pubkey: event.pubkey
      )?.avatarPNG == nil
    )
  }

  @Test("Verified inline raster profile retains its name and accepts a local thumbnail")
  func verifiedInlineRasterProfileAndAvatar() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let picture = "data:image/png;base64," + String(repeating: "A", count: 170_000)
    let content = try #require(
      String(
        data: JSONSerialization.data(withJSONObject: [
          "display_name": "Fizz",
          "picture": picture,
        ]),
        encoding: .utf8
      )
    )
    let event = try signedEvent(privateKey: profileKey, kind: 0, content: content)

    let needsAvatar = try store.updateProfiles(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      updates: [BuzzPushProfileCacheUpdate(event: event)]
    )
    #expect(needsAvatar == Set([event.id]))
    #expect(
      try loadSnapshot(directory).profile(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        pubkey: event.pubkey
      )?.displayName == "Fizz"
    )

    let png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    #expect(
      try store.updateAvatar(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        sourceURL: picture,
        avatarPNG: png
      )
    )
    #expect(
      try loadSnapshot(directory).profile(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        pubkey: event.pubkey
      )?.avatarPNG == png
    )
  }

  @Test("Verified profile falls back from blank display_name to name")
  func verifiedProfileNameFallback() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let event = try signedEvent(
      privateKey: profileKey,
      kind: 0,
      content: #"{"display_name":"  ","name":"Alice"}"#
    )

    try store.updateProfiles(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      updates: [BuzzPushProfileCacheUpdate(event: event)]
    )

    let cached = try #require(
      try loadSnapshot(directory).profile(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        pubkey: event.pubkey
      )
    )
    #expect(cached.displayName == "Alice")
  }

  @Test("Malformed verified profile clears presentation while an unverified event is ignored")
  func malformedAndUnverifiedProfileFallback() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let named = try signedEvent(
      privateKey: profileKey,
      createdAt: 100,
      kind: 0,
      content: #"{"name":"Alice"}"#
    )
    try store.updateProfiles(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      updates: [BuzzPushProfileCacheUpdate(event: named)]
    )
    let malformed = try signedEvent(
      privateKey: profileKey,
      createdAt: 101,
      kind: 0,
      content: "not-json"
    )
    try store.updateProfiles(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      updates: [BuzzPushProfileCacheUpdate(event: malformed)]
    )
    let tampered = VerifiedNostrEvent(
      id: malformed.id,
      pubkey: malformed.pubkey,
      createdAt: 102,
      kind: 0,
      tags: [],
      content: #"{"display_name":"Mallory"}"#,
      sig: malformed.sig
    )
    try store.updateProfiles(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      updates: [BuzzPushProfileCacheUpdate(event: tampered)]
    )

    let cached = try #require(
      try loadSnapshot(directory).profile(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        pubkey: malformed.pubkey
      )
    )
    #expect(cached.eventID == malformed.id)
    #expect(cached.displayName == nil)
  }

  @Test("Channel name requires the expected relay signer and accepts opaque IDs")
  func channelAuthorityAndOpaqueID() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let relayPubkey = try pubkey(for: relayKey)
    let opaqueChannelID = "channel/general:v5"
    let verified = try signedEvent(
      privateKey: relayKey,
      createdAt: 100,
      kind: 39_000,
      tags: [["d", opaqueChannelID], ["name", "  General  Chat "], ["t", "stream"]]
    )
    let wrongSigner = try signedEvent(
      privateKey: otherRelayKey,
      createdAt: 101,
      kind: 39_000,
      tags: [["d", opaqueChannelID], ["name", "Impostor"]]
    )

    try store.updateChannels(
      communityID: "community-a",
      relayOrigin: "wss://relay.example",
      relayMetadataPubkey: relayPubkey,
      metadataEvents: [wrongSigner, verified],
      membershipEvents: []
    )

    let cached = try #require(
      try loadSnapshot(directory).channel(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        channelID: opaqueChannelID
      )
    )
    #expect(cached.eventID == verified.id)
    #expect(cached.displayName == "General Chat")
    #expect(cached.channelType == "stream")
    #expect(cached.relayMetadataPubkey == relayPubkey)
  }

  @Test("Channel membership requires the same relay authority and keeps exact scoped digests")
  func channelMembershipAuthorityAndOrdering() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let relayPubkey = try pubkey(for: relayKey)
    let firstMember = try pubkey(for: profileKey)
    let secondMember = try pubkey(for: otherRelayKey)
    let metadata = try signedEvent(
      privateKey: relayKey,
      createdAt: 100,
      kind: 39_000,
      tags: [["d", "opaque-channel"], ["name", "General"], ["t", "stream"]]
    )
    let newestMembership = try signedEvent(
      privateKey: relayKey,
      createdAt: 102,
      kind: 39_002,
      tags: [
        ["d", "opaque-channel"],
        ["p", firstMember],
        ["p", secondMember],
        ["p", secondMember],
      ]
    )
    let olderMembership = try signedEvent(
      privateKey: relayKey,
      createdAt: 101,
      kind: 39_002,
      tags: [["d", "opaque-channel"], ["p", firstMember]]
    )
    let wrongSigner = try signedEvent(
      privateKey: otherRelayKey,
      createdAt: 103,
      kind: 39_002,
      tags: [["d", "opaque-channel"], ["p", firstMember]]
    )
    let malformed = try signedEvent(
      privateKey: relayKey,
      createdAt: 104,
      kind: 39_002,
      tags: [["d", "opaque-channel"], ["p", "not-a-pubkey"]]
    )

    try store.updateChannels(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      relayMetadataPubkey: relayPubkey,
      metadataEvents: [metadata],
      membershipEvents: [wrongSigner, malformed, newestMembership, olderMembership]
    )

    let cached = try #require(
      try loadSnapshot(directory).channel(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        channelID: "opaque-channel"
      )
    )
    #expect(cached.memberCount == 2)
    #expect(cached.membershipEventID == newestMembership.id)
    #expect(
      cached.memberDigests
        == [firstMember, secondMember].map {
          BuzzPushPresentationIdentity.channelMember(
            communityID: "community-a",
            channelID: "opaque-channel",
            pubkey: $0
          )
        }.sorted()
    )
  }

  @Test("Channel authority rotation clears membership signed by the old authority")
  func channelAuthorityRotationClearsMembership() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let relayPubkey = try pubkey(for: relayKey)
    let rotatedRelayPubkey = try pubkey(for: otherRelayKey)
    let member = try pubkey(for: profileKey)
    let metadata = try signedEvent(
      privateKey: relayKey,
      createdAt: 100,
      kind: 39_000,
      tags: [["d", "opaque-channel"], ["name", "General"], ["t", "stream"]]
    )
    let membership = try signedEvent(
      privateKey: relayKey,
      createdAt: 101,
      kind: 39_002,
      tags: [["d", "opaque-channel"], ["p", member]]
    )
    try store.updateChannels(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      relayMetadataPubkey: relayPubkey,
      metadataEvents: [metadata],
      membershipEvents: [membership]
    )

    let rotatedMetadata = try signedEvent(
      privateKey: otherRelayKey,
      createdAt: 50,
      kind: 39_000,
      tags: [["d", "opaque-channel"], ["name", "General"], ["t", "stream"]]
    )
    try store.updateChannels(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      relayMetadataPubkey: rotatedRelayPubkey,
      metadataEvents: [rotatedMetadata],
      membershipEvents: []
    )

    let cached = try #require(
      try loadSnapshot(directory).channel(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        channelID: "opaque-channel"
      )
    )
    #expect(cached.relayMetadataPubkey == rotatedRelayPubkey)
    #expect(cached.eventID == rotatedMetadata.id)
    #expect(cached.memberCount == nil)
    #expect(cached.memberDigests == nil)
    #expect(cached.membershipEventID == nil)
  }

  @Test("Oversized channel batches are ignored before cache mutation")
  func oversizedChannelBatchIsIgnored() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let relayPubkey = try pubkey(for: relayKey)
    let initial = try signedEvent(
      privateKey: relayKey,
      createdAt: 100,
      kind: 39_000,
      tags: [["d", "opaque-channel"], ["name", "Initial"], ["t", "stream"]]
    )
    try store.updateChannels(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      relayMetadataPubkey: relayPubkey,
      metadataEvents: [initial],
      membershipEvents: []
    )
    let replacement = try signedEvent(
      privateKey: relayKey,
      createdAt: 101,
      kind: 39_000,
      tags: [["d", "opaque-channel"], ["name", "Replacement"], ["t", "stream"]]
    )

    try store.updateChannels(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      relayMetadataPubkey: relayPubkey,
      metadataEvents: Array(
        repeating: replacement,
        count: BuzzPushPresentationCacheStore.maximumChannels + 1
      ),
      membershipEvents: []
    )

    let cached = try #require(
      try loadSnapshot(directory).channel(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        channelID: "opaque-channel"
      )
    )
    #expect(cached.eventID == initial.id)
    #expect(cached.displayName == "Initial")
  }

  @Test("Global member-digest bound drops oldest complete rosters first")
  func globalMembershipDigestBound() throws {
    let membersPerChannel = BuzzPushPresentationCacheStore.maximumMembersPerChannel
    let channelCount =
      BuzzPushPresentationCacheStore.maximumTotalMemberDigests / membersPerChannel + 1
    let digests = (0..<membersPerChannel).map { String(format: "%064x", $0 + 1) }
    let snapshot = BuzzPushPresentationCacheSnapshot(
      channels: (0..<channelCount).map { index in
        BuzzPushCachedChannel(
          communityID: "community-a",
          relayOrigin: "https://relay.example",
          channelID: "channel-\(index)",
          relayMetadataPubkey: String(repeating: "a", count: 64),
          displayName: "Channel \(index)",
          channelType: "stream",
          memberCount: membersPerChannel,
          memberDigests: digests,
          membershipEventID: "membership-\(index)",
          membershipEventCreatedAt: index,
          membershipCachedAt: 1_000 + index,
          eventID: "metadata-\(index)",
          eventCreatedAt: index,
          cachedAt: 1_000 + index
        )
      }
    )

    let encoded = try BuzzPushPresentationCacheStore.encodedBoundedSnapshot(snapshot)
    let decoded = try JSONDecoder().decode(BuzzPushPresentationCacheSnapshot.self, from: encoded)
    let totalDigests = decoded.channels.reduce(0) { $0 + ($1.memberDigests?.count ?? 0) }

    #expect(totalDigests == BuzzPushPresentationCacheStore.maximumTotalMemberDigests)
    #expect(decoded.channels.first { $0.channelID == "channel-0" }?.memberDigests == nil)
    #expect(
      decoded.channels.first { $0.channelID == "channel-\(channelCount - 1)" }?.memberDigests != nil
    )
  }

  @Test("Oversized membership keeps provenance but omits an incomplete digest set")
  func oversizedMembershipFallback() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let relayPubkey = try pubkey(for: relayKey)
    let metadata = try signedEvent(
      privateKey: relayKey,
      kind: 39_000,
      tags: [["d", "large-channel"], ["name", "Large"], ["t", "stream"]]
    )
    let memberCount = BuzzPushPresentationCacheStore.maximumMembersPerChannel + 25
    let membership = try signedEvent(
      privateKey: relayKey,
      kind: 39_002,
      tags: [["d", "large-channel"]]
        + (0..<memberCount).map {
          ["p", String(format: "%064x", $0 + 1)]
        }
    )

    try store.updateChannels(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      relayMetadataPubkey: relayPubkey,
      metadataEvents: [metadata],
      membershipEvents: [membership]
    )

    let cached = try #require(
      try loadSnapshot(directory).channel(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        channelID: "large-channel"
      )
    )
    #expect(
      cached.memberCount == BuzzPushPresentationCacheStore.maximumMembersPerChannel + 1
    )
    #expect(cached.memberDigests == nil)
    #expect(cached.membershipEventID == membership.id)
  }

  @Test("Missing or malformed channel metadata never fabricates a name")
  func malformedChannelMetadataFallback() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let relayPubkey = try pubkey(for: relayKey)
    let blankName = try signedEvent(
      privateKey: relayKey,
      createdAt: 100,
      kind: 39_000,
      tags: [["d", "opaque-channel"], ["name", "  \n "]]
    )
    let missingChannelID = try signedEvent(
      privateKey: relayKey,
      createdAt: 101,
      kind: 39_000,
      tags: [["name", "Must not be used"]]
    )

    try store.updateChannels(
      communityID: "community-a",
      relayOrigin: "https://relay.example",
      relayMetadataPubkey: relayPubkey,
      metadataEvents: [blankName, missingChannelID],
      membershipEvents: []
    )

    let snapshot = try loadSnapshot(directory)
    let cached = try #require(
      snapshot.channel(
        communityID: "community-a",
        relayOrigin: "https://relay.example",
        channelID: "opaque-channel"
      )
    )
    #expect(cached.displayName == nil)
    #expect(snapshot.channels.count == 1)
  }

  @Test("Community removal prunes profile and channel state")
  func communityPruning() throws {
    let directory = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = BuzzPushPresentationCacheStore(containerURL: directory)
    let profile = try signedEvent(privateKey: profileKey, kind: 0, content: #"{"name":"A"}"#)
    let channel = try signedEvent(
      privateKey: relayKey,
      kind: 39_000,
      tags: [["d", "opaque"], ["name", "General"]]
    )
    try store.updateProfiles(
      communityID: "removed",
      relayOrigin: "https://relay.example",
      updates: [BuzzPushProfileCacheUpdate(event: profile)]
    )
    try store.updateChannels(
      communityID: "removed",
      relayOrigin: "https://relay.example",
      relayMetadataPubkey: try pubkey(for: relayKey),
      metadataEvents: [channel],
      membershipEvents: []
    )

    try store.replaceCommunities([
      PushLeaseCommunity(
        id: "retained",
        name: "Retained",
        relayUrl: "https://relay.example",
        pubkey: nil,
        policies: []
      )
    ])

    let snapshot = try loadSnapshot(directory)
    #expect(snapshot.communities.map(\.id) == ["retained"])
    #expect(snapshot.profiles.isEmpty)
    #expect(snapshot.channels.isEmpty)
  }

  @Test("Cache deterministically evicts entries beyond its global bounds")
  func boundedEviction() {
    var snapshot = BuzzPushPresentationCacheSnapshot(
      profiles: (0...BuzzPushPresentationCacheStore.maximumProfiles).map { index in
        BuzzPushCachedProfile(
          communityID: "community-a",
          relayOrigin: "https://relay.example",
          pubkey: String(format: "%064x", index),
          displayName: "Profile \(index)",
          pictureHash: nil,
          avatarPNG: nil,
          eventID: String(format: "%064x", index),
          eventCreatedAt: index,
          cachedAt: index
        )
      },
      channels: (0...BuzzPushPresentationCacheStore.maximumChannels).map { index in
        BuzzPushCachedChannel(
          communityID: "community-a",
          relayOrigin: "https://relay.example",
          channelID: "channel-\(index)",
          relayMetadataPubkey: String(repeating: "a", count: 64),
          displayName: "Channel \(index)",
          eventID: String(format: "%064x", index),
          eventCreatedAt: index,
          cachedAt: index
        )
      }
    )

    BuzzPushPresentationCacheStore.enforceBounds(&snapshot)

    #expect(snapshot.profiles.count == BuzzPushPresentationCacheStore.maximumProfiles)
    #expect(snapshot.channels.count == BuzzPushPresentationCacheStore.maximumChannels)
    #expect(snapshot.profiles.contains { $0.cachedAt == 0 } == false)
    #expect(snapshot.channels.contains { $0.cachedAt == 0 } == false)
  }

  @Test("Encoded cache remains bounded with adversarial strings and maximum avatars")
  func encodedCacheByteBound() throws {
    let controlText = String(repeating: "\u{0001}", count: 1_024)
    let avatar =
      Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
      + Data(
        repeating: 0,
        count: BuzzPushPresentationCacheStore.maximumAvatarBytes - 8
      )
    let snapshot = BuzzPushPresentationCacheSnapshot(
      profiles: (0..<80).map { index in
        BuzzPushCachedProfile(
          communityID: controlText,
          relayOrigin: "https://relay.example",
          pubkey: String(format: "%064x", index),
          displayName: controlText,
          pictureHash: String(repeating: "a", count: 64),
          avatarPNG: avatar,
          eventID: String(format: "%064x", index),
          eventCreatedAt: index,
          cachedAt: index
        )
      },
      channels: (0..<BuzzPushPresentationCacheStore.maximumChannels).map { index in
        BuzzPushCachedChannel(
          communityID: controlText,
          relayOrigin: "https://relay.example",
          channelID: "\(controlText)\(index)",
          relayMetadataPubkey: String(repeating: "b", count: 64),
          displayName: controlText,
          eventID: String(format: "%064x", index),
          eventCreatedAt: index,
          cachedAt: 1_000 + index
        )
      }
    )

    let encoded = try BuzzPushPresentationCacheStore.encodedBoundedSnapshot(snapshot)
    let decoded = try JSONDecoder().decode(
      BuzzPushPresentationCacheSnapshot.self,
      from: encoded
    )

    #expect(encoded.count <= BuzzPushPresentationCacheStore.maximumSnapshotBytes)
    #expect(decoded.channels.first?.cachedAt == 1_511)
    #expect(decoded.profiles.count + decoded.channels.count < 592)
  }

  @Test("Display-name normalization has character and UTF-8 bounds")
  func displayNameUTF8Bound() throws {
    let oneOversizedGrapheme = "a" + String(repeating: "\u{0301}", count: 2_048)
    let event = try signedEvent(
      privateKey: profileKey,
      kind: 0,
      content: try String(
        data: JSONSerialization.data(withJSONObject: ["display_name": oneOversizedGrapheme]),
        encoding: .utf8
      ) ?? ""
    )

    let metadata = BuzzPushPresentationCacheStore.profileMetadata(event)

    #expect(metadata.displayName == nil || metadata.displayName!.utf8.count <= 512)
  }

  private func temporaryDirectory() throws -> URL {
    let url = FileManager.default.temporaryDirectory
      .appendingPathComponent("buzz-push-cache-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
  }

  private func loadSnapshot(_ directory: URL) throws -> BuzzPushPresentationCacheSnapshot {
    let data = try Data(
      contentsOf: directory.appendingPathComponent(BuzzPushPresentationCacheStore.fileName)
    )
    return try JSONDecoder().decode(BuzzPushPresentationCacheSnapshot.self, from: data)
  }

  private func pubkey(for privateKey: String) throws -> String {
    let bytes = try #require(VerifiedNostrEvent.hexBytes(privateKey))
    let key = try P256K.Schnorr.PrivateKey(dataRepresentation: bytes)
    return VerifiedNostrEvent.hex(key.xonly.bytes)
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
