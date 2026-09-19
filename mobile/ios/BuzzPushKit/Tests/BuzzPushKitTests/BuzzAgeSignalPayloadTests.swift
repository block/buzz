import Foundation
import Testing

@testable import BuzzPushKit

struct BuzzAgeSignalPayloadTests {
  @Test func `Apple under 18 gate produces a restricted inclusive age`() {
    let payload = BuzzAgeSignalPayload.sharing(exclusiveUpperBound: 18)
    #expect(payload["status"] as? String == "signal")
    #expect(payload["ageUpper"] as? Int == 17)
  }

  @Test func `Apple unbounded adult range stays unbounded`() {
    let payload = BuzzAgeSignalPayload.sharing(exclusiveUpperBound: nil)
    #expect(payload["status"] as? String == "signal")
    #expect(payload["ageUpper"] is NSNull)
  }
  @Test(arguments: [Int.min, -1, 0, 1, 17, 19, Int.max])
  func unexpectedAppleBoundsAreNotRestrictionEvidence(upper: Int) {
    let payload = BuzzAgeSignalPayload.sharing(exclusiveUpperBound: upper)
    #expect(payload["ageUpper"] is NSNull)
  }
  @Test(arguments: [Int.min, -1, 18, 19, Int.max])
  func contradictoryAppleRangeIsNotRestrictionEvidence(lower: Int) {
    let payload = BuzzAgeSignalPayload.sharing(exclusiveUpperBound: 18, lowerBound: lower)
    #expect(payload["ageUpper"] is NSNull)
  }
}
