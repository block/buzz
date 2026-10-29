import 'dart:math';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

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
  test('native signing interoperates with the pure-Dart nostr stack', () {
    final rng = Random(8446);
    for (var i = 0; i < 64; i++) {
      final secretKey = _randomSecretKey(rng);
      final tags = [
        ['h', 'channel-$i'],
        ['p', 'a' * 64, 'wss://relay.example', 'mention'],
      ];
      final content = 'message $i: ünïcødé "quotes" \\ \n newline 🐝';

      final event = signNostrEvent(
        kind: 9,
        content: content,
        secretKey: secretKey,
        createdAt: 1700000000 + i,
        tags: tags,
      );

      // Same public key and id as the pure-Dart builder.
      final reference = nostr.Event.from(
        kind: 9,
        content: content,
        secretKey: secretKey,
        createdAt: 1700000000 + i,
        tags: tags,
      );
      expect(event.pubkey, reference.pubkey);
      expect(derivePublicKeyHex(secretKey), reference.pubkey);
      expect(event.id, reference.id);

      // The pure-Dart verifier accepts the native signature and id.
      expect(
        () => nostr.Event.fromMap(event.toMap(), verify: true),
        returnsNormally,
      );
    }
  });

  test('a caller-supplied public key is used as given', () {
    final secretKey = _randomSecretKey(Random(1));
    final pubkey = nostr.Keys(secretKey).public;
    final event = signNostrEvent(
      kind: 1,
      content: '',
      secretKey: secretKey,
      pubkey: pubkey,
    );
    expect(event.pubkey, pubkey);
    expect(event.isValid(), isTrue);
  });

  test('rejects malformed secret keys', () {
    for (final bad in ['', 'zz' * 32, 'ab' * 31, '+${'1' * 63}', '0' * 64]) {
      expect(
        () => signNostrEvent(kind: 1, content: '', secretKey: bad),
        throwsArgumentError,
        reason: bad,
      );
    }
  });
}
