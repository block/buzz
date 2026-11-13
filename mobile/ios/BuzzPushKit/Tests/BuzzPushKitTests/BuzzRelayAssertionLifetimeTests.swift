import XCTest

@testable import BuzzPushKit

final class BuzzRelayAssertionLifetimeTests: XCTestCase {
  private let iat = 10_000
  private let exp = 10_180

  private func deadline(delay: TimeInterval, wall: Int, iat: Int) -> TimeInterval {
    BuzzRelayAssertionIssuer.localDeadline(
      requestStart: 1_000, receipt: 1_000 + delay,
      wallAtReceipt: Date(timeIntervalSince1970: TimeInterval(wall)), exp: exp, iat: iat)
  }

  func testPhone120SecondsSlowDoesNotStretchTheToken() {
    XCTAssertEqual(deadline(delay: 0, wall: iat - 120, iat: iat), 1_180)
  }

  func testPhone120SecondsFastKeepsOnlyWhatExpLeaves() {
    XCTAssertEqual(deadline(delay: 0, wall: iat + 120, iat: iat), 1_060)
  }

  func testDelayedReplyCountsAgainstTheToken() {
    XCTAssertEqual(deadline(delay: 20, wall: iat - 100, iat: iat), 1_180)
  }

  func testNoBudgetLeftEndsAtReceipt() {
    XCTAssertEqual(deadline(delay: 5, wall: exp + 1, iat: iat), 1_005)
  }

  func testIssuedAtReadsTheJWSPayload() {
    // {"exp":10180,"iat":10000}
    let payload = Data(#"{"exp":10180,"iat":10000}"#.utf8).base64EncodedString()
      .replacingOccurrences(of: "=", with: "")
    XCTAssertEqual(BuzzRelayAssertionIssuer.issuedAt("e30.\(payload).c2ln"), 10_000)
    XCTAssertNil(BuzzRelayAssertionIssuer.issuedAt("a.b.c"))
    XCTAssertNil(BuzzRelayAssertionIssuer.issuedAt("not-a-jws"))
  }
}
