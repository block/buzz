import Foundation
import Testing

@testable import BuzzPushKit

private let root = String(repeating: "a", count: 64)
private let parent = String(repeating: "b", count: 64)
private let sender = String(repeating: "C", count: 64)

private func event(kind: Int, tags: [[String]] = []) -> VerifiedNostrEvent {
  VerifiedNostrEvent(
    id: String(repeating: "d", count: 64),
    pubkey: sender,
    createdAt: 1_000,
    kind: kind,
    tags: tags,
    content: "message",
    sig: String(repeating: "e", count: 128)
  )
}

@Test(arguments: [
  // Top-level message: no thread root.
  ([[String]](), nil as String?),
  // Direct reply to a thread head: the lone reply marker names the root.
  ([["e", root, "", "reply"]], root),
  // Nested reply: the root marker wins over the direct parent.
  ([["e", root, "", "root"], ["e", parent, "", "reply"]], root),
  // A malformed root marker falls back to the valid reply marker.
  ([["e", "not-hex", "", "root"], ["e", parent, "", "reply"]], parent),
  // Uppercase IDs are normalized.
  ([["e", root.uppercased(), "", "root"], ["e", parent, "", "reply"]], root),
  // A root marker without a valid reply marker is not a reply, as in the app.
  ([["e", root, "", "root"]], nil),
])
func `Reply context carries the sender and NIP-10 thread root`(
  tags: [[String]], expectedRoot: String?
) throws {
  let context = try #require(BuzzNotificationReplyContext.make(for: event(kind: 9, tags: tags)))
  #expect(context.senderPubkey == sender.lowercased())
  #expect(context.threadRootID == expectedRoot)
  #expect(context.messageCreatedAt == 1_000)
  #expect(
    BuzzNotificationReplyContext.decodeIfPresent(
      from: [BuzzNotificationReplyContext.userInfoKey: context.userInfoValue]) == context)
}

@Test(arguments: [45001, 45003, 7])
func `Forum posts and other kinds offer no reply`(kind: Int) {
  #expect(BuzzNotificationReplyContext.make(for: event(kind: kind)) == nil)
}

@Test func `Reply context rejects unexpected or malformed fields`() {
  let key = BuzzNotificationReplyContext.userInfoKey
  #expect(
    BuzzNotificationReplyContext.decodeIfPresent(
      from: [key: ["sender_pubkey": "", "created_at": "1"]]) == nil)
  #expect(BuzzNotificationReplyContext.decodeIfPresent(from: [key: ["sender_pubkey": "pk"]]) == nil)
  #expect(
    BuzzNotificationReplyContext.decodeIfPresent(
      from: [key: ["sender_pubkey": "pk", "created_at": "soon"]]) == nil)
  #expect(
    BuzzNotificationReplyContext.decodeIfPresent(
      from: [key: ["sender_pubkey": "pk", "created_at": "1", "extra": "x"]]) == nil)
  #expect(
    BuzzNotificationReplyContext.decodeIfPresent(
      from: [key: ["sender_pubkey": "pk", "created_at": "1", "thread_root_id": ""]]) == nil)
}

@Test func `Resolution offers reply only for a channel message`() {
  let community = PushLeaseCommunity(
    id: "community", name: "Community", relayUrl: "https://relay.example.org",
    pubkey: String(repeating: "f", count: 64), policies: [])
  let inChannel = BuzzPushNotificationResolver.makeResolution(
    event: event(kind: 9, tags: [["h", "channel"]]), community: community, profile: nil,
    channel: nil)
  #expect(inChannel?.replyContext?.senderPubkey == sender.lowercased())
  let noChannel = BuzzPushNotificationResolver.makeResolution(
    event: event(kind: 9), community: community, profile: nil, channel: nil)
  #expect(noChannel != nil)
  #expect(noChannel?.replyContext == nil)
}

@Test func `Reply request needs text, a target and a reply context`() {
  let target = BuzzPushNavigationTarget(
    eventID: parent, communityID: "community", channelID: "channel")
  let context = BuzzNotificationReplyContext(senderPubkey: "pk", threadRootID: root, messageCreatedAt: 1)
  let userInfo: [AnyHashable: Any] = [
    BuzzPushNavigationTarget.userInfoKey: target.userInfoValue,
    BuzzNotificationReplyContext.userInfoKey: context.userInfoValue,
  ]
  let request = BuzzNotificationReplyRequest.make(
    id: "1", text: "hi", userInfo: userInfo, threadIdentifier: "thread")
  #expect(request?.target == target)
  #expect(request?.context == context)
  #expect(
    BuzzNotificationReplyRequest.make(
      id: "1", text: " \n", userInfo: userInfo, threadIdentifier: "thread") == nil)
  #expect(
    BuzzNotificationReplyRequest.make(
      id: "1", text: "hi",
      userInfo: [BuzzPushNavigationTarget.userInfoKey: target.userInfoValue],
      threadIdentifier: "thread") == nil)
}

@Test func `Each buffered reply is taken once and finishes once`() {
  let buffer = BuzzNotificationReplyBuffer()
  let target = BuzzPushNavigationTarget(eventID: parent, communityID: "c", channelID: "ch")
  let context = BuzzNotificationReplyContext(senderPubkey: "pk", messageCreatedAt: 1)
  func request(_ id: String) -> BuzzNotificationReplyRequest {
    BuzzNotificationReplyRequest(
      id: id, text: id, target: target, context: context, threadIdentifier: "t")
  }
  var outcomes: [String: [BuzzNotificationReplyOutcome]] = [:]
  for id in ["1", "2", "3", "4"] {
    buffer.add(request(id)) { outcomes[id, default: []].append($0) }
  }

  #expect(buffer.take().map(\.id) == ["1", "2", "3", "4"])
  #expect(buffer.take().isEmpty)
  // A Solo chat keeps an expired reply's draft in the chat itself.
  buffer.add(
    BuzzNotificationReplyRequest(
      id: "5", text: "5", target: target,
      context: BuzzNotificationReplyContext(senderPubkey: "pk", messageCreatedAt: 1, inSolo: true),
      threadIdentifier: "t")
  ) { outcomes["5", default: []].append($0) }

  #expect(buffer.complete(id: "1", outcome: .sent))
  // A deadline after the app reported the outcome changes nothing.
  buffer.expire(id: "1", mayHavePublished: false)
  #expect(!buffer.complete(id: "1", outcome: .sent))
  buffer.complete(id: "2", outcome: .notSent(draftThreadRootID: "head"))
  buffer.complete(id: "3", outcome: .unknown)
  buffer.expire(id: "4", mayHavePublished: false)
  buffer.expire(id: "5", mayHavePublished: false)
  // One the app may already have published can't be called not sent.
  buffer.add(request("6")) { outcomes["6", default: []].append($0) }
  buffer.expire(id: "6", mayHavePublished: true)
  #expect(!buffer.complete(id: "unknown", outcome: .sent))

  #expect(
    outcomes == [
      "1": [.sent], "2": [.notSent(draftThreadRootID: "head")], "3": [.unknown],
      // Expired, it opens the subthread under the notified message, where the
      // app keeps its draft.
      "4": [.notSent(draftThreadRootID: parent)], "5": [.notSent(draftThreadRootID: nil)],
      "6": [.unknown],
    ])
}

@Test func `A reply stays journaled until reported, across a relaunch`() throws {
  let suite = "reply-journal-\(UUID().uuidString)"
  let defaults = try #require(UserDefaults(suiteName: suite))
  defer { defaults.removePersistentDomain(forName: suite) }
  let journal = BuzzNotificationReplyJournal(defaults: defaults)
  journal.record(["id": "1", "text": "one"])
  journal.record(["id": "2", "text": "two"])

  #expect(journal.take().map { $0["id"] } == ["1", "2"])
  // The app is already working on these.
  #expect(journal.take().isEmpty)
  journal.settle(id: "1")
  journal.record(["id": "3", "text": "three"])
  #expect(journal.take().map { $0["id"] } == ["3"])

  // The app got as far as maybe publishing 3, so expiry reports it unknown.
  #expect(!journal.isPublishing(id: "3"))
  journal.markPublishing(id: "3")
  #expect(journal.isPublishing(id: "3"))
  #expect(!journal.isPublishing(id: "2"))

  // The app was stopped before reporting 2 and 3, so a relaunch hands them
  // over again, marked so the app never sends them; one recorded after the
  // relaunch is fresh.
  let relaunched = BuzzNotificationReplyJournal(defaults: defaults)
  relaunched.record(["id": "4", "text": "four"])
  #expect(
    relaunched.take() == [
      ["id": "2", "text": "two", "recovered": "true"],
      ["id": "3", "text": "three", "publishing": "true", "recovered": "true"],
      ["id": "4", "text": "four"],
    ])
  // The relaunch can still say that 3 may not have sent, then settle it.
  #expect(relaunched.recorded(id: "3") == ["id": "3", "text": "three", "publishing": "true"])
  #expect(relaunched.isPublishing(id: "3"))
  for id in ["2", "3", "4"] { relaunched.settle(id: id) }
  #expect(relaunched.recorded(id: "3") == nil)
  #expect(BuzzNotificationReplyJournal(defaults: defaults).take().isEmpty)
}

@Test func `A journaled reply rebuilds the request it came from`() {
  let request = BuzzNotificationReplyRequest(
    id: "1", text: "hi",
    target: BuzzPushNavigationTarget(
      eventID: parent, communityID: "c", channelID: "ch", threadRootID: "root"),
    context: BuzzNotificationReplyContext(
      senderPubkey: "pk", threadRootID: "root", messageCreatedAt: 7, inSolo: true),
    threadIdentifier: "t",
    deadline: Date(timeIntervalSince1970: 1_700_000_000.5))
  let rebuilt = BuzzNotificationReplyRequest(
    journalArguments: request.journalArguments.merging(["recovered": "true"]) { _, new in new })
  // The notification it came from, and its thread, are gone.
  #expect(
    rebuilt
      == BuzzNotificationReplyRequest(
        id: request.id, text: request.text, target: request.target, context: request.context,
        threadIdentifier: "", deadline: request.deadline))
  #expect(BuzzNotificationReplyRequest(journalArguments: ["id": "1", "text": "hi"]) == nil)
}

@Test func `Reply context carries whether the chat is Solo`() throws {
  for inSolo in [true, false, nil] as [Bool?] {
    let context = BuzzNotificationReplyContext(
      senderPubkey: "pk", messageCreatedAt: 1, inSolo: inSolo)
    #expect(
      BuzzNotificationReplyContext.decodeIfPresent(
        from: [BuzzNotificationReplyContext.userInfoKey: context.userInfoValue]) == context)
  }
  #expect(
    BuzzNotificationReplyContext.decodeIfPresent(
      from: [
        BuzzNotificationReplyContext.userInfoKey: [
          "sender_pubkey": "pk", "created_at": "1", "in_solo": "yes",
        ]
      ]) == nil)
}

@Test func `Selected community round-trips and clears`() throws {
  let suite = "BuzzNotificationReplyTests.\(UUID().uuidString)"
  let defaults = try #require(UserDefaults(suiteName: suite))
  defer { defaults.removePersistentDomain(forName: suite) }
  #expect(BuzzSelectedCommunity.id(in: defaults) == nil)
  BuzzSelectedCommunity.set("community", in: defaults)
  #expect(BuzzSelectedCommunity.id(in: defaults) == "community")
  BuzzSelectedCommunity.set(nil, in: defaults)
  #expect(BuzzSelectedCommunity.id(in: defaults) == nil)
}

@Test func `A sent reply clears the notifications its read marker covers`() {
  let target = BuzzPushNavigationTarget(eventID: parent, communityID: "c", channelID: "ch")
  func request(root: String?) -> BuzzNotificationReplyRequest {
    BuzzNotificationReplyRequest(
      id: "1", text: "hi", target: target,
      context: BuzzNotificationReplyContext(
        senderPubkey: "pk", threadRootID: root, messageCreatedAt: 10),
      threadIdentifier: "conversation")
  }
  func notification(root: String?, at createdAt: Int) -> [AnyHashable: Any] {
    [
      BuzzNotificationReplyContext.userInfoKey: BuzzNotificationReplyContext(
        senderPubkey: "pk", threadRootID: root, messageCreatedAt: createdAt
      ).userInfoValue
    ]
  }
  let rows: [(request: String?, notified: String?, createdAt: Int, cleared: Bool)] = [
    // A top-level reply reads the whole conversation, threads included.
    (nil, nil, 5, true),
    (nil, "thread", 5, true),
    (nil, "thread", 11, false),
    // A reply in a thread reads only that thread.
    ("thread", "thread", 5, true),
    ("thread", nil, 5, false),
    ("thread", "other", 5, false),
  ]
  for row in rows {
    #expect(
      request(root: row.request).clears(
        threadIdentifier: "conversation",
        userInfo: notification(root: row.notified, at: row.createdAt)) == row.cleared)
  }
  #expect(
    !request(root: nil).clears(
      threadIdentifier: "elsewhere", userInfo: notification(root: nil, at: 5)))
  #expect(!request(root: nil).clears(threadIdentifier: "conversation", userInfo: [:]))
}
