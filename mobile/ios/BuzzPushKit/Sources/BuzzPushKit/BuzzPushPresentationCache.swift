import CryptoKit
import Foundation

/// A verified sender profile retained for notification presentation.
public struct BuzzPushCachedProfile: Codable, Equatable, Sendable {
  public let communityID: String
  public let relayOrigin: String
  public let pubkey: String
  public let displayName: String?
  public let pictureHash: String?
  public let avatarPNG: Data?
  public let eventID: String
  public let eventCreatedAt: Int
  public let cachedAt: Int

  public init(
    communityID: String,
    relayOrigin: String,
    pubkey: String,
    displayName: String?,
    pictureHash: String?,
    avatarPNG: Data?,
    eventID: String,
    eventCreatedAt: Int,
    cachedAt: Int
  ) {
    self.communityID = communityID
    self.relayOrigin = relayOrigin
    self.pubkey = pubkey
    self.displayName = displayName
    self.pictureHash = pictureHash
    self.avatarPNG = avatarPNG
    self.eventID = eventID
    self.eventCreatedAt = eventCreatedAt
    self.cachedAt = cachedAt
  }
}

/// Verified channel metadata retained for notification presentation.
public struct BuzzPushCachedChannel: Codable, Equatable, Sendable {
  public let communityID: String
  public let relayOrigin: String
  public let channelID: String
  public let relayMetadataPubkey: String
  public let displayName: String?
  /// Relay-verified Buzz channel type, when recognized.
  public let channelType: String?
  /// Exact unique-member count when bounded, or a value above the bound when oversized.
  public let memberCount: Int?
  /// Complete community-and-channel-scoped member digests, when within bounds.
  public let memberDigests: [String]?
  /// Event ID that established the cached membership snapshot.
  public let membershipEventID: String?
  /// Creation time of the cached membership replacement event.
  public let membershipEventCreatedAt: Int?
  /// Device time when the membership snapshot was cached.
  public let membershipCachedAt: Int?
  public let eventID: String
  public let eventCreatedAt: Int
  public let cachedAt: Int

  public init(
    communityID: String,
    relayOrigin: String,
    channelID: String,
    relayMetadataPubkey: String,
    displayName: String?,
    channelType: String? = nil,
    memberCount: Int? = nil,
    memberDigests: [String]? = nil,
    membershipEventID: String? = nil,
    membershipEventCreatedAt: Int? = nil,
    membershipCachedAt: Int? = nil,
    eventID: String,
    eventCreatedAt: Int,
    cachedAt: Int
  ) {
    self.communityID = communityID
    self.relayOrigin = relayOrigin
    self.channelID = channelID
    self.relayMetadataPubkey = relayMetadataPubkey
    self.displayName = displayName
    self.channelType = channelType
    self.memberCount = memberCount
    self.memberDigests = memberDigests
    self.membershipEventID = membershipEventID
    self.membershipEventCreatedAt = membershipEventCreatedAt
    self.membershipCachedAt = membershipCachedAt
    self.eventID = eventID
    self.eventCreatedAt = eventCreatedAt
    self.cachedAt = cachedAt
  }
}

/// Atomic App Group snapshot shared by the app and its notification extension.
public struct BuzzPushPresentationCacheSnapshot: Codable, Equatable, Sendable {
  public static let currentVersion = 1

  public let version: Int
  public var communities: [PushLeaseCommunity]
  public var profiles: [BuzzPushCachedProfile]
  public var channels: [BuzzPushCachedChannel]

  public init(
    version: Int = currentVersion,
    communities: [PushLeaseCommunity] = [],
    profiles: [BuzzPushCachedProfile] = [],
    channels: [BuzzPushCachedChannel] = []
  ) {
    self.version = version
    self.communities = communities
    self.profiles = profiles
    self.channels = channels
  }

  public static func decode(_ data: Data?) -> Self {
    guard let data,
      let snapshot = try? JSONDecoder().decode(Self.self, from: data),
      snapshot.version == currentVersion
    else { return Self() }
    return snapshot
  }

  public func profile(
    communityID: String,
    relayOrigin: String,
    pubkey: String
  ) -> BuzzPushCachedProfile? {
    let normalizedPubkey = pubkey.lowercased()
    return profiles.first {
      $0.communityID == communityID && $0.relayOrigin == relayOrigin
        && $0.pubkey == normalizedPubkey
    }
  }

  public func channel(
    communityID: String,
    relayOrigin: String,
    channelID: String
  ) -> BuzzPushCachedChannel? {
    channels.first {
      $0.communityID == communityID && $0.relayOrigin == relayOrigin
        && $0.channelID == channelID
    }
  }
}

/// One app-provided profile update, optionally carrying a sanitized local thumbnail.
public struct BuzzPushProfileCacheUpdate: Sendable {
  public let event: VerifiedNostrEvent
  public let avatarPNG: Data?

  public init(event: VerifiedNostrEvent, avatarPNG: Data? = nil) {
    self.event = event
    self.avatarPNG = avatarPNG
  }
}

/// Maintains the bounded presentation snapshot. The app is the sole writer.
public final class BuzzPushPresentationCacheStore: @unchecked Sendable {
  public static let fileName = "push-snapshot.json"
  public static let freshnessLifetime: TimeInterval = 24 * 60 * 60
  public static let maximumProfiles = 256
  public static let maximumChannels = 512
  public static let maximumCommunities = 64
  public static let maximumMembersPerChannel = 512
  public static let maximumTotalMemberDigests = 8_192
  public static let maximumAvatarBytes = 64 * 1024
  public static let maximumTotalAvatarBytes = 4 * 1024 * 1024
  public static let maximumSnapshotBytes = 8 * 1024 * 1024
  static let maximumProfileMetadataBytes = 256 * 1024

  private let fileURL: URL
  private let now: () -> Date
  private let lock = NSLock()

  public init(containerURL: URL, now: @escaping () -> Date = Date.init) {
    fileURL = containerURL.appendingPathComponent(Self.fileName)
    self.now = now
  }

  /// Replaces the app's flattened, relay-accepted community query policy.
  public func replaceCommunities(_ communities: [PushLeaseCommunity]) throws {
    guard communities.count <= Self.maximumCommunities else { return }
    lock.lock()
    defer { lock.unlock() }
    var snapshot = loadLocked()
    snapshot.communities = communities
    let retained = Set(communities.map(\.id))
    snapshot.profiles.removeAll { !retained.contains($0.communityID) }
    snapshot.channels.removeAll { !retained.contains($0.communityID) }
    try writeLocked(snapshot)
  }

  /// Saves verified kind-0 events and returns the event IDs still needing thumbnails.
  @discardableResult
  public func updateProfiles(
    communityID: String,
    relayOrigin: String,
    updates: [BuzzPushProfileCacheUpdate]
  ) throws -> Set<String> {
    guard Self.isBoundedOpaqueID(communityID),
      let canonicalRelayOrigin = Self.canonicalRelayOrigin(relayOrigin),
      updates.count <= Self.maximumProfiles
    else { return [] }
    lock.lock()
    defer { lock.unlock() }

    var snapshot = loadLocked()
    let cachedAt = Int(now().timeIntervalSince1970)
    var acceptedEventIDs = Set<String>()
    for update in updates {
      let event = update.event
      guard event.kind == 0, event.hasValidIDAndSignature() else { continue }
      let pubkey = event.pubkey.lowercased()
      guard Self.isHexPubkey(pubkey) else { continue }

      let metadata = Self.profileMetadata(event)
      let index = snapshot.profiles.firstIndex {
        $0.communityID == communityID && $0.relayOrigin == canonicalRelayOrigin
          && $0.pubkey == pubkey
      }
      let existing = index.map { snapshot.profiles[$0] }
      guard
        Self.shouldReplace(
          existingCreatedAt: existing?.eventCreatedAt,
          existingID: existing?.eventID,
          candidateCreatedAt: event.createdAt,
          candidateID: event.id
        )
      else { continue }

      let suppliedAvatar = Self.normalizedAvatarPNG(update.avatarPNG)
      let preservedAvatar =
        existing?.pictureHash == metadata.pictureHash
        ? existing?.avatarPNG : nil
      let entry = BuzzPushCachedProfile(
        communityID: communityID,
        relayOrigin: canonicalRelayOrigin,
        pubkey: pubkey,
        displayName: metadata.displayName,
        pictureHash: metadata.pictureHash,
        avatarPNG: metadata.pictureHash == nil ? nil : suppliedAvatar ?? preservedAvatar,
        eventID: event.id,
        eventCreatedAt: event.createdAt,
        cachedAt: cachedAt
      )
      if let index {
        snapshot.profiles[index] = entry
      } else {
        snapshot.profiles.append(entry)
      }
      acceptedEventIDs.insert(event.id)
    }

    Self.enforceBounds(&snapshot)
    try writeLocked(snapshot)
    return Set(
      snapshot.profiles.compactMap { profile in
        guard profile.communityID == communityID,
          profile.relayOrigin == canonicalRelayOrigin,
          acceptedEventIDs.contains(profile.eventID),
          profile.pictureHash != nil,
          profile.avatarPNG == nil
        else { return nil }
        return profile.eventID
      })
  }

  /// Saves bounded relay-authorized kind-39000 metadata and kind-39002 membership snapshots.
  public func updateChannels(
    communityID: String,
    relayOrigin: String,
    relayMetadataPubkey: String,
    metadataEvents: [VerifiedNostrEvent],
    membershipEvents: [VerifiedNostrEvent]
  ) throws {
    let normalizedRelayPubkey = relayMetadataPubkey.lowercased()
    guard Self.isBoundedOpaqueID(communityID),
      let canonicalRelayOrigin = Self.canonicalRelayOrigin(relayOrigin),
      Self.isHexPubkey(normalizedRelayPubkey),
      metadataEvents.count <= Self.maximumChannels,
      membershipEvents.count <= Self.maximumChannels
    else {
      return
    }
    lock.lock()
    defer { lock.unlock() }

    var snapshot = loadLocked()
    let cachedAt = Int(now().timeIntervalSince1970)
    for event in metadataEvents {
      guard event.kind == 39_000, event.hasValidIDAndSignature(),
        event.pubkey.lowercased() == normalizedRelayPubkey,
        let channelID = Self.tagValue("d", in: event),
        Self.isBoundedOpaqueID(channelID)
      else { continue }
      let index = snapshot.channels.firstIndex {
        $0.communityID == communityID && $0.relayOrigin == canonicalRelayOrigin
          && $0.channelID == channelID
      }
      let existing = index.map { snapshot.channels[$0] }
      let hasCurrentAuthority = existing?.relayMetadataPubkey == normalizedRelayPubkey
      guard
        !hasCurrentAuthority
          || Self.shouldReplace(
            existingCreatedAt: existing?.eventCreatedAt,
            existingID: existing?.eventID,
            candidateCreatedAt: event.createdAt,
            candidateID: event.id
          )
      else { continue }

      let entry = BuzzPushCachedChannel(
        communityID: communityID,
        relayOrigin: canonicalRelayOrigin,
        channelID: channelID,
        relayMetadataPubkey: normalizedRelayPubkey,
        displayName: Self.normalizedDisplayName(Self.tagValue("name", in: event)),
        channelType: Self.normalizedChannelType(Self.tagValue("t", in: event)),
        memberCount: hasCurrentAuthority ? existing?.memberCount : nil,
        memberDigests: hasCurrentAuthority ? existing?.memberDigests : nil,
        membershipEventID: hasCurrentAuthority ? existing?.membershipEventID : nil,
        membershipEventCreatedAt: hasCurrentAuthority
          ? existing?.membershipEventCreatedAt : nil,
        membershipCachedAt: hasCurrentAuthority ? existing?.membershipCachedAt : nil,
        eventID: event.id,
        eventCreatedAt: event.createdAt,
        cachedAt: cachedAt
      )
      if let index {
        snapshot.channels[index] = entry
      } else {
        snapshot.channels.append(entry)
      }
    }
    Self.enforceChannelCountBound(&snapshot)

    for event in membershipEvents {
      guard event.kind == 39_002, event.hasValidIDAndSignature(),
        event.pubkey.lowercased() == normalizedRelayPubkey,
        let channelID = Self.tagValue("d", in: event),
        Self.isBoundedOpaqueID(channelID),
        let membership = Self.normalizedChannelMembership(
          event,
          communityID: communityID,
          channelID: channelID
        ),
        let index = snapshot.channels.firstIndex(where: {
          $0.communityID == communityID && $0.relayOrigin == canonicalRelayOrigin
            && $0.channelID == channelID
            && $0.relayMetadataPubkey == normalizedRelayPubkey
        })
      else { continue }
      let existing = snapshot.channels[index]
      guard
        Self.shouldReplace(
          existingCreatedAt: existing.membershipEventCreatedAt,
          existingID: existing.membershipEventID,
          candidateCreatedAt: event.createdAt,
          candidateID: event.id
        )
      else { continue }

      snapshot.channels[index] = BuzzPushCachedChannel(
        communityID: existing.communityID,
        relayOrigin: existing.relayOrigin,
        channelID: existing.channelID,
        relayMetadataPubkey: existing.relayMetadataPubkey,
        displayName: existing.displayName,
        channelType: existing.channelType,
        memberCount: membership.count,
        memberDigests: membership.digests,
        membershipEventID: event.id,
        membershipEventCreatedAt: event.createdAt,
        membershipCachedAt: cachedAt,
        eventID: existing.eventID,
        eventCreatedAt: existing.eventCreatedAt,
        cachedAt: existing.cachedAt
      )
      Self.enforceMemberDigestBound(&snapshot)
    }

    Self.enforceBounds(&snapshot)
    try writeLocked(snapshot)
  }

  /// Attaches an app-rendered thumbnail to every verified profile with this source digest.
  @discardableResult
  public func updateAvatar(
    communityID: String,
    relayOrigin: String,
    sourceURL: String,
    avatarPNG: Data
  ) throws -> Bool {
    guard Self.isBoundedOpaqueID(communityID),
      let canonicalRelayOrigin = Self.canonicalRelayOrigin(relayOrigin),
      let normalizedURL = Self.normalizedAvatarURL(sourceURL),
      let normalizedPNG = Self.normalizedAvatarPNG(avatarPNG)
    else { return false }
    let pictureHash = VerifiedNostrEvent.hex(
      SHA256.hash(data: Data(normalizedURL.utf8))
    )
    lock.lock()
    defer { lock.unlock() }

    var snapshot = loadLocked()
    var changed = false
    for index in snapshot.profiles.indices
    where
      snapshot.profiles[index].communityID == communityID
      && snapshot.profiles[index].relayOrigin == canonicalRelayOrigin
      && snapshot.profiles[index].pictureHash == pictureHash
      && snapshot.profiles[index].avatarPNG != normalizedPNG
    {
      let profile = snapshot.profiles[index]
      snapshot.profiles[index] = BuzzPushCachedProfile(
        communityID: profile.communityID,
        relayOrigin: profile.relayOrigin,
        pubkey: profile.pubkey,
        displayName: profile.displayName,
        pictureHash: profile.pictureHash,
        avatarPNG: normalizedPNG,
        eventID: profile.eventID,
        eventCreatedAt: profile.eventCreatedAt,
        cachedAt: profile.cachedAt
      )
      changed = true
    }
    guard changed else { return false }
    Self.enforceBounds(&snapshot)
    try writeLocked(snapshot)
    return true
  }

  private func loadLocked() -> BuzzPushPresentationCacheSnapshot {
    guard let values = try? fileURL.resourceValues(forKeys: [.fileSizeKey]),
      let fileSize = values.fileSize,
      fileSize <= Self.maximumSnapshotBytes
    else { return BuzzPushPresentationCacheSnapshot() }
    return BuzzPushPresentationCacheSnapshot.decode(try? Data(contentsOf: fileURL))
  }

  private func writeLocked(_ snapshot: BuzzPushPresentationCacheSnapshot) throws {
    let data = try Self.encodedBoundedSnapshot(snapshot)
    try data.write(to: fileURL, options: [.atomic])
    #if os(iOS)
      try FileManager.default.setAttributes(
        [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
        ofItemAtPath: fileURL.path
      )
    #endif
  }

  static func encodedBoundedSnapshot(
    _ snapshot: BuzzPushPresentationCacheSnapshot
  ) throws -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    var bounded = snapshot
    Self.enforceBounds(&bounded)
    var data = try encoder.encode(bounded)

    if data.count > Self.maximumSnapshotBytes {
      var estimatedExcess = data.count - Self.maximumSnapshotBytes + 1_024
      for index in bounded.profiles.indices.reversed() {
        guard let avatar = bounded.profiles[index].avatarPNG else { continue }
        estimatedExcess -= min(estimatedExcess, 4 * ((avatar.count + 2) / 3))
        bounded.profiles[index] = Self.removingAvatar(from: bounded.profiles[index])
        if estimatedExcess == 0 { break }
      }
      data = try encoder.encode(bounded)
    }

    if data.count > Self.maximumSnapshotBytes {
      for index in bounded.channels.indices.reversed() {
        guard bounded.channels[index].memberDigests != nil else { continue }
        bounded.channels[index] = Self.removingMemberDigests(from: bounded.channels[index])
        data = try encoder.encode(bounded)
        if data.count <= Self.maximumSnapshotBytes { break }
      }
    }

    while data.count > Self.maximumSnapshotBytes,
      !bounded.profiles.isEmpty || !bounded.channels.isEmpty
    {
      let entryCount = bounded.profiles.count + bounded.channels.count
      let ratio = Double(Self.maximumSnapshotBytes) / Double(data.count)
      let targetCount = max(0, min(entryCount - 1, Int(Double(entryCount) * ratio * 0.95)))
      Self.removeOldestEntries(entryCount - targetCount, from: &bounded)
      data = try encoder.encode(bounded)
    }
    return data
  }

  static func profileMetadata(
    _ event: VerifiedNostrEvent
  ) -> (displayName: String?, pictureHash: String?) {
    guard event.content.utf8.count <= maximumProfileMetadataBytes,
      let data = event.content.data(using: .utf8),
      let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    else { return (nil, nil) }
    let displayName =
      normalizedDisplayName(object["display_name"] as? String)
      ?? normalizedDisplayName(object["name"] as? String)
    let pictureHash = normalizedAvatarURL(object["picture"] as? String).map {
      VerifiedNostrEvent.hex(SHA256.hash(data: Data($0.utf8)))
    }
    return (displayName, pictureHash)
  }

  static func normalizedDisplayName(_ value: String?) -> String? {
    guard let value else { return nil }
    let collapsed = value.precomposedStringWithCanonicalMapping
      .components(separatedBy: .whitespacesAndNewlines)
      .filter { !$0.isEmpty }
      .joined(separator: " ")
    guard !collapsed.isEmpty else { return nil }
    var bounded = String(collapsed.prefix(128))
    while bounded.utf8.count > 512, !bounded.isEmpty {
      bounded.removeLast()
    }
    return bounded.isEmpty ? nil : bounded
  }

  static func normalizedChannelType(_ value: String?) -> String? {
    guard let value, ["stream", "forum", "dm"].contains(value) else { return nil }
    return value
  }

  static func normalizedChannelMembership(
    _ event: VerifiedNostrEvent,
    communityID: String,
    channelID: String
  ) -> (count: Int, digests: [String]?)? {
    var pubkeys = Set<String>()
    var exceededMemberBound = false
    for tag in event.tags where tag.first == "p" {
      guard tag.count >= 2 else { return nil }
      let pubkey = tag[1].lowercased()
      guard isHexPubkey(pubkey) else { return nil }
      if !exceededMemberBound {
        pubkeys.insert(pubkey)
        exceededMemberBound = pubkeys.count > maximumMembersPerChannel
      }
    }
    let digests =
      exceededMemberBound
      ? nil
      : pubkeys.map {
        BuzzPushPresentationIdentity.channelMember(
          communityID: communityID,
          channelID: channelID,
          pubkey: $0
        )
      }.sorted()
    return (
      exceededMemberBound ? maximumMembersPerChannel + 1 : pubkeys.count,
      digests
    )
  }

  static func normalizedAvatarURL(_ value: String?) -> String? {
    guard let value else { return nil }
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return nil }
    if trimmed.hasPrefix("data:image/") {
      guard trimmed.utf8.count <= maximumProfileMetadataBytes,
        let separator = trimmed.firstIndex(of: ","),
        separator < trimmed.index(before: trimmed.endIndex)
      else { return nil }
      let metadata = trimmed[..<separator].lowercased()
      guard metadata.hasSuffix(";base64"),
        ["data:image/png;base64", "data:image/jpeg;base64", "data:image/webp;base64"]
          .contains(String(metadata))
      else { return nil }
      return trimmed
    }
    guard trimmed.utf8.count <= 2_048,
      let components = URLComponents(string: trimmed),
      components.user == nil, components.password == nil,
      components.host?.isEmpty == false,
      ["http", "https"].contains(components.scheme?.lowercased() ?? "")
    else { return nil }
    return components.url?.absoluteString
  }

  public static func canonicalRelayOrigin(_ value: String) -> String? {
    guard value.utf8.count <= 2_048,
      var components = URLComponents(string: value),
      components.host?.isEmpty == false,
      components.user == nil,
      components.password == nil,
      components.query == nil,
      components.fragment == nil,
      components.path.isEmpty || components.path == "/"
    else { return nil }
    switch components.scheme?.lowercased() {
    case "wss": components.scheme = "https"
    case "ws": components.scheme = "http"
    case "https", "http": break
    default: return nil
    }
    components.path = ""
    guard let result = components.string, result.utf8.count <= 2_048 else { return nil }
    return result
  }

  static func tagValue(_ name: String, in event: VerifiedNostrEvent) -> String? {
    event.tags.first { $0.count >= 2 && $0[0] == name }?[1]
  }

  private static func isBoundedOpaqueID(_ value: String) -> Bool {
    !value.isEmpty && value.utf8.count <= 1_024
  }

  private static func isHexPubkey(_ value: String) -> Bool {
    value.count == 64 && VerifiedNostrEvent.hexBytes(value)?.count == 32
  }

  private static func normalizedAvatarPNG(_ data: Data?) -> Data? {
    guard let data, !data.isEmpty, data.count <= maximumAvatarBytes,
      data.starts(with: [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    else { return nil }
    return data
  }

  static func shouldReplace(
    existingCreatedAt: Int?,
    existingID: String?,
    candidateCreatedAt: Int,
    candidateID: String
  ) -> Bool {
    guard let existingCreatedAt, let existingID else { return true }
    return candidateCreatedAt > existingCreatedAt
      || (candidateCreatedAt == existingCreatedAt && candidateID <= existingID)
  }

  static func enforceBounds(_ snapshot: inout BuzzPushPresentationCacheSnapshot) {
    snapshot.communities = Array(snapshot.communities.prefix(maximumCommunities))
    snapshot.profiles = Array(
      snapshot.profiles.sorted(by: profileNewestFirst).prefix(maximumProfiles)
    )
    enforceChannelCountBound(&snapshot)
    enforceMemberDigestBound(&snapshot)

    var avatarBytes = snapshot.profiles.reduce(0) { $0 + ($1.avatarPNG?.count ?? 0) }
    guard avatarBytes > maximumTotalAvatarBytes else { return }
    for index in snapshot.profiles.indices.reversed() {
      guard let avatar = snapshot.profiles[index].avatarPNG else { continue }
      avatarBytes -= avatar.count
      let profile = snapshot.profiles[index]
      snapshot.profiles[index] = removingAvatar(from: profile)
      if avatarBytes <= maximumTotalAvatarBytes { break }
    }
  }

  private static func enforceChannelCountBound(
    _ snapshot: inout BuzzPushPresentationCacheSnapshot
  ) {
    snapshot.channels = Array(
      snapshot.channels.sorted(by: channelNewestFirst).prefix(maximumChannels)
    )
  }

  private static func enforceMemberDigestBound(
    _ snapshot: inout BuzzPushPresentationCacheSnapshot
  ) {
    snapshot.channels.sort(by: channelNewestFirst)
    var memberDigestCount = snapshot.channels.reduce(0) {
      $0 + ($1.memberDigests?.count ?? 0)
    }
    guard memberDigestCount > maximumTotalMemberDigests else { return }
    for index in snapshot.channels.indices.reversed() {
      guard let memberDigests = snapshot.channels[index].memberDigests else { continue }
      memberDigestCount -= memberDigests.count
      snapshot.channels[index] = removingMemberDigests(from: snapshot.channels[index])
      if memberDigestCount <= maximumTotalMemberDigests { break }
    }
  }

  private static func removingAvatar(
    from profile: BuzzPushCachedProfile
  ) -> BuzzPushCachedProfile {
    BuzzPushCachedProfile(
      communityID: profile.communityID,
      relayOrigin: profile.relayOrigin,
      pubkey: profile.pubkey,
      displayName: profile.displayName,
      pictureHash: profile.pictureHash,
      avatarPNG: nil,
      eventID: profile.eventID,
      eventCreatedAt: profile.eventCreatedAt,
      cachedAt: profile.cachedAt
    )
  }

  private static func removingMemberDigests(
    from channel: BuzzPushCachedChannel
  ) -> BuzzPushCachedChannel {
    BuzzPushCachedChannel(
      communityID: channel.communityID,
      relayOrigin: channel.relayOrigin,
      channelID: channel.channelID,
      relayMetadataPubkey: channel.relayMetadataPubkey,
      displayName: channel.displayName,
      channelType: channel.channelType,
      memberCount: channel.memberCount,
      memberDigests: nil,
      membershipEventID: channel.membershipEventID,
      membershipEventCreatedAt: channel.membershipEventCreatedAt,
      membershipCachedAt: channel.membershipCachedAt,
      eventID: channel.eventID,
      eventCreatedAt: channel.eventCreatedAt,
      cachedAt: channel.cachedAt
    )
  }

  private static func removeOldestEntries(
    _ count: Int,
    from snapshot: inout BuzzPushPresentationCacheSnapshot
  ) {
    for _ in 0..<count {
      switch (snapshot.profiles.last, snapshot.channels.last) {
      case (nil, nil): return
      case (.some, nil): snapshot.profiles.removeLast()
      case (nil, .some): snapshot.channels.removeLast()
      case (let profile?, let channel?):
        let channelCachedAt = channelLastCachedAt(channel)
        if profile.cachedAt < channelCachedAt
          || (profile.cachedAt == channelCachedAt && profile.eventID <= channel.eventID)
        {
          snapshot.profiles.removeLast()
        } else {
          snapshot.channels.removeLast()
        }
      }
    }
  }

  private static func profileNewestFirst(
    _ lhs: BuzzPushCachedProfile,
    _ rhs: BuzzPushCachedProfile
  ) -> Bool {
    lhs.cachedAt == rhs.cachedAt ? lhs.eventID > rhs.eventID : lhs.cachedAt > rhs.cachedAt
  }

  private static func channelNewestFirst(
    _ lhs: BuzzPushCachedChannel,
    _ rhs: BuzzPushCachedChannel
  ) -> Bool {
    let lhsCachedAt = channelLastCachedAt(lhs)
    let rhsCachedAt = channelLastCachedAt(rhs)
    return lhsCachedAt == rhsCachedAt ? lhs.eventID > rhs.eventID : lhsCachedAt > rhsCachedAt
  }

  private static func channelLastCachedAt(_ channel: BuzzPushCachedChannel) -> Int {
    max(channel.cachedAt, channel.membershipCachedAt ?? 0)
  }
}

/// Stable, privacy-preserving identifiers used only after the NSE resolves an event.
public enum BuzzPushPresentationIdentity {
  public static func conversation(communityID: String, channelID: String) -> String {
    scoped(namespace: "conversation", values: [communityID, channelID])
  }

  public static func sender(communityID: String, pubkey: String) -> String {
    scoped(namespace: "sender", values: [communityID, pubkey.lowercased()])
  }

  /// Returns a stable, channel-scoped digest used for exact local membership checks.
  public static func channelMember(
    communityID: String,
    channelID: String,
    pubkey: String
  ) -> String {
    scoped(
      namespace: "channel-member",
      values: [communityID, channelID, pubkey.lowercased()]
    )
  }

  private static func scoped(namespace: String, values: [String]) -> String {
    let encoded = (try? JSONEncoder().encode([namespace] + values)) ?? Data()
    return "buzz.\(namespace).\(VerifiedNostrEvent.hex(SHA256.hash(data: encoded)))"
  }
}
