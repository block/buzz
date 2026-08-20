import CoreGraphics
import Foundation
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

@Test func globalInputPostingCannotBypassAuthorization() throws {
    guard let event = CGEvent(
        mouseEventSource: nil, mouseType: .mouseMoved,
        mouseCursorPosition: CGPoint(x: 150, y: 250), mouseButton: .left
    ) else {
        Issue.record("could not create test mouse event")
        return
    }
    var authorizationCalls = 0
    var posted = false

    #expect(throws: Error.self) {
        try postGlobalInput(
            event, at: CGPoint(x: 150, y: 250), pid: 42,
            authorize: { _, _ in
                authorizationCalls += 1
                throw DriverError.message("denied by test ownership boundary")
            },
            post: { _ in posted = true }
        )
    }
    #expect(authorizationCalls == 1)
    #expect(!posted)
}

@Test func productionGlobalInputCallSitesUseTheAuthorizationChokePoint() throws {
    let source = try String(contentsOf: URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .appendingPathComponent("Sources/BuzzNativeDriver/main.swift"), encoding: .utf8)
    let helperCalls = source.components(separatedBy: "try postGlobalInput(").count - 1
    let rawGlobalPosts = source.components(separatedBy: ".post(tap: .cghidEventTap)").count - 1

    // Scroll posts two global events, pointer movement posts one in each branch,
    // and click posts move/down/up. Any new or removed site must update this
    // contract rather than silently bypass target ownership authorization.
    #expect(helperCalls == 7)
    #expect(rawGlobalPosts == 1)
}
