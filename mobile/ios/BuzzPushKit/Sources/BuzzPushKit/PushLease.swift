import Foundation

public enum PushLeaseError: Error, Equatable {
  case unsupportedAuthority(String)
  case acceptedAuthorityMissingSubscriptions
  case emptySubscriptions
}

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
  public let pubkey: String?
  public let pushSubscriptionState: PushLeaseSubscriptionState

  public init(
    id: String,
    name: String,
    relayUrl: String,
    pubkey: String?,
    pushSubscriptionState: PushLeaseSubscriptionState
  ) {
    self.id = id
    self.name = name
    self.relayUrl = relayUrl
    self.pubkey = pubkey
    self.pushSubscriptionState = pushSubscriptionState
  }
}

public struct PushLeaseSubscriptionState: Codable, Equatable, Sendable {
  public enum Authority: String, Codable, Sendable {
    case desired
    case accepted
  }

  public let authority: String
  public let desired: [PushLeaseSubscription]
  public let accepted: [PushLeaseSubscription]?

  public init(
    authority: String,
    desired: [PushLeaseSubscription],
    accepted: [PushLeaseSubscription]? = nil
  ) {
    self.authority = authority
    self.desired = desired
    self.accepted = accepted
  }

  /// Workstream A has no lease publisher, so `desired` is the only valid
  /// authority today. A later publisher must persist the observed accepted
  /// lease and switch this field explicitly. This keeps relay rejection,
  /// clamping, or expiry visible rather than assuming acceptance.
  public func authoritativeSubscriptions() throws -> [PushLeaseSubscription] {
    let subscriptions: [PushLeaseSubscription]
    switch authority {
    case Authority.desired.rawValue:
      subscriptions = desired
    case Authority.accepted.rawValue:
      guard let accepted else {
        throw PushLeaseError.acceptedAuthorityMissingSubscriptions
      }
      subscriptions = accepted
    default:
      throw PushLeaseError.unsupportedAuthority(authority)
    }
    guard !subscriptions.isEmpty else {
      throw PushLeaseError.emptySubscriptions
    }
    return subscriptions
  }
}

public struct PushLeaseSubscription: Codable, Equatable, Sendable {
  public let filter: PushLeaseFilter
  public let notificationClass: String
  public let ignore: [PushLeaseFilter]
  public let suppress: PushLeaseSuppression?

  enum CodingKeys: String, CodingKey {
    case filter
    case notificationClass = "class"
    case ignore
    case suppress
  }

  public init(
    filter: PushLeaseFilter,
    notificationClass: String,
    ignore: [PushLeaseFilter] = [],
    suppress: PushLeaseSuppression? = nil
  ) {
    self.filter = filter
    self.notificationClass = notificationClass
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

  enum CodingKeys: String, CodingKey {
    case kinds
    case authors
    case pTags = "#p"
    case hTags = "#h"
    case eTags = "#e"
  }

  public init(
    kinds: [Int],
    authors: [String]? = nil,
    pTags: [String]? = nil,
    hTags: [String]? = nil,
    eTags: [String]? = nil
  ) {
    self.kinds = kinds
    self.authors = authors
    self.pTags = pTags
    self.hTags = hTags
    self.eTags = eTags
  }

  public func queryFilter(since: Int?, limit: Int) -> [String: Any] {
    var filter: [String: Any] = ["kinds": kinds, "limit": limit]
    if let authors { filter["authors"] = authors }
    if let pTags { filter["#p"] = pTags }
    if let hTags { filter["#h"] = hTags }
    if let eTags { filter["#e"] = eTags }
    if let since { filter["since"] = since }
    return filter
  }

  public func matches(_ event: VerifiedNostrEvent) -> Bool {
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
    subscription: PushLeaseSubscription
  ) -> Bool {
    guard subscription.filter.matches(event) else { return false }
    if subscription.ignore.contains(where: { $0.matches(event) }) { return false }
    if let maximum = subscription.suppress?.pTagsMax,
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
