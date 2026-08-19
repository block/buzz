import CoreGraphics
import Testing
@testable import BuzzNativeDriverSupport

@Test func captureScheduleAdvancesAcrossBackpressure() {
    let start = ContinuousClock.now
    var schedule = CaptureSchedule(start: start, framesPerSecond: 15)

    let blockedTicks = (0..<4).map { _ in schedule.advance(isReadyForMoreMediaData: false) }

    #expect(blockedTicks.allSatisfy { !$0.shouldCapture })
    #expect(blockedTicks.map(\.presentationTime.value) == [0, 1, 2, 3])
    #expect(blockedTicks.map(\.deadline) == [
        start.advanced(by: .milliseconds(66)),
        start.advanced(by: .milliseconds(133)),
        start.advanced(by: .milliseconds(200)),
        start.advanced(by: .milliseconds(266)),
    ])
    #expect(schedule.frame == 4)
}

@Test func captureScheduleMarksReadyTickForCapture() {
    var schedule = CaptureSchedule()
    #expect(schedule.advance(isReadyForMoreMediaData: true).shouldCapture)
}

@Test func scrollEventUsesTargetFrameCenter() throws {
    let frame = CGRect(x: 40, y: 80, width: 120, height: 60)
    let event = try #require(makeScrollEvent(deltaY: 240, targetFrame: frame))

    #expect(event.location == CGPoint(x: 100, y: 110))
}
