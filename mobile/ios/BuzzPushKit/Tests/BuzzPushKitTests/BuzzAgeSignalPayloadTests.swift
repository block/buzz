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
}
