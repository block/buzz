import AVFoundation
import XCTest

@testable import BuzzNativeDriver

final class BuzzNativeDriverTests: XCTestCase {
  func testGlobalInputFenceRejectsAnotherFrontmostProcess() {
    XCTAssertThrowsError(try requireTargetIsFrontmost(pid: 42, frontmostPID: { 7 }))
  }

  func testGlobalInputFenceAcceptsTargetProcess() {
    XCTAssertNoThrow(try requireTargetIsFrontmost(pid: 42, frontmostPID: { 42 }))
  }

  func testEveryGlobalEventRechecksFrontmostOwnership() throws {
    var checks = [pid_t(42), pid_t(7)]
    var posts = 0
    let event = CGEvent(keyboardEventSource: nil, virtualKey: 48, keyDown: true)
    try postGlobalEvent(event, pid: 42, frontmostPID: { checks.removeFirst() }) { _ in posts += 1 }
    XCTAssertEqual(posts, 1)
    XCTAssertThrowsError(
      try postGlobalEvent(event, pid: 42, frontmostPID: { checks.removeFirst() }) { _ in posts += 1 }
    )
    XCTAssertEqual(posts, 1)
  }

  func testCaptureCadenceAdvancesAcrossBackpressureSkip() {
    var cadence = CaptureCadence()
    XCTAssertEqual(cadence.presentationTime, CMTime(value: 0, timescale: 15))
    cadence.advance()  // frame skipped because writer was not ready
    XCTAssertEqual(cadence.frame, 1)
    XCTAssertEqual(cadence.presentationTime, CMTime(value: 1, timescale: 15))
  }
}
