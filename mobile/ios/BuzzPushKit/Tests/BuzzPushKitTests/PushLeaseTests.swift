import XCTest

@testable import BuzzPushKit

final class PushLeaseTests: XCTestCase {
  private let mine = String(repeating: "a", count: 64)
  private let other = String(repeating: "b", count: 64)

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
    let policy = PushResolutionPolicy(
      filter: PushLeaseFilter(kinds: [1059], pTags: [mine])
    )

    XCTAssertTrue(PushLeaseMatcher.matches(event: event, policy: policy))
  }

  func testIgnoreAndHellthreadSuppressionRejectCandidates() {
    let ignored = makeEvent(kind: 9, pubkey: other, tags: [["p", mine]])
    let ignorePolicy = PushResolutionPolicy(
      filter: PushLeaseFilter(kinds: [9], pTags: [mine]),
      ignore: [PushLeaseFilter(kinds: [9], authors: [other])]
    )
    XCTAssertFalse(
      PushLeaseMatcher.matches(event: ignored, policy: ignorePolicy)
    )

    let hellthread = makeEvent(
      kind: 9,
      tags: (0..<21).map { ["p", String(format: "%064x", $0)] }
    )
    let suppressed = PushResolutionPolicy(
      filter: PushLeaseFilter(kinds: [9], authors: [other]),
      suppress: PushLeaseSuppression(pTagsMax: 20)
    )
    XCTAssertFalse(PushLeaseMatcher.matches(event: hellthread, policy: suppressed))
  }

  func testDecodesSnapshotContractFromDartShape() throws {
    let json = """
      {"communities":[{"id":"origin","name":"Team","relayUrl":"https://relay.example.com","pubkey":"\(mine)","policies":[{"filter":{"kinds":[9],"#p":["\(mine)"]},"ignore":[{"kinds":[9],"authors":["\(mine)"]}],"suppress":{"p_tags_max":20}}]}]}
      """
    let snapshot = try JSONDecoder().decode(PushLeaseSnapshot.self, from: Data(json.utf8))

    XCTAssertEqual(snapshot.communities.count, 1)
    XCTAssertEqual(
      snapshot.communities[0].policies.count,
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
