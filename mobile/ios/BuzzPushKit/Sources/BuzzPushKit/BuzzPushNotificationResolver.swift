import Foundation

#if canImport(FoundationNetworking)
  import FoundationNetworking
#endif

/// Content resolved from unread Buzz events for a mutable push notification.
public struct BuzzPushResolution: Decodable, Equatable, Sendable {
  public let title: String
  public let body: String
  public let subtitle: String?
  public let threadIdentifier: String?

  public init(title: String, body: String, subtitle: String?, threadIdentifier: String?) {
    self.title = title
    self.body = body
    self.subtitle = subtitle
    self.threadIdentifier = threadIdentifier
  }
}

/// Resolves the content used to mutate a generic Buzz push notification.
public protocol BuzzPushNotificationResolving {
  func resolve(completion: @escaping (BuzzPushResolution?) -> Void)
}

/// Reads configured Buzz communities and resolves their newest unread event.
public final class BuzzPushNotificationResolver: BuzzPushNotificationResolving {
  private let session: URLSession
  private let defaults: UserDefaults?
  private let loadCommunitiesData: () -> Data?
  private let loadPrivateKey: (String) -> String?
  private let now: () -> Int

  /// Creates a resolver around the notification extension's App Group and Keychain I/O.
  public init(
    session: URLSession,
    defaults: UserDefaults?,
    loadCommunitiesData: @escaping () -> Data?,
    loadPrivateKey: @escaping (String) -> String?,
    now: @escaping () -> Int = { Int(Date().timeIntervalSince1970) }
  ) {
    self.session = session
    self.defaults = defaults
    self.loadCommunitiesData = loadCommunitiesData
    self.loadPrivateKey = loadPrivateKey
    self.now = now
  }

  public func resolve(completion: @escaping (BuzzPushResolution?) -> Void) {
    let loadedCommunities = loadCommunities()
    removeStaleWatermarks(activeCommunityIDs: Set(loadedCommunities.map(\.id)))
    let communities = loadedCommunities.filter {
      $0.pubkey?.isEmpty == false && loadPrivateKey($0.id) != nil
    }
    guard !communities.isEmpty else { completion(nil); return }
    let group = DispatchGroup()
    let lock = NSLock()
    var candidates: [(BuzzPushResolution, VerifiedNostrEvent, BuzzPushCommunity)] = []
    for community in communities {
      group.enter()
      query(community) { candidate in
        if let candidate {
          lock.lock(); candidates.append((candidate.0, candidate.1, community)); lock.unlock()
        }
        group.leave()
      }
    }
    group.notify(queue: .global(qos: .userInitiated)) { [weak self] in
      guard let self else { return }
      let newest = candidates.max {
        $0.1.createdAt == $1.1.createdAt ? $0.1.id > $1.1.id : $0.1.createdAt < $1.1.createdAt
      }
      for candidate in candidates {
        self.defaults?.set(
          PushWatermark.persistedTimestamp(eventTimestamp: candidate.1.createdAt),
          forKey: PushWatermark.key(communityID: candidate.2.id)
        )
      }
      completion(newest?.0)
    }
  }

  private func query(
    _ community: BuzzPushCommunity,
    completion: @escaping ((BuzzPushResolution, VerifiedNostrEvent)?) -> Void
  ) {
    guard let privateKey = loadPrivateKey(community.id), let pubkey = community.pubkey else {
      completion(nil); return
    }
    var filter: [String: Any] = ["kinds": [9, 40002, 45001, 45003], "#p": [pubkey], "limit": 10]
    let watermarkKey = PushWatermark.key(communityID: community.id)
    let storedWatermark = defaults?.integer(forKey: watermarkKey) ?? 0
    let watermark = PushWatermark.queryTimestamp(storedWatermark: storedWatermark)
    if watermark != storedWatermark { defaults?.set(watermark, forKey: watermarkKey) }
    if let since = PushWatermark.querySince(watermark: watermark) { filter["since"] = since }
    guard let body = try? JSONSerialization.data(withJSONObject: [filter]) else { completion(nil); return }
    let url = URL(string: "/query", relativeTo: community.relayURL)!
    var request = URLRequest(url: url)
    request.httpMethod = "POST"; request.httpBody = body; request.timeoutInterval = 8
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    guard let auth = try? NostrHTTPAuth.authorizationHeader(
      url: url, method: "POST", body: body, privateKeyHex: privateKey
    ) else { completion(nil); return }
    request.setValue(auth, forHTTPHeaderField: "Authorization")
    session.dataTask(with: request) { data, response, _ in
      guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode),
        let data, let events = try? JSONDecoder().decode([VerifiedNostrEvent].self, from: data)
      else { completion(nil); return }
      completion(Self.decodeResolution(
        events: events.filter {
          $0.hasValidIDAndSignature()
            && PushWatermark.isAcceptable(eventTimestamp: $0.createdAt, now: self.now())
        },
        community: community
      ))
    }.resume()
  }

  static func decodeResolution(
    events: [VerifiedNostrEvent], community: BuzzPushCommunity
  ) -> (BuzzPushResolution, VerifiedNostrEvent)? {
    guard let mine = community.pubkey?.lowercased() else { return nil }
    let event = events.filter {
      $0.pubkey.lowercased() != mine && [9, 40002, 45001, 45003].contains($0.kind)
    }.sorted {
      $0.createdAt == $1.createdAt ? $0.id < $1.id : $0.createdAt > $1.createdAt
    }.first
    guard let event else { return nil }
    let body = previewBody(event.content)
    guard !body.isEmpty else { return nil }
    let channel = event.tags.first { $0.count >= 2 && $0[0] == "h" }?[1]
    return (BuzzPushResolution(
      title: shortPubkey(event.pubkey), body: body, subtitle: community.name,
      threadIdentifier: channel ?? community.id
    ), event)
  }

  static func previewBody(_ content: String) -> String {
    var result = content.replacingOccurrences(of: #"```[\s\S]*?```"#, with: "[code]", options: .regularExpression)
    result = result.replacingOccurrences(of: #"`([^`]*)`"#, with: "$1", options: .regularExpression)
    result = result.replacingOccurrences(of: #"!?\[([^\]]*)\]\([^)]*\)"#, with: "$1", options: .regularExpression)
    result = result.replacingOccurrences(of: #"https?://\S+"#, with: "[link]", options: .regularExpression)
    result = result.replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression).trimmingCharacters(in: .whitespacesAndNewlines)
    return result.count > 180 ? String(result.prefix(177)).trimmingCharacters(in: .whitespacesAndNewlines) + "…" : result
  }

  static func shortPubkey(_ pubkey: String) -> String {
    pubkey.count > 8 ? String(pubkey.prefix(8)) + "…" : pubkey
  }

  private func loadCommunities() -> [BuzzPushCommunity] {
    guard let data = loadCommunitiesData(),
      let decoded = try? JSONDecoder().decode(BuzzPushSnapshot.self, from: data)
    else { return [] }
    return decoded.communities
  }

  private func removeStaleWatermarks(activeCommunityIDs: Set<String>) {
    guard let defaults else { return }
    for key in PushWatermark.staleKeys(
      in: Array(defaults.dictionaryRepresentation().keys),
      activeCommunityIDs: activeCommunityIDs
    ) {
      defaults.removeObject(forKey: key)
    }
  }
}

/// Serialized community configuration shared with the notification service extension.
public struct BuzzPushSnapshot: Decodable, Sendable {
  public let communities: [BuzzPushCommunity]
}

/// Community configuration needed to query unread events for a push notification.
public struct BuzzPushCommunity: Decodable, Equatable, Sendable {
  public let id: String
  public let name: String
  public let relayUrl: String
  public let pubkey: String?

  public init(id: String, name: String, relayUrl: String, pubkey: String?) {
    self.id = id
    self.name = name
    self.relayUrl = relayUrl
    self.pubkey = pubkey
  }

  var relayURL: URL {
    URL(string: relayUrl) ?? URL(string: "http://127.0.0.1")!
  }
}
