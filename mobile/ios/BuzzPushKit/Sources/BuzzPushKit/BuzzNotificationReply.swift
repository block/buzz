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
  /// The account the notification was for, which the reply must be sent as,
  /// guarding against a community's credentials being replaced in place. nil
  /// for a notification an earlier build posted.
  public let recipientPubkey: String?

  public init(
    senderPubkey: String, threadRootID: String? = nil, messageCreatedAt: Int, inSolo: Bool? = nil,
    recipientPubkey: String? = nil
  ) {
    self.senderPubkey = senderPubkey
    self.threadRootID = threadRootID
    self.messageCreatedAt = messageCreatedAt
    self.inSolo = inSolo
    self.recipientPubkey = recipientPubkey
  }

  /// Kinds the app can answer with an ordinary channel or DM message.
  /// Forum posts (45001) and comments (45003) have their own reply rules.
  static let replyableKinds: Set<Int> = [9, 40002]

  static func make(
    for event: VerifiedNostrEvent, recipientPubkey: String, inSolo: Bool? = nil
  ) -> BuzzNotificationReplyContext? {
    guard replyableKinds.contains(event.kind) else { return nil }
    return BuzzNotificationReplyContext(
      senderPubkey: event.pubkey.lowercased(),
      threadRootID: nip10RootID(in: event.tags),
      messageCreatedAt: event.createdAt,
      inSolo: inSolo,
      recipientPubkey: recipientPubkey.lowercased()
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
    value["recipient_pubkey"] = recipientPubkey
    return value
  }

  /// Decodes a context without trusting other fields from the APNs payload.
  public static func decodeIfPresent(
    from userInfo: [AnyHashable: Any]
  ) -> BuzzNotificationReplyContext? {
    guard let raw = userInfo[userInfoKey] as? [String: Any],
      Set(raw.keys).isSubset(of: [
        "sender_pubkey", "thread_root_id", "created_at", "in_solo", "recipient_pubkey",
      ]),
      let sender = raw["sender_pubkey"] as? String, !sender.isEmpty,
      let createdAt = (raw["created_at"] as? String).flatMap(Int.init), createdAt > 0
    else { return nil }
    if let rawRoot = raw["thread_root_id"] {
      guard let root = rawRoot as? String, !root.isEmpty else { return nil }
    }
    if let rawRecipient = raw["recipient_pubkey"] {
      guard let recipient = rawRecipient as? String, !recipient.isEmpty else { return nil }
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
      messageCreatedAt: createdAt, inSolo: inSolo,
      recipientPubkey: raw["recipient_pubkey"] as? String)
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

  /// Whether a sent reply clears a delivered notification, matching the
  /// read marker the app writes: a reply in a thread reads that thread up to
  /// the notified message, and a top-level reply reads the whole conversation
  /// up to it, its threads included. Only notifications that offer Reply
  /// know where and when their message was.
  public func clears(threadIdentifier other: String, userInfo: [AnyHashable: Any]) -> Bool {
    guard !threadIdentifier.isEmpty, other == threadIdentifier,
      let notified = BuzzNotificationReplyContext.decodeIfPresent(from: userInfo),
      notified.messageCreatedAt <= context.messageCreatedAt
    else { return false }
    return context.threadRootID == nil || notified.threadRootID == context.threadRootID
  }

  /// The app arguments the journal keeps for this reply.
  public var journalArguments: [String: String] {
    var arguments = target.flutterArguments
    arguments["id"] = id
    arguments["text"] = text
    arguments["senderPubkey"] = context.senderPubkey
    arguments["replyRootId"] = context.threadRootID
    arguments["messageCreatedAt"] = String(context.messageCreatedAt)
    arguments["deadline"] = String(Int64(deadline.timeIntervalSince1970 * 1000))
    arguments["inSolo"] = context.inSolo == true ? "true" : nil
    arguments["recipientPubkey"] = context.recipientPubkey
    return arguments
  }

  /// Rebuilds a reply an earlier launch journaled, enough to tell the user
  /// about it; the notification it came from is gone.
  public init?(journalArguments arguments: [String: String]) {
    guard let id = arguments["id"], let text = arguments["text"],
      let eventID = arguments["eventId"], let communityID = arguments["communityId"],
      let channelID = arguments["channelId"], let sender = arguments["senderPubkey"],
      let createdAt = arguments["messageCreatedAt"].flatMap(Int.init)
    else { return nil }
    self.init(
      id: id, text: text,
      target: BuzzPushNavigationTarget(
        eventID: eventID, communityID: communityID, channelID: channelID,
        threadRootID: arguments["threadRootId"]),
      context: BuzzNotificationReplyContext(
        senderPubkey: sender, threadRootID: arguments["replyRootId"],
        messageCreatedAt: createdAt, inSolo: arguments["inSolo"] == "true" ? true : nil,
        recipientPubkey: arguments["recipientPubkey"]),
      threadIdentifier: "",
      deadline: arguments["deadline"].flatMap(Double.init).map {
        Date(timeIntervalSince1970: $0 / 1000)
      } ?? .distantFuture)
  }

  /// Where an expired reply's draft is kept: the subthread under the notified
  /// message, or a Solo chat itself (nil).
  var expiredDraftThreadRootID: String? {
    context.inSolo == true ? nil : target.eventID
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

  /// Whether a request is still waiting for its outcome.
  public func holds(id: String) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return entries.contains { $0.request.id == id }
  }

  /// Finishes a request, returning false when the id is unknown or already
  /// finished.
  @discardableResult
  public func complete(id: String, outcome: BuzzNotificationReplyOutcome) -> Bool {
    guard let entry = remove(id: id) else { return false }
    entry.finish(outcome)
    return true
  }

  /// Finishes a request at its deadline: as unknown when the app may already
  /// have published it, else as not sent, pointing at where the app will keep
  /// its draft.
  public func expire(id: String, mayHavePublished: Bool) {
    guard let entry = remove(id: id) else { return }
    entry.finish(
      mayHavePublished
        ? .unknown : .notSent(draftThreadRootID: entry.request.expiredDraftThreadRootID))
  }

  private func remove(id: String) -> Entry? {
    lock.lock()
    defer { lock.unlock() }
    return entries.firstIndex { $0.request.id == id }.map { entries.remove(at: $0) }
  }
}

/// Replies typed into notifications that the app hasn't reported on, kept on
/// disk from the moment they arrive. If the app is stopped before reporting
/// one, the next launch hands it over again marked `recovered`, which the app
/// never sends: it keeps the text as a draft, or, when the reply was marked
/// `publishing`, reports it unknown, since it may have gone out.
public final class BuzzNotificationReplyJournal: @unchecked Sendable {
  public static let defaultsKey = "buzz.notification_reply.pending"

  private let defaults: UserDefaults
  private let lock = NSLock()
  /// Ids handed to the app by this process, which aren't handed over twice.
  private var handedOver: Set<String> = []
  /// Ids an earlier process recorded.
  private let recovered: Set<String>

  public init(defaults: UserDefaults) {
    self.defaults = defaults
    recovered = Set(
      (defaults.array(forKey: Self.defaultsKey) as? [[String: String]] ?? []).compactMap {
        $0["id"]
      })
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
    return fresh.map { reply in
      guard recovered.contains(reply["id"] ?? "") else { return reply }
      return reply.merging(["recovered": "true"]) { _, new in new }
    }
  }

  /// Notes, before the app may publish a reply, that it may go out, so a
  /// relaunch never drafts or resends it.
  public func markPublishing(id: String) {
    lock.lock()
    defer { lock.unlock() }
    defaults.set(
      pending().map { $0["id"] == id ? $0.merging(["publishing": "true"]) { _, new in new } : $0 },
      forKey: Self.defaultsKey)
  }

  /// What is kept for a reply not yet settled.
  public func recorded(id: String) -> [String: String]? {
    lock.lock()
    defer { lock.unlock() }
    return pending().first { $0["id"] == id }
  }

  /// Whether the app noted that the reply may go out.
  public func isPublishing(id: String) -> Bool {
    recorded(id: id)?["publishing"] == "true"
  }

  /// Forgets a reply once the app has reported it sent, kept its draft, or
  /// found its community gone (after its "Reply not sent" notice is posted).
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
