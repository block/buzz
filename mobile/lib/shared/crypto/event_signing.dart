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
