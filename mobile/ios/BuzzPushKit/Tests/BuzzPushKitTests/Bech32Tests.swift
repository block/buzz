import XCTest
@testable import BuzzPushKit

/// Bech32 codec tests against independently published npub vectors — the
/// NIP-19 spec example key and the nostr crate 0.44 test suite — plus
/// degenerate key material for the padding and payload-length boundaries.
/// Generic BIP-173 conformance is not asserted here: nothing outside the
/// codec calls raw decode, so alphabet, checksum, case, and length
/// rejection are pinned at canonicalNpub/npubBytes, the npub seam Buzz
/// actually uses.
final class Bech32Tests: XCTestCase {
  /// Hex public keys with their published npub equivalents.
  static let npubVectors: [(hex: String, npub: String)] = [
    // nostr-rs 0.44 key test: aa4fc866… ↔ npub14f8usejl…qqh9nsy.
    (
      "aa4fc8665f5696e33db7e1a572e3b0f5b3d615837b0f362dcb1c8068b098c7b4",
      "npub14f8usejl26twx0dhuxjh9cas7keav9vr0v8nvtwtrjqx3vycc76qqh9nsy"
    ),
    // The NIP-19 spec's example profile key.
    (
      "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d",
      "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6"
    ),
    // Degenerate key material still encodes (and exercises 5-bit padding).
    (String(repeating: "00", count: 32), "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqzqujme"),
  ]

  func testNpubEncodingMatchesKnownVectors() throws {
    for vector in Self.npubVectors {
      let bytes = try XCTUnwrap(VerifiedNostrEvent.hexBytes(vector.hex))
      XCTAssertEqual(bytes.count, 32)
      XCTAssertEqual(Bech32.npub(from: bytes), vector.npub, "hex: \(vector.hex)")
      XCTAssertEqual(Bech32.canonicalNpub(from: vector.hex), vector.npub, "hex: \(vector.hex)")
    }
  }

  func testCanonicalNpubValidatesAndCanonicalizesNpubInputs() throws {
    for vector in Self.npubVectors {
      let npub = try XCTUnwrap(Bech32.canonicalNpub(from: vector.npub))
      XCTAssertEqual(npub, vector.npub, "npub: \(vector.npub)")
      XCTAssertEqual(
        Bech32.npubBytes(from: npub).map(VerifiedNostrEvent.hex), vector.hex,
        "npub: \(vector.npub)")
    }
    // Uppercase bech32 is valid per BIP-173; canonical output is lowercase.
    XCTAssertEqual(
      Bech32.canonicalNpub(
        from: "NPUB14F8USEJL26TWX0DHUXJH9CAS7KEAV9VR0V8NVTWTRJQX3VYCC76QQH9NSY"),
      "npub14f8usejl26twx0dhuxjh9cas7keav9vr0v8nvtwtrjqx3vycc76qqh9nsy")
    // Letter case within a hex key stays valid input; digits are caseless,
    // so mixed-case letters still canonicalize to the lowercase npub.
    let mixedCaseHex = String(
      Self.npubVectors[0].hex.enumerated().map {
        $0.offset.isMultiple(of: 2) ? $0.element : Character($0.element.uppercased())
      })
    XCTAssertEqual(Bech32.canonicalNpub(from: mixedCaseHex), Self.npubVectors[0].npub)
  }

  func testCanonicalNpubRejectsInvalidAndLookalikeKeys() {
    let rejected = [
      // Junk and empty payloads.
      "",
      "author-pubkey",
      String(repeating: "a", count: 63),
      "0",
      // Hex that is not a 32-byte key.
      String(repeating: "ab", count: 31),
      String(repeating: "ab", count: 33),
      // Signed radix-16 chunks ("+a"→10, "-0"→0) parse via UInt8(_:radix:)
      // but are not literal ASCII hex digits, so never 32-byte keys.
      String(repeating: "+a", count: 32),
      String(repeating: "+A", count: 32),
      String(repeating: "-0", count: 32),
      // Npub-shaped payload with a character outside the bech32 alphabet
      // ("b" never appears in the charset).
      "npub1bqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqzqujme",
      // Valid npub with the checksum's final character mutated.
      "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqzqujma",
      // Mixed case is never valid bech32.
      "Npub14f8usejl26twx0dhuxjh9cas7keav9vr0v8nvtwtrjqx3vycc76qqh9nsy",
      // Valid checksum but the wrong payload length for a key.
      "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqk7h3rf",
      "npub1llllllllllllllllllllllllllllllllllllllllllllllllllll7w6tc2n",
      // Valid bech32 under a different human-readable part.
      "nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqwkhnav",
    ]
    for string in rejected {
      XCTAssertNil(Bech32.canonicalNpub(from: string), "expected rejection: \(string)")
      XCTAssertNil(Bech32.npubBytes(from: string), "expected rejection: \(string)")
    }
  }

  func testNpubRejectsNon32ByteKeys() {
    XCTAssertNil(Bech32.npub(from: [UInt8](repeating: 0, count: 16)))
    XCTAssertNil(Bech32.npub(from: [UInt8](repeating: 0xff, count: 33)))
    XCTAssertNil(Bech32.npub(from: []))
  }
}
