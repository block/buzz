import Foundation
import XCTest
@testable import BuzzPushKit

final class APNsRegistrationBufferTests: XCTestCase {
    func testReplaysTokenAfterChannelAttachment() {
        let buffer = APNsRegistrationBuffer()
        buffer.recordToken(Data([0x01, 0xAB, 0x00]))
        var delivered: [APNsRegistrationUpdate] = []
        buffer.attach { delivered.append($0) }
        XCTAssertEqual(delivered, [
            APNsRegistrationUpdate(method: "apnsTokenChanged", arguments: ["token": "01ab00"])
        ])
        XCTAssertNil(buffer.pending)
    }

    func testKeepsLatestUpdateAndDeliversLiveFailures() {
        let buffer = APNsRegistrationBuffer()
        buffer.recordToken(Data([0x01]))
        buffer.recordError("offline")
        var delivered: [APNsRegistrationUpdate] = []
        buffer.attach { delivered.append($0) }
        buffer.recordError("denied")
        XCTAssertEqual(delivered, [
            APNsRegistrationUpdate(method: "apnsRegistrationFailed", arguments: ["message": "offline"]),
            APNsRegistrationUpdate(method: "apnsRegistrationFailed", arguments: ["message": "denied"]),
        ])
    }
}
