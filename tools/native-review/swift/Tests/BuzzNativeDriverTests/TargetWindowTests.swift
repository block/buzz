import CoreGraphics
import Testing
@testable import BuzzNativeDriver

@Test func targetWindowContainsOnlyOwnedCoordinates() {
    let bounds = CGRect(x: 100, y: 200, width: 400, height: 300)

    #expect(targetWindowContains(point: CGPoint(x: 100, y: 200), bounds: bounds))
    #expect(targetWindowContains(point: CGPoint(x: 499.999, y: 499.999), bounds: bounds))
    #expect(!targetWindowContains(point: CGPoint(x: 99.999, y: 250), bounds: bounds))
    #expect(!targetWindowContains(point: CGPoint(x: 500, y: 250), bounds: bounds))
    #expect(!targetWindowContains(point: CGPoint(x: 250, y: 500), bounds: bounds))
    #expect(!targetWindowContains(point: CGPoint(x: CGFloat.nan, y: 250), bounds: bounds))
    #expect(!targetWindowContains(point: CGPoint(x: 250, y: CGFloat.infinity), bounds: bounds))
}

@Test func targetOwnershipRejectsFocusStealAndOffWindowInput() throws {
    let bounds = CGRect(x: 100, y: 200, width: 400, height: 300)

    #expect(
        safePointerAnimationStart(
            current: CGPoint(x: 250, y: 300), target: CGPoint(x: 300, y: 350), windowBounds: bounds
        ) == CGPoint(x: 250, y: 300)
    )
    #expect(
        safePointerAnimationStart(
            current: CGPoint(x: 50, y: 300), target: CGPoint(x: 300, y: 350), windowBounds: bounds
        ) == CGPoint(x: 300, y: 350)
    )
    try validateTargetOwns(
        point: CGPoint(x: 250, y: 300), frontmostPID: 42, targetPID: 42, windowBounds: bounds
    )
    #expect(throws: Error.self) {
        try validateTargetOwns(
            point: CGPoint(x: 250, y: 300), frontmostPID: 99, targetPID: 42, windowBounds: bounds
        )
    }
    #expect(throws: Error.self) {
        try validateTargetOwns(
            point: CGPoint(x: 50, y: 300), frontmostPID: 42, targetPID: 42, windowBounds: bounds
        )
    }
}
