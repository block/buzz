import 'dart:math' as math;
import 'dart:typed_data';

import 'package:buzz_secp256k1/buzz_secp256k1.dart' as secp256k1;

/// Compute secp256k1 ECDH shared secret (raw x-coordinate, unhashed).
///
/// Both [privateKeyHex] and [publicKeyHex] are 64-char lowercase hex strings
/// (32 bytes each). The public key is an x-only BIP-340 key, lifted to even Y.
///
/// Returns the 32-byte x-coordinate of the shared point, computed in constant
/// time by libsecp256k1. Throws [ArgumentError] when either key is invalid.
Uint8List ecdhSharedSecret(String privateKeyHex, String publicKeyHex) {
  if (privateKeyHex.length != 64 || publicKeyHex.length != 64) {
    throw ArgumentError('ECDH keys must be 64 hex characters');
  }
  final secretKey = hexToBytes(privateKeyHex);
  try {
    return secp256k1.ecdhX(
      secretKey: secretKey,
      publicKey: hexToBytes(publicKeyHex),
    );
  } finally {
    secretKey.fillRange(0, secretKey.length, 0);
  }
}

/// Generate secure random bytes.
Uint8List secureRandomBytes(int length) {
  final rng = math.Random.secure();
  final data = Uint8List(length);
  for (var i = 0; i < length; i++) {
    data[i] = rng.nextInt(256);
  }
  return data;
}

// ── Hex helpers (shared across crypto modules) ──────────────────────────────

Uint8List hexToBytes(String hex) {
  final result = Uint8List(hex.length ~/ 2);
  for (var i = 0; i < result.length; i++) {
    result[i] = int.parse(hex.substring(i * 2, i * 2 + 2), radix: 16);
  }
  return result;
}

String bytesToHex(Uint8List bytes) {
  return bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
}
