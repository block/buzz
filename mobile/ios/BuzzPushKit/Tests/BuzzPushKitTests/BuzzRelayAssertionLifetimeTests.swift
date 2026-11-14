import XCTest

@testable import BuzzPushKit

final class BuzzRelayAssertionLifetimeTests: XCTestCase {
  func testDeadlineWithPhoneClock120SecondsSlowOrFast() {
    // Issuer: iat 10_000, exp 10_180. Request sent at uptime 1_000, answered at once.
    func deadline(wall: Int) -> TimeInterval {
      BuzzRelayAssertionIssuer.localDeadline(
        requestStart: 1_000, receipt: 1_000,
        wallAtReceipt: Date(timeIntervalSince1970: TimeInterval(wall)), exp: 10_180, iat: 10_000)
    }
    XCTAssertEqual(deadline(wall: 9_880), 1_180, "slow: the lifetime caps it")
    XCTAssertEqual(deadline(wall: 10_120), 1_060, "fast: only what exp leaves")
    XCTAssertEqual(deadline(wall: 10_181), 1_000, "none left: refused at receipt")
  }
}
