import XCTest

@testable import BuzzPushKit

final class PushNotificationIdentityTests: XCTestCase {
  private let identity = PushNotificationIdentity(eventID: "EVENT", origin: "origin")

  func testSequentialDuplicateRemovalStopsStackGrowthButDuplicateWakeCanStillAlertBeforeOlderCopyIsRemoved() throws {
    let requestIdentifiers = try PushDuplicateAbsorption.requestIdentifiersToRemove(
      matching: identity,
      from: [
        PushDeliveredNotificationRecord(
          requestIdentifier: "older-request",
          userInfo: [PushNotificationIdentity.userInfoKey: identity.userInfoValue]
        )
      ]
    )

    XCTAssertEqual(requestIdentifiers, ["older-request"])
  }

  func testConcurrentNSEInvocationsCanEachMissTheOthersDeliveredNotificationAndBothLeaveACopy() throws {
    let firstObserved = try PushDuplicateAbsorption.requestIdentifiersToRemove(
      matching: identity,
      from: []
    )
    let secondObserved = try PushDuplicateAbsorption.requestIdentifiersToRemove(
      matching: identity,
      from: []
    )

    XCTAssertTrue(firstObserved.isEmpty)
    XCTAssertTrue(secondObserved.isEmpty)
  }

  func testMissingOrMalformedCurrentIdentityFailsLoudly() {
    XCTAssertThrowsError(try PushNotificationIdentity.require(from: [:])) { error in
      XCTAssertEqual(error as? PushNotificationIdentityError, .missingIdentity)
    }
    XCTAssertThrowsError(
      try PushNotificationIdentity.require(
        from: [PushNotificationIdentity.userInfoKey: ["event_id": "event"]]
      )
    ) { error in
      XCTAssertEqual(error as? PushNotificationIdentityError, .malformedIdentity)
    }
  }

  func testUnrelatedDeliveredNotificationWithoutBuzzIdentityIsIgnored() throws {
    let requestIdentifiers = try PushDuplicateAbsorption.requestIdentifiersToRemove(
      matching: identity,
      from: [
        PushDeliveredNotificationRecord(
          requestIdentifier: "unrelated",
          userInfo: ["other": "value"]
        )
      ]
    )

    XCTAssertTrue(requestIdentifiers.isEmpty)
  }

  func testMalformedDeliveredBuzzIdentityFailsLoudly() {
    XCTAssertThrowsError(
      try PushDuplicateAbsorption.requestIdentifiersToRemove(
        matching: identity,
        from: [
          PushDeliveredNotificationRecord(
            requestIdentifier: "malformed",
            userInfo: [
              PushNotificationIdentity.userInfoKey: ["event_id": "event"]
            ]
          )
        ]
      )
    ) { error in
      XCTAssertEqual(error as? PushNotificationIdentityError, .malformedIdentity)
    }
  }
}
