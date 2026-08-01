import Foundation
import XCTest

@testable import BuzzPushKit

final class PushConsumptionStateTests: XCTestCase {
  func testSameSecondSiblingsRemainReachable() {
    var state = PushConsumptionState()
    let first = PushEventPosition(createdAt: 1_000, id: "a")
    let sibling = PushEventPosition(createdAt: 1_000, id: "b")

    state.consume(first, for: "origin")

    XCTAssertEqual(state.querySince(for: "origin", now: 2_000), 1_000)
    XCTAssertFalse(state.canSelect(first, for: "origin", now: 2_000))
    XCTAssertTrue(state.canSelect(sibling, for: "origin", now: 2_000))
  }

  func testLosingOriginIsNotConsumed() {
    var state = PushConsumptionState()
    state.consume(PushEventPosition(createdAt: 2_000, id: "winner"), for: "winner-origin")

    XCTAssertTrue(
      state.canSelect(
        PushEventPosition(createdAt: 1_999, id: "loser"),
        for: "loser-origin",
        now: 3_000
      )
    )
    XCTAssertNil(state.querySince(for: "loser-origin", now: 3_000))
  }

  func testDeliveredEventIsNeverSelectedAgainAndActiveSecondHistoryIsExact() {
    var state = PushConsumptionState()
    for index in 0..<70 {
      state.consume(
        PushEventPosition(createdAt: 1_000, id: String(format: "%064x", index)),
        for: "origin"
      )
    }
    let latest = PushEventPosition(createdAt: 1_000, id: String(format: "%064x", 69))

    XCTAssertFalse(state.canSelect(latest, for: "origin", now: 2_000))
    XCTAssertEqual(state.state(for: "origin").delivered.count, 70)
  }

  func testActiveSecondHistoryPrunesWhenTimestampAdvances() {
    var state = PushConsumptionState()
    for index in 0..<70 {
      state.consume(
        PushEventPosition(createdAt: 1_000, id: String(format: "%064x", index)),
        for: "origin"
      )
    }
    let first = PushEventPosition(createdAt: 1_000, id: String(format: "%064x", 0))
    let nextSecond = PushEventPosition(createdAt: 1_001, id: "next")

    XCTAssertTrue(state.hasConsumed(eventID: first.id, for: "origin"))
    state.consume(nextSecond, for: "origin")

    XCTAssertFalse(state.hasConsumed(eventID: first.id, for: "origin"))
    XCTAssertEqual(state.state(for: "origin").delivered, [nextSecond])
  }

  func testDisplayedIDsAtOrAboveFloorAreRetained() {
    var state = PushConsumptionState()
    let floor = PushEventPosition(createdAt: 1_000, id: "floor")
    let newer = PushEventPosition(createdAt: 1_002, id: "newer")
    let middle = PushEventPosition(createdAt: 1_001, id: "middle")

    state.consume(floor, for: "origin")
    state.consume(newer, for: "origin", advanceFloor: false)
    state.consume(middle, for: "origin", advanceFloor: false)

    let originState = state.state(for: "origin")
    XCTAssertEqual(originState.cursor, floor)
    XCTAssertEqual(originState.delivered, [floor, middle, newer])
  }

  func testIncompleteTraversalKeepsFloorAndPersistsScanThroughConsumeAndCodable() throws {
    let floor = PushEventPosition(createdAt: 1_000, id: "floor")
    let displayed = PushEventPosition(createdAt: 1_015, id: "displayed")
    let before = PushEventPosition(createdAt: 1_010, id: "raw-tail")
    let scan = PushCatchUpScan(subscriptionIndex: 2, before: before)
    var state = PushConsumptionState()
    state.consume(floor, for: "origin")

    state.consume(displayed, for: "origin", advanceFloor: false, scan: scan)

    let originState = state.state(for: "origin")
    XCTAssertEqual(originState.cursor, floor)
    XCTAssertEqual(originState.scan, scan)
    XCTAssertTrue(state.hasConsumed(eventID: displayed.id, for: "origin"))
    let decoded = try JSONDecoder().decode(
      PushConsumptionState.self,
      from: JSONEncoder().encode(state)
    )
    XCTAssertEqual(decoded.state(for: "origin"), originState)
  }

  func testNegativePersistedScanIndexIsRejected() throws {
    let data = try XCTUnwrap(
      #"{"version":1,"origins":{"origin":{"delivered":[],"scan":{"subscriptionIndex":-1}}}}"#
        .data(using: .utf8)
    )

    XCTAssertThrowsError(try JSONDecoder().decode(PushConsumptionState.self, from: data))
  }

  func testLowerIDSiblingArrivingAfterHigherIDRemainsSelectable() {
    var state = PushConsumptionState()
    let higher = PushEventPosition(createdAt: 1_000, id: "ff")
    let laterLower = PushEventPosition(createdAt: 1_000, id: "00")

    state.consume(higher, for: "origin")

    XCTAssertTrue(state.canSelect(laterLower, for: "origin", now: 2_000))
  }

  func testEventOlderThanCompositeCursorIsNotSelected() {
    var state = PushConsumptionState()
    state.consume(PushEventPosition(createdAt: 2_000, id: "newer"), for: "origin")

    XCTAssertFalse(
      state.canSelect(
        PushEventPosition(createdAt: 1_999, id: "older"),
        for: "origin",
        now: 3_000
      )
    )
  }

  func testEventBeyondAllowedFutureSkewIsNotSelected() {
    let state = PushConsumptionState()

    XCTAssertTrue(
      state.canSelect(
        PushEventPosition(createdAt: 1_300, id: "boundary"),
        for: "origin",
        now: 1_000,
        allowedFutureSkew: 300
      )
    )
    XCTAssertFalse(
      state.canSelect(
        PushEventPosition(createdAt: 1_301, id: "future"),
        for: "origin",
        now: 1_000,
        allowedFutureSkew: 300
      )
    )
  }

  func testInterleavedWritersCannotLoseDeliveredIDOrMoveCursorBackward() throws {
    let lock = TestLock()
    let persistence = TestPersistence()
    let storeA = PushConsumptionStateStore(lock: lock, persistence: persistence)
    let storeB = PushConsumptionStateStore(lock: lock, persistence: persistence)
    let writerARead = expectation(description: "writer A entered")
    let letWriterAFinish = DispatchSemaphore(value: 0)
    let completed = expectation(description: "writers completed")
    completed.expectedFulfillmentCount = 2

    DispatchQueue.global().async {
      defer { completed.fulfill() }
      do {
        try storeA.update { state in
          writerARead.fulfill()
          _ = letWriterAFinish.wait(timeout: .now() + 2)
          state.consume(PushEventPosition(createdAt: 2_000, id: "b"), for: "origin")
        }
      } catch {
        XCTFail("writer A failed: \(error)")
      }
    }
    wait(for: [writerARead], timeout: 1)
    DispatchQueue.global().async {
      defer { completed.fulfill() }
      do {
        try storeB.update { state in
          state.consume(PushEventPosition(createdAt: 2_000, id: "a"), for: "origin")
        }
      } catch {
        XCTFail("writer B failed: \(error)")
      }
    }
    letWriterAFinish.signal()
    wait(for: [completed], timeout: 3)

    let state = try storeA.read().state(for: "origin")
    XCTAssertEqual(state.cursor, PushEventPosition(createdAt: 2_000, id: "b"))
    XCTAssertEqual(Set(state.delivered.map(\.id)), ["a", "b"])
  }

  func testStateFileIsVersionedAndDeterministic() throws {
    let persistence = TestPersistence()
    let store = PushConsumptionStateStore(lock: TestLock(), persistence: persistence)
    try store.update { state in
      state.consume(PushEventPosition(createdAt: 1_000, id: "event"), for: "origin")
    }

    let json = try XCTUnwrap(
      persistence.data.flatMap {
        try? JSONSerialization.jsonObject(with: $0) as? [String: Any]
      })
    XCTAssertEqual(json["version"] as? Int, 1)
    XCTAssertNotNil(json["origins"])
  }
}

private final class TestLock: PushConsumptionStateLocking, @unchecked Sendable {
  private let lock = NSRecursiveLock()

  func withExclusiveLock<T>(_ body: () throws -> T) throws -> T {
    lock.lock()
    defer { lock.unlock() }
    return try body()
  }
}

private final class TestPersistence: PushConsumptionStatePersisting, @unchecked Sendable {
  var data: Data?

  func read() throws -> Data? { data }
  func write(_ data: Data) throws { self.data = data }
}
