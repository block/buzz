import 'dart:typed_data';

import 'package:buzz_secp256k1/buzz_secp256k1.dart' as secp256k1;
import 'package:nostr/nostr.dart' as nostr;

final _hex64 = RegExp(r'^[0-9a-fA-F]{64}$');

Uint8List _decodeSecretKey(String secretKeyHex) {
  if (!_hex64.hasMatch(secretKeyHex)) {
    throw ArgumentError('Secret key must be 64 hex characters');
  }
  return _decodeHex(secretKeyHex);
}

Uint8List _decodeHex(String value) => Uint8List.fromList([
  for (var i = 0; i < value.length; i += 2)
    int.parse(value.substring(i, i + 2), radix: 16),
]);

String _encodeHex(Uint8List bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

/// Returns the lowercase x-only public key hex for [secretKeyHex], computed
/// by libsecp256k1.
String derivePublicKeyHex(String secretKeyHex) =>
    _encodeHex(secp256k1.xOnlyPublicKey(_decodeSecretKey(secretKeyHex)));

/// Builds and signs a Nostr event with libsecp256k1.
///
/// Drop-in replacement for `nostr.Event.from(..., verify: false)`: the event
/// id is computed by the same `nostr` canonical serialization, and only the
/// elliptic-curve work (public key derivation and BIP-340 signing) runs
/// natively. Pass [pubkey] when the caller already knows it to skip the
/// derivation.
nostr.Event signNostrEvent({
  required int kind,
  required String content,
  required String secretKey,
  int? createdAt,
  List<List<String>> tags = const [],
  String? pubkey,
}) {
  final secretKeyBytes = _decodeSecretKey(secretKey);
  final unsigned = nostr.Event.unsigned(
    pubkey: pubkey ?? _encodeHex(secp256k1.xOnlyPublicKey(secretKeyBytes)),
    kind: kind,
    content: content,
    createdAt: createdAt,
    tags: tags,
  );
  final signature = secp256k1.schnorrSign(
    message: _decodeHex(unsigned.id),
    secretKey: secretKeyBytes,
  );
  return unsigned.copyWith(sig: _encodeHex(signature));
}

final _hex128 = RegExp(r'^[0-9a-fA-F]{128}$');

/// Verifies a BIP-340 Schnorr [signature] (128 hex) over the 32-byte
/// [messageHex] digest for the x-only [publicKey] with libsecp256k1.
///
/// Returns false for malformed hex as well as for invalid signatures.
bool verifySchnorrHex({
  required String publicKey,
  required String messageHex,
  required String signature,
}) {
  if (!_hex64.hasMatch(publicKey) ||
      !_hex64.hasMatch(messageHex) ||
      !_hex128.hasMatch(signature)) {
    return false;
  }
  return secp256k1.schnorrVerify(
    signature: _decodeHex(signature),
    message: _decodeHex(messageHex),
    publicKey: _decodeHex(publicKey),
  );
}

/// Signs a 32-byte [messageHex] digest with [secretKey] and returns the
/// 128-hex BIP-340 signature, computed by libsecp256k1.
String signSchnorrHex({required String secretKey, required String messageHex}) {
  if (!_hex64.hasMatch(messageHex)) {
    throw ArgumentError('Message must be 64 hex characters');
  }
  return _encodeHex(
    secp256k1.schnorrSign(
      message: _decodeHex(messageHex),
      secretKey: _decodeSecretKey(secretKey),
    ),
  );
}

/// Parses [payload] like `nostr.Event.fromJson` and verifies its timestamp,
/// id and signature like `verify: true` does, with the signature check in
/// libsecp256k1.
///
/// Throws the same `nostr` exception types as the pure-Dart path.
nostr.Event verifiedNostrEventFromJson(String payload) {
  final event = nostr.Event.fromJson(payload, verify: false);
  if (event.createdAt <= 0 || event.createdAt >= 253402300800) {
    throw nostr.EventValidationException(
      'created_at ${event.createdAt} is not a valid Unix timestamp (seconds)',
      nostr.EventValidationReason.invalidTimestamp,
    );
  }
  if (event.id != event.getEventId()) {
    throw const nostr.EventValidationException(
      'event id mismatch',
      nostr.EventValidationReason.idMismatch,
    );
  }
  if (!_hex64.hasMatch(event.pubkey) || !_hex128.hasMatch(event.sig)) {
    throw const nostr.EventValidationException(
      'malformed pubkey or signature',
      nostr.EventValidationReason.malformedSignature,
    );
  }
  if (!verifySchnorrHex(
    publicKey: event.pubkey,
    messageHex: event.id,
    signature: event.sig,
  )) {
    throw const nostr.EventValidationException(
      'Schnorr signature invalid',
      nostr.EventValidationReason.invalidSignature,
    );
  }
  return event;
}
