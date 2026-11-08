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
  /// Cancels any retry in flight and prevents new ones. Call when the notification is delivered.
  func cancel()
  /// Whether the organization session behind the resolved message, if any, still holds.
  func isStillAuthorized() -> Bool
}

/// Reads configured Buzz communities and resolves their newest message not yet shown.
public final class BuzzPushNotificationResolver: BuzzPushNotificationResolving {
  // Each independent refresh has a hard transfer limit, allowing bounded
  // content plus JSON escaping, tags, and envelopes.
  // Each record still passes the cache's separate content and signature checks.
  static let maximumPresentationResponseBytes = 2 * 1_024 * 1_024

  private let session: URLSession
  private let makeRetrySession: () -> URLSession
  // Recursive because a retry that fails synchronously completes while the lock is held,
  // and that completion may finish the notification and call cancel().
  private let retryLock = NSRecursiveLock()
  private var isCancelled = false
  private var retrySessions: [URLSession] = []
  private let loadCommunitiesData: () -> Data?
  private let loadPrivateKey: (String) -> String?
  private let assertionLock = NSLock()
  private var resolvedAssertion: BuzzRelayAssertion?
  private let now: () -> Date
  private let notificationCache: BuzzNotificationCache?
  private let avatarLoader: BuzzNotificationAvatarLoader?
  private let assertionIssuer: BuzzRelayAssertionIssuer?

  /// Creates a resolver around the notification extension's App Group and Keychain I/O.
  public init(
    session: URLSession,
    loadCommunitiesData: @escaping () -> Data?,
    loadPrivateKey: @escaping (String) -> String?,
    notificationCache: BuzzNotificationCache? = nil,
    avatarLoader: BuzzNotificationAvatarLoader? = nil,
    assertionIssuer: BuzzRelayAssertionIssuer? = nil,
    now: @escaping () -> Date = Date.init,
    makeRetrySession: (() -> URLSession)? = nil
  ) {
    self.makeRetrySession = makeRetrySession ?? { URLSession(configuration: session.configuration) }
    self.notificationCache = notificationCache
    self.avatarLoader = avatarLoader
    self.assertionIssuer = assertionIssuer
    self.session = session
    self.loadCommunitiesData = loadCommunitiesData
    self.loadPrivateKey = loadPrivateKey
    self.now = now
  }

  public func resolve(completion: @escaping (BuzzPushResolution?) -> Void) {
    let communities = loadCommunities().filter {
      $0.pubkey?.isEmpty == false
        && accountPrivateKey(for: $0) != nil
        && !$0.policies.isEmpty
    }
    guard !communities.isEmpty else {
      completion(nil)
      return
    }
    let group = DispatchGroup()
    let lock = NSLock()
    var candidates: [(VerifiedNostrEvent, PushLeaseCommunity, BuzzRelayAssertion?)] = []
    for community in communities {
      group.enter()
      query(community) { messages, assertion in
        lock.lock()
        candidates += messages.map { ($0, community, assertion) }
        lock.unlock()
        group.leave()
      }
    }
    group.notify(queue: .global(qos: .userInitiated)) {
      let newestFirst = candidates.sorted {
        $0.0.createdAt == $1.0.createdAt ? $0.0.id < $1.0.id : $0.0.createdAt > $1.0.createdAt
      }
      guard let newest = self.firstUnpresented(newestFirst, communities: communities) else {
        completion(nil)
        return
      }
      self.assertionLock.lock()
      self.resolvedAssertion = newest.2
      self.assertionLock.unlock()
      self.resolvePresentation(event: newest.0, community: newest.1, completion: completion)
    }
  }

  /// The push doesn't say which event it is for, so take the newest message not yet
  /// shown, in any community. Without extension storage, fall back to the newest message.
  /// The claim is made before presentation: if presenting then fails or expires, that
  /// push shows the generic text and the message isn't offered again, which is better
  /// than repeating a message in later notifications.
  private func firstUnpresented(
    _ newestFirst: [(VerifiedNostrEvent, PushLeaseCommunity, BuzzRelayAssertion?)],
    communities: [PushLeaseCommunity]
  ) -> (VerifiedNostrEvent, PushLeaseCommunity, BuzzRelayAssertion?)? {
    guard let notificationCache else { return newestFirst.first }
    let candidates = newestFirst.compactMap { event, community, assertion in
      BuzzNotificationCacheScope(community: community).map {
        (candidate: (event, community, assertion), scope: $0)
      }
    }
    do {
      // Every community's history bounds the others, even one whose query failed, so a
      // community with nothing shown yet can't surface its old messages.
      return try notificationCache.claimFirstUnpresented(
        candidates.map { (event: $0.candidate.0, scope: $0.scope) },
        scopes: communities.compactMap(BuzzNotificationCacheScope.init(community:))
      ).map { candidates[$0].candidate }
    } catch {
      // Unusable storage must not hide new messages; it can only fail to dedupe them.
      NSLog("Notification dedupe unavailable: %@", String(describing: error))
      return candidates.first?.candidate
    }
  }

  private func matchingPrivateKey(for community: PushLeaseCommunity) -> String? {
    guard let scope = BuzzNotificationCacheScope(community: community),
      loadCommunities().contains(where: { BuzzNotificationCacheScope(community: $0) == scope })
    else { return nil }
    return accountPrivateKey(for: community)
  }

  // Initial requests share the snapshot loaded by resolve. Asynchronous results
  // use matchingPrivateKey to check that the account and session are still current.
  private func accountPrivateKey(for community: PushLeaseCommunity) -> String? {
    guard let scope = BuzzNotificationCacheScope(community: community),
      let key = loadPrivateKey(community.id), scope.matchesAccountKey(key)
    else { return nil }
    return key
  }

  private func query(
    _ community: PushLeaseCommunity,
    completion: @escaping ([VerifiedNostrEvent], BuzzRelayAssertion?) -> Void
  ) {
    guard let privateKey = accountPrivateKey(for: community), community.pubkey?.isEmpty == false else {
      completion([], nil)
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
      completion([], nil)
      return
    }
    withFederatedIdentity(relayURL: relayURL, privateKey: privateKey) {
      (assertion, finish: @escaping ((Data?, URLResponse?), Bool) -> Void) in
      self.send(
        url: url, body: body, privateKey: privateKey, federatedIdentity: assertion?.header,
        session: self.session,
        canRetry: {
          self.matchingPrivateKey(for: community) == privateKey && self.isCurrent(assertion)
        }
      ) { data, response in
        finish((data, response), BuzzRelayAssertionIssuer.isChallenge(response))
      }
    } completion: { result, assertion in
      let (data, response) = result
      guard self.matchingPrivateKey(for: community) != nil, self.isCurrent(assertion),
        let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode),
        let data, let events = try? JSONDecoder().decode([VerifiedNostrEvent].self, from: data)
      else {
        completion([], assertion)
        return
      }
      completion(
        Self.messages(
          events: events.filter { event in
            event.hasValidIDAndSignature()
              && community.policies.contains { policy in
                PushLeaseMatcher.matches(event: event, policy: policy)
              }
          },
          community: community
        ), assertion)
    }
  }

  /// Runs `attempt` with the relay's NIP-FI assertion, if any. A hosted relay is tried without
  /// one first; only if it challenges does the extension ask Builderlab, then retry once.
  private func withFederatedIdentity<Output>(
    relayURL: URL, privateKey: String,
    attempt: @escaping (BuzzRelayAssertion?, @escaping (Output, Bool) -> Void) -> Void,
    completion: @escaping (Output, BuzzRelayAssertion?) -> Void
  ) {
    federatedIdentity(relayURL: relayURL, privateKey: privateKey) { assertion in
      attempt(assertion) { output, challenged in
        guard challenged, assertion == nil, let assertionIssuer = self.assertionIssuer else {
          completion(output, assertion)
          return
        }
        assertionIssuer.header(relayURL: relayURL, privateKeyHex: privateKey, challenged: true) {
          answer in
          guard let answer else {
            completion(output, nil)
            return
          }
          attempt(answer) { output, _ in completion(output, answer) }
        }
      }
    }
  }

  /// The NIP-FI assertion an organization relay requires, or nil for any other relay.
  private func federatedIdentity(
    relayURL: URL, privateKey: String, completion: @escaping (BuzzRelayAssertion?) -> Void
  ) {
    guard let assertionIssuer else {
      completion(nil)
      return
    }
    assertionIssuer.header(relayURL: relayURL, privateKeyHex: privateKey, completion: completion)
  }

  /// Whether the organization session behind the resolved message, if any, is still the shared
  /// one. The extension checks this again before delivering, as it does the account scope.
  public func isStillAuthorized() -> Bool {
    assertionLock.lock()
    let assertion = resolvedAssertion
    assertionLock.unlock()
    return isCurrent(assertion)
  }

  /// Responses fetched with an assertion are accepted only while its organization session lasts.
  private func isCurrent(_ assertion: BuzzRelayAssertion?) -> Bool {
    guard let assertion else { return true }
    return assertionIssuer?.isCurrent(assertion) ?? false
  }

  /// Sends one signed `/query`. A timeout or dropped connection is retried once on a new
  /// session, because the failed attempt may have used a pooled connection that went stale
  /// when the network path changed. Each attempt is signed afresh so the relay's NIP-98
  /// replay guard accepts it. `canRetry` rechecks the account first; nil means no retry.
  public func cancel() {
    retryLock.lock()
    isCancelled = true
    let sessions = retrySessions
    retrySessions = []
    retryLock.unlock()
    sessions.forEach { $0.invalidateAndCancel() }
  }

  private func send(
    url: URL, body: Data, privateKey: String, federatedIdentity: String?, session: URLSession,
    canRetry: (() -> Bool)?,
    completion: @escaping (Data?, URLResponse?) -> Void
  ) {
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
      completion(nil, nil)
      return
    }
    request.setValue(auth, forHTTPHeaderField: "Authorization")
    if let federatedIdentity {
      request.setValue(federatedIdentity, forHTTPHeaderField: "Nostr-Federated-Identity")
    }
    let task = session.dataTask(with: request) { data, response, error in
      guard let error = error as? URLError,
        [.timedOut, .networkConnectionLost].contains(error.code), canRetry?() == true
      else {
        completion(data, response)
        return
      }
      // Creating the retry under the lock means cancel() either stops it or sees its session.
      self.retryLock.lock()
      guard !self.isCancelled else {
        self.retryLock.unlock()
        completion(data, response)
        return
      }
      defer { self.retryLock.unlock() }
      let retrySession = self.makeRetrySession()
      self.retrySessions.append(retrySession)
      self.send(
        url: url, body: body, privateKey: privateKey, federatedIdentity: federatedIdentity,
        session: retrySession, canRetry: nil
      ) { data, response in
        retrySession.finishTasksAndInvalidate()
        completion(data, response)
      }
    }
    // A redirect would forward the federated identity header to its target, so it is refused.
    task.delegate = BuzzRedirectRefusal.shared
    task.resume()
  }

  private func resolvePresentation(
    event: VerifiedNostrEvent,
    community: PushLeaseCommunity,
    completion: @escaping (BuzzPushResolution?) -> Void
  ) {
    guard matchingPrivateKey(for: community) != nil else {
      completion(nil)
      return
    }
    if let notificationCache {
      guard let scope = BuzzNotificationCacheScope(community: community) else {
        completion(nil)
        return
      }
      let operation = BuzzNotificationEnrichmentOperation(
        event: event, community: community, scope: scope, cache: notificationCache,
        avatarLoader: avatarLoader, now: now,
        isCurrent: { [self] in
          matchingPrivateKey(for: community) != nil && isStillAuthorized()
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
    event: VerifiedNostrEvent, community: PushLeaseCommunity,
    refreshProfile: Bool, refreshChannel: Bool,
    completion: @escaping (BuzzNotificationEnrichmentOperation.RefreshResult) -> Void
  ) {
    // A full channel roster can be much larger than a profile. Keep these
    // transfers independent so its size or latency cannot hide the sender.
    if refreshProfile {
      queryPresentation(
        filters: [["kinds": [0], "authors": [event.pubkey.lowercased()], "limit": 1]],
        community: community
      ) { events in
        completion(.profile(Self.newest(events.filter {
          $0.kind == 0 && $0.pubkey.lowercased() == event.pubkey.lowercased()
        })))
      }
    }
    if refreshChannel {
      guard let channelID = Self.tagValue("h", in: event),
        let authority = community.relayMetadataPubkey?.lowercased()
      else {
        completion(.channel(nil, nil))
        return
      }
      let group = DispatchGroup()
      let lock = NSLock()
      var records: [Int: VerifiedNostrEvent] = [:]
      for kind in [39000, 39002] {
        group.enter()
        queryPresentation(filters: [[
          "kinds": [kind], "authors": [authority], "#d": [channelID], "limit": 1,
        ]], community: community) { events in
          let record = Self.newest(events.filter {
            $0.kind == kind && $0.pubkey.lowercased() == authority
              && Self.tagValue("d", in: $0) == channelID
          })
          lock.lock()
          records[kind] = record
          lock.unlock()
          group.leave()
        }
      }
      group.notify(queue: .global(qos: .utility)) {
        completion(.channel(records[39000], records[39002]))
      }
    }
  }

  private func queryPresentation(
    filters: [[String: Any]], community: PushLeaseCommunity,
    completion: @escaping ([VerifiedNostrEvent]) -> Void
  ) {
    guard let privateKey = matchingPrivateKey(for: community),
      let relayURL = community.relayURL,
      let url = URL(string: "/query", relativeTo: relayURL),
      let body = try? JSONSerialization.data(withJSONObject: filters)
    else {
      completion([])
      return
    }
    withFederatedIdentity(relayURL: relayURL, privateKey: privateKey) {
      (assertion, finish: @escaping (Result<Data, Error>?, Bool) -> Void) in
      var request = URLRequest(url: url)
      request.httpMethod = "POST"
      request.httpBody = body
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      guard let auth = try? NostrHTTPAuth.authorizationHeader(
        url: url, method: "POST", body: body, privateKeyHex: privateKey)
      else {
        finish(nil, false)
        return
      }
      request.setValue(auth, forHTTPHeaderField: "Authorization")
      if let assertion {
        request.setValue(assertion.header, forHTTPHeaderField: "Nostr-Federated-Identity")
      }
      BuzzNotificationDownload.start(
        request: request, configuration: self.session.configuration,
        maximumBytes: Self.maximumPresentationResponseBytes
      ) { result in
        guard case .failure(BuzzNotificationDownload.Failure.challenged) = result else {
          finish(result, false)
          return
        }
        finish(result, true)
      }
    } completion: { result, assertion in
      guard self.matchingPrivateKey(for: community) != nil, self.isCurrent(assertion),
        case .success(let data)? = result,
        let events = try? JSONDecoder().decode([VerifiedNostrEvent].self, from: data)
      else {
        completion([])
        return
      }
      completion(events.filter { $0.hasValidIDAndSignature() })
    }
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
    messages(events: events, community: community).first
  }

  /// Messages from others that a notification can show, newest first.
  private static func messages(
    events: [VerifiedNostrEvent],
    community: PushLeaseCommunity
  ) -> [VerifiedNostrEvent] {
    guard let mine = community.pubkey?.lowercased() else { return [] }
    return events.filter {
      $0.pubkey.lowercased() != mine && [9, 40002, 45001, 45003].contains($0.kind)
    }.sorted {
      $0.createdAt == $1.createdAt ? $0.id < $1.id : $0.createdAt > $1.createdAt
    }
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
      isSolo: metadataEvent.map {
        BuzzPushPresentationCacheStore.isSoloChannel($0)
      } ?? cached?.isSolo,
      creatorPubkey: metadataEvent.map {
        BuzzPushPresentationCacheStore.normalizedCreator($0)
      } ?? cached?.creatorPubkey,
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
    // Your own Solo chats are personal conversations, not channels, so they
    // get no `#`. Like the app (`Channel.isPersonalChat`), someone else's
    // Solo chat you belong to stays an ordinary channel.
    if channel.isSolo == true, channel.creatorPubkey == currentUser {
      return (displayName, recipientCount)
    }
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

/// Refuses every redirect, so a signed relay request never forwards its proof or assertion.
final class BuzzRedirectRefusal: NSObject, URLSessionTaskDelegate, Sendable {
  static let shared = BuzzRedirectRefusal()

  func urlSession(
    _ session: URLSession, task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
    completionHandler: @escaping (URLRequest?) -> Void
  ) {
    completionHandler(nil)
  }
}
