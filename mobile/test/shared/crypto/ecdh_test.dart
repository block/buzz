import 'dart:math';
import 'dart:typed_data';

import 'package:buzz/shared/crypto/ecdh.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/ecc/curves/secp256k1.dart';

/// The previous pure-Dart ECDH, kept as a reference implementation.
Uint8List _referenceEcdh(String privateKeyHex, String publicKeyHex) {
  final params = ECCurve_secp256k1();
  final compressed = Uint8List(33)
    ..[0] = 0x02
    ..setRange(1, 33, hexToBytes(publicKeyHex));
  final shared =
      params.curve.decodePoint(compressed)! *
      BigInt.parse(privateKeyHex, radix: 16);
  return hexToBytes(
    shared!.x!.toBigInteger()!.toRadixString(16).padLeft(64, '0'),
  );
}

String _randomSecretKey(Random rng) {
  while (true) {
    final hex = List.generate(
      32,
      (_) => rng.nextInt(256).toRadixString(16).padLeft(2, '0'),
    ).join();
    try {
      nostr.Keys(hex);
      return hex;
    } on Object {
      // Out of range for secp256k1; draw again.
    }
  }
}

void main() {
  test('matches the previous pure-Dart ECDH for random key pairs', () {
    final rng = Random(44);
    for (var i = 0; i < 32; i++) {
      final secretKey = _randomSecretKey(rng);
      final peer = nostr.Keys(_randomSecretKey(rng)).public;
      expect(
        ecdhSharedSecret(secretKey, peer),
        _referenceEcdh(secretKey, peer),
      );
    }
  });

  test('NIP-44 v2 conversation key vector', () {
    expect(
      bytesToHex(
        getConversationKey(
          '0000000000000000000000000000000000000000000000000000000000000001',
          'c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5',
        ),
      ),
      'c41c775356fd92eadc63ff5a0dc1da211b268cbea22316767095b2871ea1412d',
    );
  });

  test('rejects invalid keys', () {
    final valid = nostr.Keys('1' * 64).public;
    for (final (secretKey, publicKey) in [
      ('0' * 64, valid),
      ('1' * 64, '0' * 64),
      ('1' * 62, valid),
      ('1' * 64, valid.substring(2)),
    ]) {
      expect(
        () => ecdhSharedSecret(secretKey, publicKey),
        throwsArgumentError,
        reason: '$secretKey / $publicKey',
      );
    }
  });
}
