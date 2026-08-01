import XCTest

@testable import BuzzPushKit

final class PushCatchUpTests: XCTestCase {
  private let mine = String(repeating: "a", count: 64)
  private let other = String(repeating: "b", count: 64)

  func testSequentialWakesSelectSameSecondSiblingBeforeConsumedDuplicateFallback() {
    let subscription = self.subscription()
    let id0 = String(format: "%064x", 0)
    let id1 = String(format: "%064x", 1)
    let events = [event(id: id0), event(id: id1)]
    var state = PushConsumptionState()

    let first = PushCatchUp.orderedSelections(
      events: events,
      origin: "origin",
      subscriptions: [subscription],
      consumptionState: state,
      verify: { _ in true }
    )
    XCTAssertEqual(first.first?.event.id, id0)
    XCTAssertEqual(first.first?.wasPreviouslyConsumed, false)
    state.consume(PushEventPosition(createdAt: 1_000, id: id0), for: "origin")

    let second = PushCatchUp.orderedSelections(
      events: events,
      origin: "origin",
      subscriptions: [subscription],
      consumptionState: state,
      verify: { _ in true }
    )
    XCTAssertEqual(second.map(\.event.id), [id1, id0])
    XCTAssertEqual(second.map(\.wasPreviouslyConsumed), [false, true])
  }

  func testColdStartSelectsOldestAcrossDistinctSeconds() {
    let subscription = self.subscription()
    let older = event(id: String(format: "%064x", 1))
    let newer = event(id: String(format: "%064x", 2), createdAt: 1_001)
    let state = PushConsumptionState()

    let first = PushCatchUp.orderedSelections(
      events: [newer, older],
      origin: "origin",
      subscriptions: [subscription],
      consumptionState: state,
      verify: { _ in true }
    ).first

    XCTAssertEqual(first?.event.id, older.id)
  }

  func testActiveSecondSiblingWinsBeforeNewerSecondCandidate() {
    let subscription = self.subscription()
    let displayed = event(id: String(format: "%064x", 0))
    let activeSibling = event(id: String(format: "%064x", 1))
    let newer = event(id: String(format: "%064x", 2), createdAt: 1_001)
    var state = PushConsumptionState()
    state.consume(
      PushEventPosition(createdAt: displayed.createdAt, id: displayed.id),
      for: "origin"
    )

    let first = PushCatchUp.orderedSelections(
      events: [newer, activeSibling, displayed],
      origin: "origin",
      subscriptions: [subscription],
      consumptionState: state,
      verify: { _ in true }
    ).first
    XCTAssertEqual(first?.event.id, activeSibling.id)

    state.consume(
      PushEventPosition(createdAt: activeSibling.createdAt, id: activeSibling.id),
      for: "origin"
    )
    let second = PushCatchUp.orderedSelections(
      events: [newer, activeSibling, displayed],
      origin: "origin",
      subscriptions: [subscription],
      consumptionState: state,
      verify: { _ in true }
    ).first
    XCTAssertEqual(second?.event.id, newer.id)
  }

  func testLateLowerIDSiblingIsSelectedThenDuplicateFallbackStaysSeparate() {
    let subscription = self.subscription()
    let higherID = String(repeating: "f", count: 64)
    let lowerID = String(repeating: "0", count: 64)
    let high = event(id: higherID)
    let lateLow = event(id: lowerID)
    var state = PushConsumptionState()
    state.consume(PushEventPosition(createdAt: 1_000, id: higherID), for: "origin")

    let originState = state.state(for: "origin")
    var pager = PushCatchUpPager(
      subscriptions: [subscription],
      since: state.querySince(for: "origin", now: 2_000),
      scan: originState.scan,
      pageLimit: 10
    )
    var relayPage: [VerifiedNostrEvent] = []
    while let filter = pager.nextFilter() {
      let page = relayResponse(events: [lateLow, high], filters: [filter])
      relayPage.append(contentsOf: page)
      pager.receive(rawPage: page)
    }
    let selections = PushCatchUp.orderedSelections(
      events: relayPage,
      origin: "origin",
      subscriptions: [subscription],
      consumptionState: state,
      verify: { _ in true }
    )

    XCTAssertEqual(selections.map(\.event.id), [lowerID, higherID])
    XCTAssertEqual(selections.map(\.wasPreviouslyConsumed), [false, true])
  }

  func testRawTailPagingReachesEverySameSecondEvent() {
    let subscription = self.subscription()
    let events = (0..<25).map { event(id: String(format: "%064x", $0)) }
    var pager = PushCatchUpPager(
      subscriptions: [subscription],
      since: nil,
      pageLimit: 10,
      maximumPages: 10
    )
    var observed: Set<String> = []

    while let filter = pager.nextFilter() {
      let page = relayResponse(events: events, filters: [filter])
      observed.formUnion(page.map(\.id))
      pager.receive(rawPage: page)
    }

    XCTAssertEqual(observed, Set(events.map(\.id)))
    XCTAssertEqual(pager.stopReason, .complete)
  }

  func testBudgetedTraversalResumesFromPersistedRawTailAndClearsOnlyOnComplete() {
    let subscription = self.subscription()
    let events = (0..<25).map {
      event(id: String(format: "%064x", $0), createdAt: 1_000 + $0)
    }
    var scan = PushCatchUpScan()
    var observations: [String: Int] = [:]
    var stops: [PushCatchUpStopReason] = []

    for _ in 0..<3 {
      var pager = PushCatchUpPager(
        subscriptions: [subscription],
        since: nil,
        scan: scan,
        pageLimit: 5,
        maximumPages: 2
      )
      while let filter = pager.nextFilter() {
        let page = relayResponse(events: events, filters: [filter])
        for event in page { observations[event.id, default: 0] += 1 }
        pager.receive(rawPage: page)
      }
      stops.append(try! XCTUnwrap(pager.stopReason))
      scan = pager.scan
    }

    XCTAssertEqual(stops, [.pageBudgetExceeded, .pageBudgetExceeded, .complete])
    XCTAssertEqual(observations.count, events.count)
    XCTAssertTrue(observations.values.allSatisfy { $0 == 1 })
    XCTAssertEqual(scan, PushCatchUpScan())
  }

  func testMultiChannelSubscriptionEmitsOneHTTPFilterPerHTag() {
    let firstChannel = "11111111-1111-4111-8111-111111111111"
    let secondChannel = "22222222-2222-4222-8222-222222222222"
    let subscription = PushLeaseSubscription(
      filter: PushLeaseFilter(
        kinds: [9],
        hTags: [firstChannel, secondChannel]
      ),
      notificationClass: "default"
    )
    var pager = PushCatchUpPager(subscriptions: [subscription], since: nil)
    var emittedHTags: [[String]] = []

    while let filter = pager.nextFilter() {
      emittedHTags.append(filter["#h"] as? [String] ?? [])
      pager.receive(rawPage: [])
    }

    XCTAssertEqual(emittedHTags, [[firstChannel], [secondChannel]])
    XCTAssertEqual(pager.stopReason, .complete)
  }

  private func subscription() -> PushLeaseSubscription {
    PushLeaseSubscription(
      filter: PushLeaseFilter(kinds: [9], pTags: [mine]),
      notificationClass: "default"
    )
  }

  private func event(id: String, createdAt: Int = 1_000) -> VerifiedNostrEvent {
    VerifiedNostrEvent(
      id: id,
      pubkey: other,
      createdAt: createdAt,
      kind: 9,
      tags: [["p", mine]],
      content: "message",
      sig: String(repeating: "c", count: 128)
    )
  }

  private func relayResponse(
    events: [VerifiedNostrEvent],
    filters: [[String: Any]]
  ) -> [VerifiedNostrEvent] {
    var byID: [String: VerifiedNostrEvent] = [:]
    for filter in filters {
      let limit = filter["limit"] as? Int ?? events.count
      let ids = (filter["ids"] as? [String]).map(Set.init)
      let since = filter["since"] as? Int
      let until = filter["until"] as? Int
      let beforeID = filter["before_id"] as? String
      let matches = events.filter { event in
        guard ids?.contains(event.id) ?? true else { return false }
        guard since.map({ event.createdAt >= $0 }) ?? true else { return false }
        if let until, let beforeID {
          return event.createdAt < until
            || (event.createdAt == until && event.id > beforeID)
        }
        return until.map({ event.createdAt <= $0 }) ?? true
      }.sorted {
        $0.createdAt == $1.createdAt ? $0.id < $1.id : $0.createdAt > $1.createdAt
      }.prefix(limit)
      for event in matches {
        byID[event.id] = event
      }
    }
    return Array(byID.values)
  }
}
