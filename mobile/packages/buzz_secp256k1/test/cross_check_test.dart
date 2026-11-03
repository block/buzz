import 'dart:isolate';
import 'dart:math';
import 'dart:typed_data';

import 'package:buzz_secp256k1/buzz_secp256k1.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/ecc/curves/secp256k1.dart';
import 'package:test/test.dart';

Uint8List _hex(String s) => Uint8List.fromList(
  List.generate(
    s.length ~/ 2,
    (i) => int.parse(s.substring(i * 2, i * 2 + 2), radix: 16),
  ),
);

String _toHex(Uint8List b) =>
    b.map((x) => x.toRadixString(16).padLeft(2, '0')).join();

// secp256k1 field prime p and group order n.
const _p = 'fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f';
const _n = 'fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141';

Uint8List _pointycastleEcdh(Uint8List sk, Uint8List pub) {
  final curve = ECCurve_secp256k1().curve;
  final point = curve.decodePoint([0x02, ...pub])!;
  final shared = (point * BigInt.parse(_toHex(sk), radix: 16))!;
  return _hex(shared.x!.toBigInteger()!.toRadixString(16).padLeft(64, '0'));
}

void main() {
  final rng = Random(1);
  Uint8List message() =>
      Uint8List.fromList(List.generate(32, (_) => rng.nextInt(256)));

  test('agrees with the pure-Dart stack on random keys', () {
    for (var i = 0; i < 25; i++) {
      final a = nostr.Keys.generate();
      final b = nostr.Keys.generate();
      final msg = message();

      expect(_toHex(xOnlyPublicKey(_hex(a.secret))), a.public);

      final nativeSig = schnorrSign(message: msg, secretKey: _hex(a.secret));
      expect(
        nostr.Schnorr.verify(
          publicKey: a.public,
          message: _toHex(msg),
          signature: _toHex(nativeSig),
        ),
        isTrue,
      );

      final dartSig = nostr.Schnorr.sign(
        secretKey: a.secret,
        message: _toHex(msg),
      );
      expect(
        schnorrVerify(
          signature: _hex(dartSig),
          message: msg,
          publicKey: _hex(a.public),
        ),
        isTrue,
      );

      expect(
        ecdhX(secretKey: _hex(a.secret), publicKey: _hex(b.public)),
        _pointycastleEcdh(_hex(a.secret), _hex(b.public)),
      );
    }
  });

  test('rejects secret keys that are zero or not below the order', () {
    for (final bad in ['0' * 64, _n, 'f' * 64]) {
      expect(() => xOnlyPublicKey(_hex(bad)), throwsArgumentError);
      expect(
        () => schnorrSign(message: message(), secretKey: _hex(bad)),
        throwsArgumentError,
      );
      expect(
        () => ecdhX(
          secretKey: _hex(bad),
          publicKey: xOnlyPublicKey(_hex('${'0' * 63}1')),
        ),
        throwsArgumentError,
      );
    }
  });

  test('rejects public keys that are not on the curve', () {
    final sk = _hex('${'0' * 63}1');
    // x = 5 has no curve point; x = p is out of range.
    for (final bad in ['${'0' * 63}5', _p]) {
      expect(
        () => ecdhX(secretKey: sk, publicKey: _hex(bad)),
        throwsArgumentError,
      );
      expect(
        schnorrVerify(
          signature: Uint8List(64),
          message: Uint8List(32),
          publicKey: _hex(bad),
        ),
        isFalse,
      );
    }
  });

  test('works from short-lived isolates', () async {
    final keys = nostr.Keys.generate();
    final msg = message();
    for (var i = 0; i < 3; i++) {
      final sig = await Isolate.run(
        () => schnorrSign(message: msg, secretKey: _hex(keys.secret)),
      );
      final ok = await Isolate.run(
        () => schnorrVerify(
          signature: sig,
          message: msg,
          publicKey: _hex(keys.public),
        ),
      );
      expect(ok, isTrue);
    }
  });
}
