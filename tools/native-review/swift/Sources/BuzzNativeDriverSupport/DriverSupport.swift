import CoreGraphics
import CoreMedia
import Foundation

public struct CaptureTick {
    public let shouldCapture: Bool
    public let presentationTime: CMTime
    public let deadline: ContinuousClock.Instant
}

public struct CaptureSchedule {
    private let start: ContinuousClock.Instant
    private let framesPerSecond: Int64
    public private(set) var frame: Int64 = 0

    public init(start: ContinuousClock.Instant = .now, framesPerSecond: Int64 = 15) {
        self.start = start
        self.framesPerSecond = framesPerSecond
    }

    public mutating func advance(isReadyForMoreMediaData: Bool) -> CaptureTick {
        let tick = CaptureTick(
            shouldCapture: isReadyForMoreMediaData,
            presentationTime: CMTime(value: frame, timescale: Int32(framesPerSecond)),
            deadline: start.advanced(by: .milliseconds(Int((frame + 1) * 1000 / framesPerSecond)))
        )
        frame += 1
        return tick
    }
}

public func makeScrollEvent(deltaY: Int32, targetFrame: CGRect) -> CGEvent? {
    guard let event = CGEvent(
        scrollWheelEvent2Source: nil,
        units: .pixel,
        wheelCount: 1,
        wheel1: deltaY,
        wheel2: 0,
        wheel3: 0
    ) else { return nil }
    event.location = CGPoint(x: targetFrame.midX, y: targetFrame.midY)
    return event
}
