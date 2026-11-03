import Foundation

/// A stable destination attached by the notification service extension after
/// it resolves and verifies the event that produced a push wake.
public struct BuzzPushNavigationTarget: Codable, Equatable, Sendable {
  public static let userInfoKey = "buzz_push_navigation"

  public let eventID: String
  public let communityID: String
  public let channelID: String

  /// Direct parent whose thread contains the target reply, matching Inbox.
  public let threadRootID: String?

  public init(
    eventID: String, communityID: String, channelID: String, threadRootID: String? = nil
  ) {
    self.eventID = eventID
    self.communityID = communityID
    self.channelID = channelID
    self.threadRootID = threadRootID
  }

  public var userInfoValue: [String: String] {
    var value = [
      "event_id": eventID,
      "community_id": communityID,
      "channel_id": channelID,
    ]
    value["thread_root_id"] = threadRootID
    return value
  }

  static func threadRootID(in tags: [[String]]) -> String? {
    // Inbox opens broadcast replies in the channel and other replies in their
    // direct parent's thread, even when the NIP-10 root names an outer thread.
    if tags.contains(where: { $0.count >= 2 && $0[0] == "broadcast" && $0[1] == "1" }) {
      return nil
    }
    // Match relay ingest: the last valid 64-character ASCII-hex reply ID wins.
    // A later malformed marker must not hide an earlier valid parent.
    return tags.last(where: { tag in
      guard tag.count >= 4, tag[0] == "e", tag[3] == "reply" else { return false }
      let bytes = tag[1].utf8
      return bytes.count == 64 && bytes.allSatisfy {
        (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0)
      }
    })?[1].lowercased()
  }

  /// Decodes a target without trusting other fields from the APNs payload.
  public static func decodeIfPresent(
    from userInfo: [AnyHashable: Any]
  ) -> BuzzPushNavigationTarget? {
    guard let raw = userInfo[userInfoKey] as? [String: Any],
      Set(raw.keys).isSubset(of: ["event_id", "community_id", "channel_id", "thread_root_id"]),
      let eventID = raw["event_id"] as? String,
      let communityID = raw["community_id"] as? String,
      let channelID = raw["channel_id"] as? String,
      !eventID.isEmpty,
      !communityID.isEmpty,
      !channelID.isEmpty
    else {
      return nil
    }
    if let rawThread = raw["thread_root_id"] {
      guard let thread = rawThread as? String, !thread.isEmpty else { return nil }
    }
    return BuzzPushNavigationTarget(
      eventID: eventID,
      communityID: communityID,
      channelID: channelID,
      threadRootID: raw["thread_root_id"] as? String
    )
  }

}

/// Thread-safe one-item buffer spanning notification delivery and Flutter
/// engine startup during a cold notification launch.
public final class BuzzPushNavigationBuffer: @unchecked Sendable {
  private let lock = NSLock()
  private var target: BuzzPushNavigationTarget?

  public init() {}

  public func record(_ target: BuzzPushNavigationTarget) {
    lock.lock()
    self.target = target
    lock.unlock()
  }

  public func peek() -> BuzzPushNavigationTarget? {
    lock.lock()
    defer { lock.unlock() }
    return target
  }

  public func take() -> BuzzPushNavigationTarget? {
    lock.lock()
    defer { lock.unlock() }
    let current = target
    target = nil
    return current
  }

  public func remove(ifMatching expected: BuzzPushNavigationTarget) {
    lock.lock()
    defer { lock.unlock() }
    if target == expected {
      target = nil
    }
  }
}
