import XCTest
@testable import BuzzPushKit

/// Bech32 codec tests against known independent vectors: the NIP-19 example
/// key, the nostr crate 0.44 test suite, and the test vectors published with
/// BIP-173 itself.
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
    // secp256k1 generator points, used as sender keys by the resolver tests.
    (
      "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
      "npub10xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqpkge6d"
    ),
    (
      "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5",
      "npub1ccz8l9zpa47k6vz9gphftsrumpw80rjt3nhnefat4symjhrsnmjs38mnyd"
    ),
    // Degenerate key material still encodes.
    (String(repeating: "00", count: 32), "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqzqujme"),
    (String(repeating: "ff", count: 32), "npub1lllllllllllllllllllllllllllllllllllllllllllllllllllsq7lrjw"),
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

  func testDecodeAcceptsPublishedBip173Vectors() {
    let valid = [
      "A12UEL5L",
      "a12uel5l",
      "an83characterlonghumanreadablepartthatcontainsthenumber1andtheexcludedcharactersbio1tt5tgs",
      "abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxw",
      "11qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqc8247j",
      "split1checkupstagehandshakeupstreamerranterredcaperred2y9e3w",
      "?1ezyfcl",
    ]
    for string in valid {
      XCTAssertNotNil(Bech32.decode(string), "expected valid: \(string)")
    }
    // Uppercase forms decode to the lowercase canonical hrp.
    XCTAssertEqual(Bech32.decode("A12UEL5L")?.hrp, "a")
    XCTAssertEqual(Bech32.decode("A12UEL5L")?.values, [])
  }

  func testDecodeRejectsPublishedBip173Vectors() {
    let invalid = [
      "pzry9x0s0muk",  // no separator character
      "1pzry9x0s0muk",  // empty hrp
      "x1b4n0q5v",  // invalid data character
      "li1dgmt3",  // too short checksum
      "A1G7SGD8",  // checksum calculated with uppercase form of hrp
      "10a06t8",  // empty hrp
      "1qzzf3qj",  // empty hrp
      "abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxx",  // bad checksum
    ]
    for string in invalid {
      XCTAssertNil(Bech32.decode(string), "expected invalid: \(string)")
    }
  }

  func testDecodeRejectsNonPrintableAndNonASCIICharacters() {
    XCTAssertNil(Bech32.decode("a\u{1f}1b4n0q5v"))
    XCTAssertNil(Bech32.decode("a1b4n0q5v "))
    XCTAssertNil(Bech32.decode("Ω1ezyfcl"))
    XCTAssertNil(Bech32.decode("a1özyfcl"))
  }
}
