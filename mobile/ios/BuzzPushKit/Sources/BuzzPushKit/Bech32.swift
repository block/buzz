import Foundation

/// Minimal bech32 (BIP-173) codec backing NIP-19 npub sender labels.
///
/// Only what push notification presentation needs is implemented: encoding
/// 32-byte public keys as npub and validating npub inputs well enough to
/// canonicalize them. Segwit addresses and bech32m are out of scope; NIP-19
/// uses the original bech32 checksum.
enum Bech32 {
  /// Bech32 data charset from BIP-173, indexed by 5-bit value.
  private static let charset: [Character] = Array("qpzry9x8gf2tvdw0s3jn54khce6mua7l")
  /// Bech32 charset lookup, built once for decoding.
  private static let valueByCharacter: [Character: UInt8] = Dictionary(
    uniqueKeysWithValues: charset.enumerated().map { ($1, UInt8($0)) })
  /// Generator polynomial coefficients from BIP-173.
  private static let generator: [UInt32] = [
    0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3,
  ]
  /// Checksum length in 5-bit values, from BIP-173.
  private static let checksumLength = 6
  /// Maximum bech32 string length, from BIP-173.
  private static let maximumLength = 90

  // MARK: Codec

  /// Encodes 5-bit values under a human-readable part (hrp), appending the
  /// bech32 checksum. Returns nil for an invalid hrp or out-of-range values.
  static func encode(hrp: String, values: [UInt8]) -> String? {
    let valuesInRange = values.allSatisfy { $0 < 32 }
    guard isPrintableASCII(hrp), valuesInRange,
      hrp.count + 1 + values.count + checksumLength <= maximumLength
    else { return nil }
    let checksum = checksum(hrp: hrp, values: values)
    return hrp + "1" + String((values + checksum).map { charset[Int($0)] })
  }

  /// Decodes a bech32 string into its hrp and 5-bit values, checksum-verified
  /// and stripped. Follows BIP-173: characters must be printable ASCII, the
  /// string must be entirely lowercase or entirely uppercase and at most 90
  /// characters long, and it must end in a valid checksum.
  static func decode(_ string: String) -> (hrp: String, values: [UInt8])? {
    guard !string.isEmpty, string.count <= maximumLength, isPrintableASCII(string)
    else { return nil }
    let lowercased = string.lowercased()
    guard lowercased == string || lowercased.uppercased() == string,
      let separator = lowercased.lastIndex(of: "1"),
      separator != lowercased.startIndex
    else { return nil }
    let hrp = String(lowercased[..<separator])
    var allValues: [UInt8] = []
    for character in lowercased[lowercased.index(after: separator)...] {
      guard let value = valueByCharacter[character] else { return nil }
      allValues.append(value)
    }
    guard allValues.count >= checksumLength,
      polymod(hrpExpanded(hrp) + allValues.map(UInt32.init)) == 1
    else { return nil }
    return (hrp, Array(allValues.dropLast(checksumLength)))
  }

  // MARK: NIP-19

  /// The npub of a 32-byte public key, or nil for any other length.
  static func npub(from bytes: [UInt8]) -> String? {
    guard bytes.count == 32,
      let values = convertBits(bytes, fromBits: 8, toBits: 5, padding: true)
    else { return nil }
    return encode(hrp: "npub", values: values)
  }

  /// The 32-byte public key encoded by a valid npub, or nil for anything
  /// else — an `npub1…` prefix alone is never trusted: the bech32 checksum
  /// must validate and the payload must decode to exactly 32 bytes.
  static func npubBytes(from string: String) -> [UInt8]? {
    guard let (hrp, values) = decode(string), hrp == "npub",
      let bytes = convertBits(values, fromBits: 5, toBits: 8, padding: false),
      bytes.count == 32
    else { return nil }
    return bytes
  }

  /// The canonical npub for a public key supplied as exactly 64 ASCII hex
  /// digits or as an existing npub (including the uppercase bech32 form,
  /// which BIP-173 decoders must accept). Returns nil for anything else;
  /// callers must fall back to a neutral label rather than raw key material.
  static func canonicalNpub(from identifier: String) -> String? {
    if isHexKey(identifier),
      let bytes = VerifiedNostrEvent.hexBytes(identifier.lowercased()), bytes.count == 32
    {
      return npub(from: bytes)
    }
    return npubBytes(from: identifier).flatMap { npub(from: $0) }
  }

  // MARK: Internals

  /// Whether `value` is exactly 64 ASCII hex digits (0-9, A-F, a-f).
  ///
  /// `VerifiedNostrEvent.hexBytes` parses pairs with `UInt8(_:radix: 16)`,
  /// which also accepts a leading sign — "+a" parses as 10 and "-0" as 0 —
  /// so strings like "+a"×32 would otherwise decode to 32 bytes. The hex
  /// branch of `canonicalNpub` gates on this literal key shape before any
  /// hex parsing or allocation; every other input must arrive as a strictly
  /// checksummed npub.
  private static func isHexKey(_ value: String) -> Bool {
    guard value.count == 64 else { return false }
    return value.allSatisfy { character in
      guard let ascii = character.asciiValue else { return false }
      return (48...57).contains(ascii)  // 0-9
        || (65...70).contains(ascii)  // A-F
        || (97...102).contains(ascii)  // a-f
    }
  }

  private static func isPrintableASCII(_ string: String) -> Bool {
    !string.isEmpty && string.allSatisfy { (33...126).contains($0.asciiValue ?? 0) }
  }

  private static func checksum(hrp: String, values: [UInt8]) -> [UInt8] {
    let expanded =
      hrpExpanded(hrp) + values.map(UInt32.init)
      + [UInt32](repeating: 0, count: checksumLength)
    let polymod = polymod(expanded) ^ 1
    return (0..<checksumLength).map { index in
      let shift = 5 * (checksumLength - 1 - index)
      return UInt8((polymod >> shift) & 31)
    }
  }

  private static func polymod(_ values: [UInt32]) -> UInt32 {
    var accumulator: UInt32 = 1
    for value in values {
      let top = accumulator >> 25
      accumulator = ((accumulator & 0x1ffffff) << 5) ^ value
      for (index, coefficient) in generator.enumerated() where (top >> index) & 1 == 1 {
        accumulator ^= coefficient
      }
    }
    return accumulator
  }

  private static func hrpExpanded(_ hrp: String) -> [UInt32] {
    let scalars = hrp.unicodeScalars.map { $0.value }
    return scalars.map { $0 >> 5 } + [0] + scalars.map { $0 & 31 }
  }

  /// Regroups a byte string between bit widths, as in BIP-173's `convertbits`.
  /// With padding disabled, a nonzero remainder is rejected instead of padded.
  private static func convertBits(
    _ bytes: [UInt8], fromBits: Int, toBits: Int, padding: Bool
  ) -> [UInt8]? {
    var accumulator: UInt32 = 0
    var accumulated = 0
    var result: [UInt8] = []
    result.reserveCapacity((bytes.count * fromBits + toBits - 1) / toBits)
    let maxValue: UInt32 = (1 << toBits) - 1
    let maxAccumulator: UInt32 = (1 << (fromBits + toBits - 1)) - 1
    let maxInput: Int = 1 << fromBits
    for byte in bytes {
      guard Int(byte) < maxInput else { return nil }
      accumulator = ((accumulator << fromBits) | UInt32(byte)) & maxAccumulator
      accumulated += fromBits
      while accumulated >= toBits {
        accumulated -= toBits
        result.append(UInt8((accumulator >> accumulated) & maxValue))
      }
    }
    if padding {
      if accumulated > 0 {
        result.append(UInt8((accumulator << (toBits - accumulated)) & maxValue))
      }
    } else if accumulated >= fromBits
      || ((accumulator << (toBits - accumulated)) & maxValue) != 0
    {
      return nil
    }
    return result
  }
}
