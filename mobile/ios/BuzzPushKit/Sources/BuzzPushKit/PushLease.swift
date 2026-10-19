import Foundation

public struct PushLeaseSnapshot: Codable, Equatable, Sendable {
  public let communities: [PushLeaseCommunity]

  public init(communities: [PushLeaseCommunity]) {
    self.communities = communities
  }
}

public struct PushLeaseCommunity: Codable, Equatable, Sendable {
  public let id: String
  public let name: String
  public let relayUrl: String
  /// Relay NIP-11 `self` key used to verify NIP-29 channel metadata.
  public let relayMetadataPubkey: String?
  public let pubkey: String?
  /// Identity of this local sign-in, renewed after account removal.
  public var notificationSessionID: String?
  public let policies: [PushResolutionPolicy]

  public init(
    id: String,
    name: String,
    relayUrl: String,
    relayMetadataPubkey: String? = nil,
    pubkey: String?,
    policies: [PushResolutionPolicy],
    notificationSessionID: String? = nil
  ) {
    self.id = id
    self.name = name
    self.relayUrl = relayUrl
    self.relayMetadataPubkey = relayMetadataPubkey
    self.pubkey = pubkey
    self.policies = policies
    self.notificationSessionID = notificationSessionID
  }
}

public struct PushResolutionPolicy: Codable, Equatable, Sendable {
  public let filter: PushLeaseFilter
  public let ignore: [PushLeaseFilter]
  public let suppress: PushLeaseSuppression?

  public init(
    filter: PushLeaseFilter,
    ignore: [PushLeaseFilter] = [],
    suppress: PushLeaseSuppression? = nil
  ) {
    self.filter = filter
    self.ignore = ignore
    self.suppress = suppress
  }
}

public struct PushLeaseSuppression: Codable, Equatable, Sendable {
  public let pTagsMax: Int

  enum CodingKeys: String, CodingKey {
    case pTagsMax = "p_tags_max"
  }

  public init(pTagsMax: Int) {
    self.pTagsMax = pTagsMax
  }
}

public struct PushLeaseFilter: Codable, Equatable, Sendable {
  public let kinds: [Int]
  public let authors: [String]?
  public let pTags: [String]?
  public let hTags: [String]?
  public let eTags: [String]?
  public let authoredThreadReplies: Bool?
  public let since: Int?

  enum CodingKeys: String, CodingKey {
    case kinds
    case since
    case authors
    case pTags = "#p"
    case hTags = "#h"
    case eTags = "#e"
    case authoredThreadReplies = "authored_thread_replies"
  }

  public init(
    kinds: [Int],
    authors: [String]? = nil,
    pTags: [String]? = nil,
    hTags: [String]? = nil,
    eTags: [String]? = nil,
    authoredThreadReplies: Bool? = nil,
    since: Int? = nil
  ) {
    self.kinds = kinds
    self.authors = authors
    self.pTags = pTags
    self.hTags = hTags
    self.eTags = eTags
    self.authoredThreadReplies = authoredThreadReplies
    self.since = since
  }

  public func queryFilter(since: Int?, limit: Int) -> [String: Any] {
    var filter: [String: Any] = ["kinds": kinds, "limit": limit]
    if let authors { filter["authors"] = authors }
    if let pTags { filter["#p"] = pTags }
    if let hTags { filter["#h"] = hTags }
    if let eTags { filter["#e"] = eTags }
    if authoredThreadReplies == true { filter["authored_thread_replies"] = true }
    if let since = [self.since, since].compactMap({ $0 }).max() { filter["since"] = since }
    return filter
  }

  public func matches(_ event: VerifiedNostrEvent, authoredThreadQuery: Bool = false) -> Bool {
    if authoredThreadReplies == true && !authoredThreadQuery { return false }
    if let since, event.createdAt < since { return false }
    guard kinds.contains(event.kind) else { return false }
    if let authors, !authors.contains(event.pubkey.lowercased()) { return false }
    if let pTags, !event.hasAnyTag(named: "p", values: pTags) { return false }
    if let hTags, !event.hasAnyTag(named: "h", values: hTags) { return false }
    if let eTags, !event.hasAnyTag(named: "e", values: eTags) { return false }
    return true
  }
}

public enum PushLeaseMatcher {
  public static func matches(
    event: VerifiedNostrEvent,
    policy: PushResolutionPolicy,
    authoredThreadQuery: Bool = false
  ) -> Bool {
    guard policy.filter.matches(event, authoredThreadQuery: authoredThreadQuery) else { return false }
    if policy.ignore.contains(where: { $0.matches(event) }) { return false }
    if let maximum = policy.suppress?.pTagsMax,
      event.tagCount(named: "p") > maximum
    {
      return false
    }
    return true
  }
}

extension VerifiedNostrEvent {
  public func tagCount(named name: String) -> Int {
    tags.reduce(into: 0) { count, tag in
      if tag.count >= 2 && tag[0] == name { count += 1 }
    }
  }

  public func hasAnyTag(named name: String, values: [String]) -> Bool {
    let expected = Set(values.map { $0.lowercased() })
    return tags.contains { tag in
      tag.count >= 2 && tag[0] == name && expected.contains(tag[1].lowercased())
    }
  }
}
