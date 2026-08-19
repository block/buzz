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

  func testPointerInputRequiresTargetWindowBounds() throws {
    let event = CGEvent(mouseEventSource: nil, mouseType: .mouseMoved,
                        mouseCursorPosition: CGPoint(x: 25, y: 25), mouseButton: .left)
    var posts = 0
    try postGlobalPointerEvent(
      event, at: CGPoint(x: 25, y: 25), pid: 42,
      frontmostPID: { 42 }, targetBounds: { _ in CGRect(x: 0, y: 0, width: 50, height: 50) }
    ) { _ in posts += 1 }
    XCTAssertEqual(posts, 1)
    XCTAssertThrowsError(
      try postGlobalPointerEvent(
        event, at: CGPoint(x: 75, y: 25), pid: 42,
        frontmostPID: { 42 }, targetBounds: { _ in CGRect(x: 0, y: 0, width: 50, height: 50) }
      ) { _ in posts += 1 }
    )
    XCTAssertEqual(posts, 1)
  }

  func testCaptureTickAdvancesAcrossBackpressureSkipThenUsesAdvancedPTS() {
    var cadence = CaptureCadence()
    var appended = [CMTime]()

    XCTAssertTrue(captureTick(cadence: &cadence, isReady: false) { time in
      appended.append(time)
      return true
    })
    XCTAssertEqual(cadence.frame, 1)
    XCTAssertTrue(appended.isEmpty)

    XCTAssertTrue(captureTick(cadence: &cadence, isReady: true) { time in
      appended.append(time)
      return true
    })
    XCTAssertEqual(cadence.frame, 2)
    XCTAssertEqual(appended, [CMTime(value: 1, timescale: 15)])
  }
}
