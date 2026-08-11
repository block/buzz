import 'package:nostr/nostr.dart' as nostr;

/// Canonical forms of one Nostr identity.
///
/// Bech32 is used at storage and presentation boundaries. The hex forms are
/// exposed only for NIP-01 wire data and cryptographic operations.
typedef NostrIdentityKeys = ({
  String nsec,
  String npub,
  String privateKeyHex,
  String publicKeyHex,
});

final _hexKeyPattern = RegExp(r'^[0-9a-fA-F]{64}$');
final _secp256k1Field = BigInt.parse(
  'fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f',
  radix: 16,
);

String _canonicalBech32(String value, String prefix) {
  final hasLower = RegExp('[a-z]').hasMatch(value);
  final hasUpper = RegExp('[A-Z]').hasMatch(value);
  if (hasLower && hasUpper) throw const FormatException('Mixed-case bech32');
  final canonical = value.toLowerCase();
  if (!canonical.startsWith(prefix)) {
    throw const FormatException('Unexpected bech32 prefix');
  }
  return canonical;
}

bool _isValidXOnlyPublicKey(String value) {
  final x = BigInt.parse(value, radix: 16);
  if (x >= _secp256k1Field) return false;
  final ySquared = (x * x * x + BigInt.from(7)) % _secp256k1Field;
  final y = ySquared.modPow(
    (_secp256k1Field + BigInt.one) >> 2,
    _secp256k1Field,
  );
  return (y * y) % _secp256k1Field == ySquared;
}

/// Parses a canonical bech32 secret key and derives all identity forms.
///
/// Unlike [nostr.Keys], this deliberately rejects a raw hex secret. Raw
/// secrets are accepted only by [identityFromStoredSecret] for one-time local
/// migration.
NostrIdentityKeys identityFromNsec(String value) {
  final trimmed = value.trim();
  try {
    final decoded = nostr.Nip19.decode(
      payload: _canonicalBech32(trimmed, 'nsec1'),
    );
    if (decoded.prefix != nostr.Nip19Prefix.nsec ||
        !_hexKeyPattern.hasMatch(decoded.data)) {
      throw const FormatException('Expected a valid nsec');
    }
    return _identityFromPrivateHex(decoded.data);
  } catch (_) {
    throw const FormatException('Expected a valid nsec');
  }
}

/// Parses an nsec, also accepting a legacy raw secret for local migration.
NostrIdentityKeys identityFromStoredSecret(String value) {
  final trimmed = value.trim();
  if (_hexKeyPattern.hasMatch(trimmed)) {
    return _identityFromPrivateHex(trimmed);
  }
  return identityFromNsec(trimmed);
}

NostrIdentityKeys _identityFromPrivateHex(String value) {
  try {
    final keys = nostr.Keys(value.toLowerCase());
    return (
      nsec: keys.nsec,
      npub: keys.npub,
      privateKeyHex: keys.secret.toLowerCase(),
      publicKeyHex: keys.public.toLowerCase(),
    );
  } catch (_) {
    throw const FormatException('Expected a valid Nostr secret key');
  }
}

/// Returns the canonical npub for either an npub or a 64-character public hex.
String npubFromPublicKey(String value) {
  final publicKeyHex = publicKeyHexFromInput(value, allowLegacyHex: true);
  return nostr.Bech32Entity.encode(
    prefix: nostr.Nip19Prefix.npub,
    data: publicKeyHex,
  );
}

/// Converts an npub to the lowercase hex form required by NIP-01.
///
/// [allowLegacyHex] exists for explicit compatibility boundaries such as old
/// pairing payloads and on-disk migration. New app-facing inputs should leave
/// it false.
String publicKeyHexFromInput(String value, {bool allowLegacyHex = false}) {
  final trimmed = value.trim();
  if (allowLegacyHex && _hexKeyPattern.hasMatch(trimmed)) {
    final canonical = trimmed.toLowerCase();
    if (_isValidXOnlyPublicKey(canonical)) return canonical;
    throw const FormatException('Expected a valid npub');
  }

  try {
    final decoded = nostr.Nip19.decode(
      payload: _canonicalBech32(trimmed, 'npub1'),
    );
    if (decoded.prefix != nostr.Nip19Prefix.npub ||
        !_hexKeyPattern.hasMatch(decoded.data) ||
        !_isValidXOnlyPublicKey(decoded.data)) {
      throw const FormatException('Expected a valid npub');
    }
    return decoded.data.toLowerCase();
  } catch (_) {
    throw const FormatException('Expected a valid npub');
  }
}

String? tryNpubFromPublicKey(String? value) {
  if (value == null || value.trim().isEmpty) return null;
  try {
    return npubFromPublicKey(value);
  } catch (_) {
    return null;
  }
}

String? tryPublicKeyHexFromInput(String? value, {bool allowLegacyHex = false}) {
  if (value == null || value.trim().isEmpty) return null;
  try {
    return publicKeyHexFromInput(value, allowLegacyHex: allowLegacyHex);
  } catch (_) {
    return null;
  }
}

NostrIdentityKeys? tryIdentityFromNsec(String? value) {
  if (value == null || value.trim().isEmpty) return null;
  try {
    return identityFromNsec(value);
  } catch (_) {
    return null;
  }
}

NostrIdentityKeys? tryIdentityFromStoredSecret(String? value) {
  if (value == null || value.trim().isEmpty) return null;
  try {
    return identityFromStoredSecret(value);
  } catch (_) {
    return null;
  }
}
