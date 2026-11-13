import Foundation

/// What a notification needs so the app can send a reply typed into it.
///
/// Only a notification that shows a resolved channel or DM message carries
/// one. See docs/adr/0004-notification-reply.md.
public struct BuzzNotificationReplyContext: Codable, Equatable, Sendable {
  /// The notification category that offers the Reply action.
  public static let categoryIdentifier = "buzz.message"
  /// The text-input action a reply arrives through.
  public static let actionIdentifier = "buzz.reply"
  public static let userInfoKey = "buzz_notification_reply"

  /// Author of the notified message, mentioned by the reply.
  public let senderPubkey: String
  /// NIP-10 thread root of the notified message; nil when it is top-level.
  public let threadRootID: String?
  /// When the notified message was created; replying marks it read up to here.
  public let messageCreatedAt: Int
  /// Whether channel metadata showed a Solo chat, which takes no thread
  /// replies; nil when it was unknown.
  public let inSolo: Bool?

  public init(
    senderPubkey: String, threadRootID: String? = nil, messageCreatedAt: Int, inSolo: Bool? = nil
  ) {
    self.senderPubkey = senderPubkey
    self.threadRootID = threadRootID
    self.messageCreatedAt = messageCreatedAt
    self.inSolo = inSolo
  }

  /// Kinds the app can answer with an ordinary channel or DM message.
  /// Forum posts (45001) and comments (45003) have their own reply rules.
  static let replyableKinds: Set<Int> = [9, 40002]

  static func make(
    for event: VerifiedNostrEvent, inSolo: Bool? = nil
  ) -> BuzzNotificationReplyContext? {
    guard replyableKinds.contains(event.kind) else { return nil }
    return BuzzNotificationReplyContext(
      senderPubkey: event.pubkey.lowercased(),
      threadRootID: nip10RootID(in: event.tags),
      messageCreatedAt: event.createdAt,
      inSolo: inSolo
    )
  }

  /// The thread root, matching the app's thread parsing: a message is a reply
  /// only when it has a valid `reply` marker; its root is the `root` marker,
  /// or the parent itself for a direct reply. The last valid marker of each
  /// kind wins.
  static func nip10RootID(in tags: [[String]]) -> String? {
    func lastMarked(_ marker: String) -> String? {
      tags.last(where: { tag in
        tag.count >= 4 && tag[0] == "e" && tag[3] == marker && isEventID(tag[1])
      })?[1].lowercased()
    }
    guard let parent = lastMarked("reply") else { return nil }
    return lastMarked("root") ?? parent
  }

  private static func isEventID(_ value: String) -> Bool {
    let bytes = value.utf8
    return bytes.count == 64
      && bytes.allSatisfy {
        (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0)
      }
  }

  public var userInfoValue: [String: String] {
    var value = ["sender_pubkey": senderPubkey, "created_at": String(messageCreatedAt)]
    value["thread_root_id"] = threadRootID
    value["in_solo"] = inSolo.map { $0 ? "true" : "false" }
    return value
  }

  /// Decodes a context without trusting other fields from the APNs payload.
  public static func decodeIfPresent(
    from userInfo: [AnyHashable: Any]
  ) -> BuzzNotificationReplyContext? {
    guard let raw = userInfo[userInfoKey] as? [String: Any],
      Set(raw.keys).isSubset(of: ["sender_pubkey", "thread_root_id", "created_at", "in_solo"]),
      let sender = raw["sender_pubkey"] as? String, !sender.isEmpty,
      let createdAt = (raw["created_at"] as? String).flatMap(Int.init), createdAt > 0
    else { return nil }
    if let rawRoot = raw["thread_root_id"] {
      guard let root = rawRoot as? String, !root.isEmpty else { return nil }
    }
    var inSolo: Bool?
    if let rawSolo = raw["in_solo"] {
      switch rawSolo as? String {
      case "true": inSolo = true
      case "false": inSolo = false
      default: return nil
      }
    }
    return BuzzNotificationReplyContext(
      senderPubkey: sender, threadRootID: raw["thread_root_id"] as? String,
      messageCreatedAt: createdAt, inSolo: inSolo)
  }
}

/// A reply typed into a notification, waiting for the app to send it.
public struct BuzzNotificationReplyRequest: Equatable, Sendable {
  public let id: String
  public let text: String
  public let target: BuzzPushNavigationTarget
  public let context: BuzzNotificationReplyContext
  /// The notification's thread, whose delivered notifications a sent reply clears.
  public let threadIdentifier: String
  /// When the reply counts as not sent if the app hasn't reported otherwise.
  public let deadline: Date

  public init(
    id: String,
    text: String,
    target: BuzzPushNavigationTarget,
    context: BuzzNotificationReplyContext,
    threadIdentifier: String,
    deadline: Date = .distantFuture
  ) {
    self.id = id
    self.text = text
    self.target = target
    self.context = context
    self.threadIdentifier = threadIdentifier
    self.deadline = deadline
  }

  /// Builds a request from a Reply action, or nil when the notification
  /// carries no reply context or the typed text is blank.
  public static func make(
    id: String,
    text: String,
    userInfo: [AnyHashable: Any],
    threadIdentifier: String,
    deadline: Date = .distantFuture
  ) -> BuzzNotificationReplyRequest? {
    guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
      let target = BuzzPushNavigationTarget.decodeIfPresent(from: userInfo),
      let context = BuzzNotificationReplyContext.decodeIfPresent(from: userInfo)
    else { return nil }
    return BuzzNotificationReplyRequest(
      id: id, text: text, target: target, context: context,
      threadIdentifier: threadIdentifier, deadline: deadline)
  }
}

/// How a notification reply ended.
public enum BuzzNotificationReplyOutcome: Equatable, Sendable {
  case sent
  /// Never published. The app keeps its text as a draft in the thread named
  /// here, or in the notified conversation when nil.
  case notSent(draftThreadRootID: String?)
  /// Published without an acknowledgement, so it may have gone out.
  case unknown
}

/// Holds replies from notification actions until the app finishes each one.
///
/// Every request finishes exactly once: when the app reports the outcome, or
/// as not sent when it expires first.
public final class BuzzNotificationReplyBuffer: @unchecked Sendable {
  private struct Entry {
    let request: BuzzNotificationReplyRequest
    let finish: (BuzzNotificationReplyOutcome) -> Void
    var taken: Bool
  }

  private let lock = NSLock()
  private var entries: [Entry] = []

  public init() {}

  public func add(
    _ request: BuzzNotificationReplyRequest,
    finish: @escaping (BuzzNotificationReplyOutcome) -> Void
  ) {
    lock.lock()
    entries.append(Entry(request: request, finish: finish, taken: false))
    lock.unlock()
  }

  /// Requests the app hasn't taken yet, oldest first.
  public func take() -> [BuzzNotificationReplyRequest] {
    lock.lock()
    defer { lock.unlock() }
    var taken: [BuzzNotificationReplyRequest] = []
    for index in entries.indices where !entries[index].taken {
      entries[index].taken = true
      taken.append(entries[index].request)
    }
    return taken
  }

  /// Finishes a request. Unknown or already finished ids are ignored.
  public func complete(id: String, outcome: BuzzNotificationReplyOutcome) {
    remove(id: id)?.finish(outcome)
  }

  /// Finishes a request as not sent at its deadline.
  public func expire(id: String) {
    remove(id: id)?.finish(.notSent(draftThreadRootID: nil))
  }

  private func remove(id: String) -> Entry? {
    lock.lock()
    defer { lock.unlock() }
    return entries.firstIndex { $0.request.id == id }.map { entries.remove(at: $0) }
  }
}

/// Replies typed into notifications that the app hasn't reported on, kept on
/// disk from the moment they arrive. If the app is stopped before reporting
/// one, the next launch hands it over again, and its deadline having passed,
/// the app keeps its text as a draft instead of sending it.
public final class BuzzNotificationReplyJournal: @unchecked Sendable {
  public static let defaultsKey = "buzz.notification_reply.pending"

  private let defaults: UserDefaults
  private let lock = NSLock()
  /// Ids handed to the app by this process, which aren't handed over twice.
  private var handedOver: Set<String> = []

  public init(defaults: UserDefaults) {
    self.defaults = defaults
  }

  /// Keeps a reply's app arguments, which carry its `id`.
  public func record(_ arguments: [String: String]) {
    lock.lock()
    defer { lock.unlock() }
    defaults.set(pending() + [arguments], forKey: Self.defaultsKey)
  }

  /// Replies not yet handed over by this process, oldest first.
  public func take() -> [[String: String]] {
    lock.lock()
    defer { lock.unlock() }
    let fresh = pending().filter { !handedOver.contains($0["id"] ?? "") }
    handedOver.formUnion(fresh.compactMap { $0["id"] })
    return fresh
  }

  /// Forgets a reply once the app has reported it sent, or kept its draft.
  public func settle(id: String) {
    lock.lock()
    defer { lock.unlock() }
    defaults.set(pending().filter { $0["id"] != id }, forKey: Self.defaultsKey)
  }

  private func pending() -> [[String: String]] {
    defaults.array(forKey: Self.defaultsKey) as? [[String: String]] ?? []
  }
}

/// The community selected in the app, shared with the notification extension.
///
/// Replying can't switch communities and the app can reach only the selected
/// one, so only its notifications offer Reply.
public enum BuzzSelectedCommunity {
  static let defaultsKey = "buzz.selected_community_id"

  public static func id(in defaults: UserDefaults?) -> String? {
    guard let id = defaults?.string(forKey: defaultsKey), !id.isEmpty else { return nil }
    return id
  }

  public static func set(_ id: String?, in defaults: UserDefaults?) {
    if let id, !id.isEmpty {
      defaults?.set(id, forKey: defaultsKey)
    } else {
      defaults?.removeObject(forKey: defaultsKey)
    }
  }
}
