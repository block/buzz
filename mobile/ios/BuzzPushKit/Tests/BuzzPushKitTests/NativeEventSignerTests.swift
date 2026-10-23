import CSecp256k1
import Foundation
import XCTest

@testable import BuzzPushKit

final class NativeEventSignerTests: XCTestCase {
  func testFixtureKeysAndCrossImplementationSignatures() throws {
    let url = try XCTUnwrap(Bundle.module.url(forResource: "bip340", withExtension: "csv"))
    for row in try String(contentsOf: url, encoding: .utf8).split(whereSeparator: \.isNewline).dropFirst() {
      let fields = row.split(separator: ",", omittingEmptySubsequences: false).map(String.init)
      guard !fields[1].isEmpty else { continue }
      let secret = try XCTUnwrap(VerifiedNostrEvent.hexBytes(fields[1]))
      let message = [UInt8](repeating: 42, count: 32)
      var publicKey = [UInt8](repeating: 0, count: 32)
      var handle: OpaquePointer?
      XCTAssertEqual(buzz_signer_create(secret, &publicKey, &handle), 0)
      let signer = try XCTUnwrap(handle)
      defer { buzz_signer_destroy(signer) }
      XCTAssertEqual(publicKey, try XCTUnwrap(VerifiedNostrEvent.hexBytes(fields[2])))
      var signature = [UInt8](repeating: 0, count: 64)
      XCTAssertEqual(buzz_signer_sign(signer, message, &signature), 0)
      XCTAssertTrue(NostrSchnorr.verify(signature: signature, message: message, publicKey: publicKey))
    }
  }

  func testInvalidScalarDoesNotReturnHandle() {
    for secret in [[UInt8](repeating: 0, count: 32), [UInt8](repeating: 255, count: 32)] {
      var publicKey = [UInt8](repeating: 0, count: 32)
      var handle: OpaquePointer?
      XCTAssertEqual(buzz_signer_create(secret, &publicKey, &handle), 1)
      XCTAssertNil(handle)
    }
    buzz_signer_destroy(nil)
  }

  func testConcurrentIndependentKeypairs() {
    DispatchQueue.concurrentPerform(iterations: 64) { index in
      let secret = [UInt8](repeating: 0, count: 31) + [UInt8(index + 1)]
      var publicKey = [UInt8](repeating: 0, count: 32)
      var handle: OpaquePointer?
      XCTAssertEqual(buzz_signer_create(secret, &publicKey, &handle), 0)
      defer { buzz_signer_destroy(handle) }
      let message = [UInt8](repeating: UInt8(index), count: 32)
      var signature = [UInt8](repeating: 0, count: 64)
      XCTAssertEqual(buzz_signer_sign(handle, message, &signature), 0)
      XCTAssertTrue(NostrSchnorr.verify(signature: signature, message: message, publicKey: publicKey))
    }
  }
}
