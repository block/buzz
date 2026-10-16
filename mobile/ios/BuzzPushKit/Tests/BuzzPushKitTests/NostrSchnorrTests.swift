import Foundation
import XCTest

@testable import BuzzPushKit

final class NostrSchnorrTests: XCTestCase {
  func testOfficialBIP340Vectors() throws {
    let url = try XCTUnwrap(Bundle.module.url(forResource: "bip340", withExtension: "csv"))
    let rows = try String(contentsOf: url, encoding: .utf8).split(whereSeparator: \.isNewline)
      .dropFirst()
    var checked = 0
    for row in rows {
      let columns = row.split(separator: ",", omittingEmptySubsequences: false).map(String.init)
      let message = try XCTUnwrap(VerifiedNostrEvent.hexBytes(columns[4]))
      // Nostr signs a SHA-256 event ID, always exactly 32 bytes.
      guard message.count == 32 else { continue }
      let pubkey = try XCTUnwrap(VerifiedNostrEvent.hexBytes(columns[2]))
      let signature = try XCTUnwrap(VerifiedNostrEvent.hexBytes(columns[5]))
      XCTAssertEqual(
        NostrSchnorr.verify(signature: signature, message: message, publicKey: pubkey),
        columns[6] == "TRUE", "vector \(columns[0])")
      if !columns[1].isEmpty {
        let secret = try XCTUnwrap(VerifiedNostrEvent.hexBytes(columns[1]))
        let auxiliary = try XCTUnwrap(VerifiedNostrEvent.hexBytes(columns[3]))
        XCTAssertEqual(try NostrSchnorr.publicKey(secret: secret), pubkey)
        XCTAssertEqual(
          try NostrSchnorr.sign(
            message: message, secret: secret,
            auxiliaryRandomness: auxiliary), signature)
      }
      checked += 1
    }
    XCTAssertEqual(checked, 15)
  }

  func testRejectsInvalidSecretsAndLengths() throws {
    let message = [UInt8](repeating: 0, count: 32)
    for secret in [[], [1], [UInt8](repeating: 0, count: 32), [UInt8](repeating: 255, count: 32)] {
      XCTAssertThrowsError(try NostrSchnorr.publicKey(secret: secret))
      XCTAssertThrowsError(try NostrSchnorr.sign(message: message, secret: secret))
    }
    let secret = [UInt8](repeating: 0, count: 31) + [3]
    XCTAssertThrowsError(try NostrSchnorr.sign(message: [], secret: secret))
    XCTAssertThrowsError(
      try NostrSchnorr.sign(message: message, secret: secret, auxiliaryRandomness: [1]))
    XCTAssertFalse(NostrSchnorr.verify(signature: [], message: message, publicKey: message))
    XCTAssertFalse(
      NostrSchnorr.verify(
        signature: [UInt8](repeating: 0, count: 64), message: [], publicKey: message))
  }
}
