import UIKit
import XCTest
@testable import Buzz

final class SavedMessageCacheLifecycleTests: XCTestCase {
  @MainActor func testExpirationInterruptsAndWaitsForCloseBeforeEndingAssertion() throws {
    let worker = SavedMessageCacheWorkerState()
    let probe = InterruptProbe()
    let allowClose = DispatchSemaphore(value: 0)
    defer { allowClose.signal() }
    var expire: (() -> Void)?
    let lifecycle = SavedMessageCacheLifecycle(
      worker: worker,
      begin: { expire = $0; return UIBackgroundTaskIdentifier(rawValue: 42) },
      end: { probe.record("end \($0.rawValue)") },
      waitForClose: { finished in
        XCTAssertEqual(probe.events, ["interrupt"])
        allowClose.signal()
        XCTAssertEqual(finished.wait(timeout: .now() + .seconds(2)), .success)
      }
    )
    lifecycle.didBecomeActive()
    let lease = try XCTUnwrap(lifecycle.acquire())
    XCTAssertTrue(worker.start(lease))
    XCTAssertTrue(worker.register(lease, handle: Unmanaged.passUnretained(probe).toOpaque()) { pointer in
      let probe = Unmanaged<InterruptProbe>.fromOpaque(pointer).takeUnretainedValue()
      probe.record("interrupt")
      probe.interrupted.signal()
    })
    let closed = expectation(description: "Worker cleaned up after native interruption")
    DispatchQueue.global().async {
      guard probe.interrupted.wait(timeout: .now() + .seconds(2)) == .success else {
        XCTFail("Expiration failed to call the active connection's interrupt")
        closed.fulfill()
        return
      }
      guard allowClose.wait(timeout: .now() + .seconds(2)) == .success else {
        XCTFail("Expiration did not wait for the worker to close")
        closed.fulfill()
        return
      }
      XCTAssertTrue(worker.isCancelled(lease))
      worker.detach(lease)
      probe.record("close")
      _ = worker.finish(lease)
      closed.fulfill()
    }

    try XCTUnwrap(expire)()
    XCTAssertEqual(probe.events, ["interrupt", "close", "end 42"])
    lifecycle.release(lease)
    try XCTUnwrap(expire)()
    XCTAssertEqual(probe.events, ["interrupt", "close", "end 42"])
    wait(for: [closed], timeout: 2)
  }

  @MainActor func testExpiredWorkerBlocksAnotherLeaseUntilItCloses() throws {
    let worker = SavedMessageCacheWorkerState()
    var expire: (() -> Void)?
    var ended: [Int] = []
    let lifecycle = SavedMessageCacheLifecycle(
      worker: worker,
      begin: { expire = $0; return UIBackgroundTaskIdentifier(rawValue: 42) },
      end: { ended.append($0.rawValue) },
      waitForClose: { _ in }
    )
    lifecycle.didBecomeActive()
    let lease = try XCTUnwrap(lifecycle.acquire())
    // The worker can be inside open, or inside close after detaching.
    XCTAssertTrue(worker.start(lease))
    try XCTUnwrap(expire)()
    XCTAssertEqual(ended, [42])
    XCTAssertTrue(worker.isCancelled(lease))
    XCTAssertNil(lifecycle.acquire())
    XCTAssertTrue(worker.finish(lease))
    let freshLease = try XCTUnwrap(lifecycle.acquire())
    XCTAssertNotEqual(freshLease, lease)
    XCTAssertFalse(worker.start(lease))
    lifecycle.release(freshLease)
    XCTAssertEqual(ended, [42, 42])
  }

  @MainActor func testInactiveSceneAndRefusedAssertionNeverAdmitWork() throws {
    let worker = SavedMessageCacheWorkerState()
    var allowAssertion = false
    var ended = 0
    let lifecycle = SavedMessageCacheLifecycle(
      worker: worker,
      begin: { _ in allowAssertion ? UIBackgroundTaskIdentifier(rawValue: 42) : .invalid },
      end: { _ in ended += 1 }
    )
    XCTAssertNil(lifecycle.acquire())
    lifecycle.didBecomeActive()
    XCTAssertNil(lifecycle.acquire())
    XCTAssertEqual(ended, 0)

    allowAssertion = true
    let queued = try XCTUnwrap(lifecycle.acquire())
    lifecycle.willResignActive()
    XCTAssertFalse(worker.start(queued))
    XCTAssertNil(lifecycle.acquire())
    XCTAssertEqual(ended, 1)
  }

  @MainActor func testExpirationDuringAcquireEndsReturnedTaskAndFreesSlot() throws {
    let worker = SavedMessageCacheWorkerState()
    var expireOnBegin = true
    var ended = 0
    let lifecycle = SavedMessageCacheLifecycle(
      worker: worker,
      begin: { expire in
        if expireOnBegin { expire() }
        return UIBackgroundTaskIdentifier(rawValue: 42)
      },
      end: { _ in ended += 1 }
    )
    lifecycle.didBecomeActive()
    XCTAssertNil(lifecycle.acquire())
    XCTAssertEqual(ended, 1)
    expireOnBegin = false
    lifecycle.release(try XCTUnwrap(lifecycle.acquire()))
    XCTAssertEqual(ended, 2)
  }

  @MainActor func testBackgroundCancelsRunningWorkerAndKeepsAssertionUntilRelease() throws {
    let worker = SavedMessageCacheWorkerState()
    let probe = InterruptProbe()
    let lifecycle = SavedMessageCacheLifecycle(
      worker: worker,
      begin: { _ in UIBackgroundTaskIdentifier(rawValue: 42) },
      end: { _ in probe.record("end") }
    )
    lifecycle.didBecomeActive()
    let lease = try XCTUnwrap(lifecycle.acquire())
    XCTAssertTrue(worker.start(lease))
    XCTAssertTrue(worker.register(lease, handle: Unmanaged.passUnretained(probe).toOpaque()) { pointer in
      Unmanaged<InterruptProbe>.fromOpaque(pointer).takeUnretainedValue().record("interrupt")
    })
    lifecycle.willResignActive()
    XCTAssertTrue(probe.events.isEmpty)
    lifecycle.didEnterBackground()
    XCTAssertTrue(worker.isCancelled(lease))
    worker.detach(lease)
    probe.record("close")
    XCTAssertFalse(worker.finish(lease))
    lifecycle.release(lease)
    XCTAssertEqual(probe.events, ["interrupt", "close", "end"])
  }
}

private final class InterruptProbe: @unchecked Sendable {
  let interrupted = DispatchSemaphore(value: 0)
  private let lock = NSLock()
  private var trace: [String] = []

  var events: [String] { lock.withLock { trace } }

  func record(_ event: String) {
    lock.withLock { trace.append(event) }
  }
}
