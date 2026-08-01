import XCTest

@testable import BuzzPushKit

final class PushLeaseTests: XCTestCase {
  private let mine = String(repeating: "a", count: 64)
  private let other = String(repeating: "b", count: 64)

  func testDesiredAuthorityIsExplicitAndAcceptedAuthorityRequiresState() throws {
    let subscription = PushLeaseSubscription(
      filter: PushLeaseFilter(kinds: [9], pTags: [mine]),
      notificationClass: "default"
    )
    XCTAssertEqual(
      try PushLeaseSubscriptionState(
        authority: "desired",
        desired: [subscription]
      ).authoritativeSubscriptions(),
      [subscription]
    )
    XCTAssertThrowsError(
      try PushLeaseSubscriptionState(
        authority: "accepted",
        desired: [subscription]
      ).authoritativeSubscriptions()
    ) { error in
      XCTAssertEqual(error as? PushLeaseError, .acceptedAuthorityMissingSubscriptions)
    }
  }

  func testFilterBuildsQueryFromLeaseWithoutHardcodedKinds() {
    let filter = PushLeaseFilter(
      kinds: [7, 1059],
      authors: [other],
      pTags: [mine],
      hTags: ["channel"],
      eTags: [String(repeating: "c", count: 64)]
    )
    let query = filter.queryFilter(since: 1_000, limit: 10)

    XCTAssertEqual(query["kinds"] as? [Int], [7, 1059])
    XCTAssertEqual(query["authors"] as? [String], [other])
    XCTAssertEqual(query["#p"] as? [String], [mine])
    XCTAssertEqual(query["#h"] as? [String], ["channel"])
    XCTAssertEqual(query["since"] as? Int, 1_000)
  }

  func testPushEligibleKindAbsentFromOldConstantMatchesLease() {
    let event = makeEvent(kind: 1059, tags: [["p", mine]])
    let subscription = PushLeaseSubscription(
      filter: PushLeaseFilter(kinds: [1059], pTags: [mine]),
      notificationClass: "default"
    )

    XCTAssertTrue(PushLeaseMatcher.matches(event: event, subscription: subscription))
  }

  func testIgnoreAndHellthreadSuppressionRejectCandidates() {
    let ignored = makeEvent(kind: 9, pubkey: other, tags: [["p", mine]])
    let ignoreSubscription = PushLeaseSubscription(
      filter: PushLeaseFilter(kinds: [9], pTags: [mine]),
      notificationClass: "default",
      ignore: [PushLeaseFilter(kinds: [9], authors: [other])]
    )
    XCTAssertFalse(
      PushLeaseMatcher.matches(event: ignored, subscription: ignoreSubscription)
    )

    let hellthread = makeEvent(
      kind: 9,
      tags: (0..<21).map { ["p", String(format: "%064x", $0)] }
    )
    let suppressed = PushLeaseSubscription(
      filter: PushLeaseFilter(kinds: [9], authors: [other]),
      notificationClass: "default",
      suppress: PushLeaseSuppression(pTagsMax: 20)
    )
    XCTAssertFalse(PushLeaseMatcher.matches(event: hellthread, subscription: suppressed))
  }

  func testDecodesSnapshotContractFromDartShape() throws {
    let json = """
      {"communities":[{"id":"origin","name":"Team","relayUrl":"https://relay.example.com","pubkey":"\(mine)","pushSubscriptionState":{"authority":"desired","desired":[{"filter":{"kinds":[9],"#p":["\(mine)"]},"class":"default","ignore":[{"kinds":[9],"authors":["\(mine)"]}],"suppress":{"p_tags_max":20}}]}}]}
      """
    let snapshot = try JSONDecoder().decode(PushLeaseSnapshot.self, from: Data(json.utf8))

    XCTAssertEqual(snapshot.communities.count, 1)
    XCTAssertEqual(
      try snapshot.communities[0].pushSubscriptionState.authoritativeSubscriptions().count,
      1
    )
  }

  private func makeEvent(
    kind: Int,
    pubkey: String? = nil,
    tags: [[String]] = []
  ) -> VerifiedNostrEvent {
    VerifiedNostrEvent(
      id: String(repeating: "d", count: 64),
      pubkey: pubkey ?? other,
      createdAt: 1_000,
      kind: kind,
      tags: tags,
      content: "message",
      sig: String(repeating: "e", count: 128)
    )
  }
}
