import Foundation

#if canImport(FoundationNetworking)
  import FoundationNetworking
#endif

/// Content resolved from unread Buzz events for a mutable push notification.
public struct BuzzPushResolution: Decodable, Equatable, Sendable {
  /// Local sign-in that authorized this result; checked again before delivery.
  public let notificationScope: BuzzNotificationCacheScope?
  public let title: String
  public let body: String
  public let subtitle: String?
  public let threadIdentifier: String?
  public let navigationTarget: BuzzPushNavigationTarget?
  public let senderPubkey: String?
  public let senderAvatarPNG: Data?
  public let conversationIdentifier: String?
  public let conversationDisplayName: String?
  /// Exact verified recipient count for Communication Notifications specialization.
  public let conversationRecipientCount: Int?

  public init(
    title: String,
    body: String,
    subtitle: String?,
    threadIdentifier: String?,
    navigationTarget: BuzzPushNavigationTarget? = nil,
    senderPubkey: String? = nil,
    senderAvatarPNG: Data? = nil,
    conversationIdentifier: String? = nil,
    conversationDisplayName: String? = nil,
    conversationRecipientCount: Int? = nil,
    notificationScope: BuzzNotificationCacheScope? = nil
  ) {
    self.notificationScope = notificationScope
    self.title = title
    self.body = body
    self.subtitle = subtitle
    self.threadIdentifier = threadIdentifier
    self.navigationTarget = navigationTarget
    self.senderPubkey = senderPubkey
    self.senderAvatarPNG = senderAvatarPNG
    self.conversationIdentifier = conversationIdentifier
    self.conversationDisplayName = conversationDisplayName
    self.conversationRecipientCount = conversationRecipientCount
  }
}

/// Resolves the content used to mutate a generic Buzz push notification.
public protocol BuzzPushNotificationResolving {
  func resolve(completion: @escaping (BuzzPushResolution?) -> Void)
}

/// Reads configured Buzz communities and resolves their newest unread event.
public final class BuzzPushNotificationResolver: BuzzPushNotificationResolving {
  // Verified profiles may carry bounded inline raster avatars. Keep the refresh
  // response capped, but large enough to recover the sender name from those
  // otherwise valid kind-0 events when the app cache has not been populated.
  static let maximumPresentationResponseBytes = 256 * 1_024

  private let session: URLSession
  private let loadCommunitiesData: () -> Data?
  private let loadPrivateKey: (String) -> String?
  private let now: () -> Date
  private let notificationCache: BuzzNotificationCache?
  private let avatarLoader: BuzzNotificationAvatarLoader?

  /// Creates a resolver around the notification extension's App Group and Keychain I/O.
  public init(
    session: URLSession,
    loadCommunitiesData: @escaping () -> Data?,
    loadPrivateKey: @escaping (String) -> String?,
    notificationCache: BuzzNotificationCache? = nil,
    avatarLoader: BuzzNotificationAvatarLoader? = nil,
    now: @escaping () -> Date = Date.init
  ) {
    self.notificationCache = notificationCache
    self.avatarLoader = avatarLoader
    self.session = session
    self.loadCommunitiesData = loadCommunitiesData
    self.loadPrivateKey = loadPrivateKey
    self.now = now
  }

  public func resolve(completion: @escaping (BuzzPushResolution?) -> Void) {
    let communities = loadCommunities().filter {
      $0.pubkey?.isEmpty == false
        && loadPrivateKey($0.id) != nil
        && !$0.policies.isEmpty
    }
    guard !communities.isEmpty else {
      completion(nil)
      return
    }
    let group = DispatchGroup()
    let lock = NSLock()
    var candidates: [(VerifiedNostrEvent, PushLeaseCommunity)] = []
    for community in communities {
      group.enter()
      query(community) { candidate in
        if let candidate {
          lock.lock()
          candidates.append(candidate)
          lock.unlock()
        }
        group.leave()
      }
    }
    group.notify(queue: .global(qos: .userInitiated)) {
      let newest = candidates.max {
        $0.0.createdAt == $1.0.createdAt ? $0.0.id > $1.0.id : $0.0.createdAt < $1.0.createdAt
      }
      guard let newest else {
        completion(nil)
        return
      }
      self.resolvePresentation(event: newest.0, community: newest.1, completion: completion)
    }
  }

  private func query(
    _ community: PushLeaseCommunity,
    completion: @escaping ((VerifiedNostrEvent, PushLeaseCommunity)?) -> Void
  ) {
    guard let privateKey = loadPrivateKey(community.id), community.pubkey?.isEmpty == false else {
      completion(nil)
      return
    }
    guard
      !community.policies.isEmpty,
      let relayURL = community.relayURL,
      let url = URL(string: "/query", relativeTo: relayURL),
      let body = try? JSONSerialization.data(
        withJSONObject: community.policies.map { $0.filter.queryFilter(since: nil, limit: 10) }
      )
    else {
      completion(nil)
      return
    }
    var request = URLRequest(url: url)
    request.httpMethod = "POST"
    request.httpBody = body
    request.timeoutInterval = 8
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    guard
      let auth = try? NostrHTTPAuth.authorizationHeader(
        url: url, method: "POST", body: body, privateKeyHex: privateKey
      )
    else {
      completion(nil)
      return
    }
    request.setValue(auth, forHTTPHeaderField: "Authorization")
    session.dataTask(with: request) { data, response, _ in
      guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode),
        let data, let events = try? JSONDecoder().decode([VerifiedNostrEvent].self, from: data)
      else {
        completion(nil)
        return
      }
      let candidate = Self.newestMessage(
        events: events.filter { event in
          event.hasValidIDAndSignature()
            && community.policies.contains { policy in
              PushLeaseMatcher.matches(event: event, policy: policy)
            }
        },
        community: community
      )
      completion(candidate.map { ($0, community) })
    }.resume()
  }

  private func resolvePresentation(
    event: VerifiedNostrEvent,
    community: PushLeaseCommunity,
    completion: @escaping (BuzzPushResolution?) -> Void
  ) {
    if let notificationCache {
      guard let scope = BuzzNotificationCacheScope(community: community) else {
        completion(nil)
        return
      }
      let operation = BuzzNotificationEnrichmentOperation(
        event: event, community: community, scope: scope, cache: notificationCache,
        avatarLoader: avatarLoader, now: now,
        isCurrent: { [self] in
          loadCommunities().contains {
            BuzzNotificationCacheScope(community: $0) == scope
          } && loadPrivateKey(community.id) != nil
        },
        refresh: { [self] profile, channel, done in
          refreshPresentation(event: event, community: community,
            refreshProfile: profile, refreshChannel: channel, completion: done)
        }, completion: completion)
      operation.start()
      return
    }
    // Without extension storage, keep the verified message but omit enrichment.
    completion(Self.makeResolution(event: event, community: community, profile: nil, channel: nil))
  }

  private func refreshPresentation(
    event: VerifiedNostrEvent,
    community: PushLeaseCommunity,
    refreshProfile: Bool,
    refreshChannel: Bool,
    completion:
      @escaping (
        VerifiedNostrEvent?, VerifiedNostrEvent?, VerifiedNostrEvent?
      ) -> Void
  ) {
    guard let privateKey = loadPrivateKey(community.id),
      let relayURL = community.relayURL,
      let url = URL(string: "/query", relativeTo: relayURL)
    else {
      completion(nil, nil, nil)
      return
    }
    let channelID = Self.tagValue("h", in: event)
    let relayMetadataPubkey = community.relayMetadataPubkey?.lowercased()
    var filters: [[String: Any]] = []
    if refreshProfile {
      filters.append(["kinds": [0], "authors": [event.pubkey.lowercased()], "limit": 1])
    }
    if refreshChannel, let channelID, let relayMetadataPubkey {
      filters.append([
        "kinds": [39_000],
        "authors": [relayMetadataPubkey],
        "#d": [channelID],
        "limit": 1,
      ])
      filters.append([
        "kinds": [39_002],
        "authors": [relayMetadataPubkey],
        "#d": [channelID],
        "limit": 1,
      ])
    }
    guard !filters.isEmpty,
      let body = try? JSONSerialization.data(withJSONObject: filters)
    else {
      completion(nil, nil, nil)
      return
    }

    var request = URLRequest(url: url)
    request.httpMethod = "POST"
    request.httpBody = body
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    guard
      let auth = try? NostrHTTPAuth.authorizationHeader(
        url: url,
        method: "POST",
        body: body,
        privateKeyHex: privateKey
      )
    else {
      completion(nil, nil, nil)
      return
    }
    request.setValue(auth, forHTTPHeaderField: "Authorization")
    session.downloadTask(with: request) { fileURL, response, _ in
      guard let response = response as? HTTPURLResponse,
        (200..<300).contains(response.statusCode),
        let fileURL,
        let values = try? fileURL.resourceValues(forKeys: [.fileSizeKey]),
        let fileSize = values.fileSize,
        fileSize <= Self.maximumPresentationResponseBytes,
        let data = try? Data(contentsOf: fileURL),
        let events = try? JSONDecoder().decode([VerifiedNostrEvent].self, from: data)
      else {
        completion(nil, nil, nil)
        return
      }
      let verified = events.filter { $0.hasValidIDAndSignature() }
      let profile =
        refreshProfile
        ? Self.newest(
          verified.filter {
            $0.kind == 0 && $0.pubkey.lowercased() == event.pubkey.lowercased()
          }) : nil
      let channel =
        refreshChannel
        ? channelID.flatMap { channelID in
          Self.newest(
            verified.filter {
              $0.kind == 39_000
                && $0.pubkey.lowercased() == relayMetadataPubkey
                && Self.tagValue("d", in: $0) == channelID
            })
        } : nil
      let membership =
        refreshChannel
        ? channelID.flatMap { channelID in
          Self.newest(
            verified.filter {
              $0.kind == 39_002
                && $0.pubkey.lowercased() == relayMetadataPubkey
                && Self.tagValue("d", in: $0) == channelID
            })
        } : nil
      completion(profile, channel, membership)
    }.resume()
  }

  static func decodeResolution(
    events: [VerifiedNostrEvent], community: PushLeaseCommunity
  ) -> (BuzzPushResolution, VerifiedNostrEvent)? {
    let event = newestMessage(events: events, community: community)
    guard let event else { return nil }
    guard
      let resolution = makeResolution(
        event: event,
        community: community,
        profile: nil,
        channel: nil
      )
    else { return nil }
    return (resolution, event)
  }

  private static func newestMessage(
    events: [VerifiedNostrEvent],
    community: PushLeaseCommunity
  ) -> VerifiedNostrEvent? {
    guard let mine = community.pubkey?.lowercased() else { return nil }
    return events.filter {
      $0.pubkey.lowercased() != mine && [9, 40002, 45001, 45003].contains($0.kind)
    }.sorted {
      $0.createdAt == $1.createdAt ? $0.id < $1.id : $0.createdAt > $1.createdAt
    }.first
  }

  static func makeResolution(
    event: VerifiedNostrEvent,
    community: PushLeaseCommunity,
    profile: BuzzPushCachedProfile?,
    channel: BuzzPushCachedChannel?
  ) -> BuzzPushResolution? {
    let body = previewBody(event.content)
    guard !body.isEmpty else { return nil }
    let channelID = tagValue("h", in: event)
    let conversationIdentifier = channelID.map {
      BuzzPushPresentationIdentity.conversation(communityID: community.id, channelID: $0)
    }
    let conversation = channel.flatMap {
      communicationConversation(event: event, community: community, channel: $0)
    }
    return BuzzPushResolution(
      title: profile?.displayName ?? "",
      body: body,
      subtitle: community.name,
      threadIdentifier: conversationIdentifier ?? community.id,
      navigationTarget: channelID.map {
        BuzzPushNavigationTarget(
          eventID: event.id,
          communityID: community.id,
          channelID: $0,
          threadRootID: BuzzPushNavigationTarget.threadRootID(in: event.tags)
        )
      },
      senderPubkey: event.pubkey.lowercased(),
      senderAvatarPNG: profile?.avatarPNG,
      conversationIdentifier: conversationIdentifier,
      conversationDisplayName: conversation?.displayName,
      conversationRecipientCount: conversation?.recipientCount,
      notificationScope: BuzzNotificationCacheScope(community: community)
    )
  }

  static func ephemeralChannel(
    metadataEvent: VerifiedNostrEvent?,
    membershipEvent: VerifiedNostrEvent?,
    cached: BuzzPushCachedChannel?,
    communityID: String,
    relayOrigin: String,
    relayMetadataPubkey: String,
    cachedAt: Int
  ) -> BuzzPushCachedChannel? {
    guard metadataEvent != nil || cached != nil else { return nil }
    let channelID = metadataEvent.flatMap { tagValue("d", in: $0) } ?? cached?.channelID
    guard let channelID, !channelID.isEmpty,
      let eventID = metadataEvent?.id ?? cached?.eventID,
      let eventCreatedAt = metadataEvent?.createdAt ?? cached?.eventCreatedAt,
      let metadataCachedAt = metadataEvent == nil ? cached?.cachedAt : cachedAt
    else { return nil }
    let membership = membershipEvent.flatMap {
      BuzzPushPresentationCacheStore.normalizedChannelMembership(
        $0,
        communityID: communityID,
        channelID: channelID
      )
    }
    let acceptedMembershipEvent = membership == nil ? nil : membershipEvent
    return BuzzPushCachedChannel(
      communityID: communityID,
      relayOrigin: relayOrigin,
      channelID: channelID,
      relayMetadataPubkey: relayMetadataPubkey,
      displayName: metadataEvent.map {
        BuzzPushPresentationCacheStore.normalizedDisplayName(tagValue("name", in: $0))
      } ?? cached?.displayName,
      channelType: metadataEvent.map {
        BuzzPushPresentationCacheStore.normalizedChannelType(tagValue("t", in: $0))
      } ?? cached?.channelType,
      memberCount: membership?.count ?? cached?.memberCount,
      memberDigests: membership?.digests ?? cached?.memberDigests,
      membershipEventID: acceptedMembershipEvent?.id ?? cached?.membershipEventID,
      membershipEventCreatedAt: acceptedMembershipEvent?.createdAt
        ?? cached?.membershipEventCreatedAt,
      membershipCachedAt: acceptedMembershipEvent == nil
        ? cached?.membershipCachedAt : cachedAt,
      eventID: eventID,
      eventCreatedAt: eventCreatedAt,
      cachedAt: metadataCachedAt
    )
  }

  private static func communicationConversation(
    event: VerifiedNostrEvent,
    community: PushLeaseCommunity,
    channel: BuzzPushCachedChannel
  ) -> (displayName: String?, recipientCount: Int)? {
    guard let currentUser = community.pubkey?.lowercased(),
      let memberCount = channel.memberCount,
      let memberDigests = channel.memberDigests,
      memberDigests.count == memberCount
    else { return nil }
    let currentUserDigest = BuzzPushPresentationIdentity.channelMember(
      communityID: community.id,
      channelID: channel.channelID,
      pubkey: currentUser
    )
    guard memberDigests.contains(currentUserDigest) else { return nil }
    let senderDigest = BuzzPushPresentationIdentity.channelMember(
      communityID: community.id,
      channelID: channel.channelID,
      pubkey: event.pubkey
    )
    let senderIsMember = memberDigests.contains(senderDigest)
    let recipientCount = memberCount - (senderIsMember ? 1 : 0)
    guard recipientCount > 0 else { return nil }

    if channel.channelType == "dm" {
      guard memberCount == 2, senderIsMember else { return nil }
      return (nil, recipientCount)
    }
    guard let channelType = channel.channelType,
      ["stream", "forum"].contains(channelType),
      let displayName = channel.displayName
    else { return nil }
    return (displayName.hasPrefix("#") ? displayName : "#\(displayName)", recipientCount)
  }

  private static func newest(_ events: [VerifiedNostrEvent]) -> VerifiedNostrEvent? {
    events.sorted {
      $0.createdAt == $1.createdAt ? $0.id < $1.id : $0.createdAt > $1.createdAt
    }.first
  }

  static func tagValue(_ name: String, in event: VerifiedNostrEvent) -> String? {
    event.tags.first { $0.count >= 2 && $0[0] == name }?[1]
  }

  static func previewBody(_ content: String) -> String {
    var result = content.replacingOccurrences(
      of: #"```[\s\S]*?```"#, with: "[code]", options: .regularExpression)
    result = result.replacingOccurrences(of: #"`([^`]*)`"#, with: "$1", options: .regularExpression)
    result = result.replacingOccurrences(
      of: #"!?\[([^\]]*)\]\([^)]*\)"#, with: "$1", options: .regularExpression)
    result = result.replacingOccurrences(
      of: #"https?://\S+"#, with: "[link]", options: .regularExpression)
    result = result.replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
      .trimmingCharacters(in: .whitespacesAndNewlines)
    return result.count > 180
      ? String(result.prefix(177)).trimmingCharacters(in: .whitespacesAndNewlines) + "…" : result
  }

  /// Compact sender label for unnamed senders: the first 8 and last 4
  /// characters of the sender's full npub, matching the compact npub shape
  /// used across Buzz. A key that cannot be encoded as an npub falls back to
  /// the neutral "Someone" identity so malformed payloads leak no key
  /// material while the notification keeps a useful title.
  static func shortPubkey(_ pubkey: String) -> String {
    guard let npub = Bech32.canonicalNpub(from: pubkey) else { return "Someone" }
    return npub.count > 12
      ? String(npub.prefix(8)) + "…" + String(npub.suffix(4)) : npub
  }

  private func loadCommunities() -> [PushLeaseCommunity] {
    guard let data = loadCommunitiesData(),
      let decoded = try? JSONDecoder().decode(PushLeaseSnapshot.self, from: data)
    else { return [] }
    return decoded.communities
  }
}

extension PushLeaseCommunity {
  var relayURL: URL? {
    guard var components = URLComponents(string: relayUrl),
      components.host?.isEmpty == false,
      components.user == nil,
      components.password == nil,
      components.query == nil,
      components.fragment == nil,
      components.path.isEmpty || components.path == "/"
    else { return nil }
    components.scheme =
      switch components.scheme?.lowercased() {
      case "wss": "https"
      case "ws": "http"
      case "https": "https"
      case "http": "http"
      default: nil
      }
    components.path = ""
    return components.url
  }
}
