import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:buzz_secp256k1/buzz_secp256k1.dart';
import 'package:test/test.dart';

Uint8List _hex(String s) => Uint8List.fromList(
  List.generate(
    s.length ~/ 2,
    (i) => int.parse(s.substring(i * 2, i * 2 + 2), radix: 16),
  ),
);

/// Official vectors from bitcoin/bips bip-0340/test-vectors.csv.
void main() {
  final rows = File('test/bip340_vectors.csv')
      .readAsLinesSync()
      .skip(1)
      .where((line) => line.isNotEmpty)
      .map((line) => line.split(','))
      .toList();

  for (final row in rows) {
    final [index, secret, pub, aux, msg, sig, result, ...comment] = row;
    final label = 'vector $index ${comment.join(',')}'.trim();

    // This API only accepts 32-byte messages (NIP-01 event ids). The
    // variable-length vectors exercise a code path it does not expose.
    if (msg.length != 64) continue;

    if (secret.isNotEmpty) {
      test('$label signs', () {
        expect(
          schnorrSign(
            message: _hex(msg),
            secretKey: _hex(secret),
            auxRand: _hex(aux),
          ),
          _hex(sig),
        );
        expect(xOnlyPublicKey(_hex(secret)), _hex(pub));
      });
    }

    test('$label verifies as $result', () {
      expect(
        schnorrVerify(
          signature: _hex(sig),
          message: _hex(msg),
          publicKey: _hex(pub),
        ),
        result == 'TRUE',
      );
    });
  }

  test(
    'the first signature in a fresh isolate uses the given auxRand',
    () async {
      // Vector 1 has nonzero auxRand, so a signature made with clobbered aux
      // input would differ. Run it as the isolate's first native call.
      final [_, secret, _, aux, msg, sig, ...] = rows[1];
      final first = await Isolate.run(
        () => schnorrSign(
          message: _hex(msg),
          secretKey: _hex(secret),
          auxRand: _hex(aux),
        ),
      );
      expect(first, _hex(sig));
    },
  );

  test('covers every 32-byte-message vector', () {
    expect(rows.where((r) => r[4].length == 64).length, 15);
  });

  test('rejects wrong-length inputs loudly', () {
    expect(
      () => schnorrVerify(
        signature: Uint8List(63),
        message: Uint8List(32),
        publicKey: Uint8List(32),
      ),
      throwsArgumentError,
    );
    expect(() => xOnlyPublicKey(Uint8List(32)), throwsArgumentError);
  });

  test('ecdh is symmetric and returns raw x', () {
    final a = _hex('0' * 63 + '2');
    final b = _hex('0' * 63 + '3');
    final ab = ecdhX(secretKey: a, publicKey: xOnlyPublicKey(b));
    final ba = ecdhX(secretKey: b, publicKey: xOnlyPublicKey(a));
    expect(ab, ba);
    // 6*G has x = fff97bd5...; raw x, not SHA256 of the point.
    expect(
      ab,
      _hex('fff97bd5755eeea420453a14355235d382f6472f8568a18b2f057a1460297556'),
    );
  });
}
