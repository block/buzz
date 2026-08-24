import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'package:buzz/shared/relay/event_signing.dart';

// Two distinct keys, so the cache is exercised in both directions.
const _keyA =
    '5ee1c8000ab28edd64d74a7d951ac2dd559814887b1b9e1ac7c5f89e96125c12';
const _keyB =
    '0000000000000000000000000000000000000000000000000000000000000003';

void main() {
  group('pubkeyForPrivkey', () {
    test('matches the key the nostr package derives', () {
      expect(pubkeyForPrivkey(_keyA), nostr.Schnorr.derivePublicKey(_keyA));
      expect(pubkeyForPrivkey(_keyB), nostr.Schnorr.derivePublicKey(_keyB));
    });

    test('is stable across repeat calls and key switches', () {
      final first = pubkeyForPrivkey(_keyA);
      expect(pubkeyForPrivkey(_keyA), first);
      final other = pubkeyForPrivkey(_keyB);
      expect(other, isNot(first));
      // Switching back must re-derive rather than return the stale slot.
      expect(pubkeyForPrivkey(_keyA), first);
    });
  });

  group('supplying the pubkey to Event.from', () {
    // The point of the change: skipping Event.from's own derive must not alter
    // the wire bytes. Asserted rather than argued, because it is crypto.
    test('produces an identical event to letting Event.from derive it', () {
      const createdAt = 1771000000;
      final tags = [
        ['h', 'a-channel'],
      ];

      final derived = nostr.Event.from(
        kind: 9,
        content: 'equivalence',
        tags: tags,
        secretKey: _keyA,
        createdAt: createdAt,
      );
      final supplied = nostr.Event.from(
        kind: 9,
        content: 'equivalence',
        tags: tags,
        secretKey: _keyA,
        createdAt: createdAt,
        pubkey: pubkeyForPrivkey(_keyA),
      );

      expect(supplied.id, derived.id);
      expect(supplied.pubkey, derived.pubkey);
      expect(supplied.isValid(), isTrue);
      expect(derived.isValid(), isTrue);
    });

    test('the derived pubkey is already lower-case hex', () {
      // Event.from lower-cases only in the branch that derives, so a supplied
      // key reaches the serialized `pubkey` field verbatim.
      final pubkey = pubkeyForPrivkey(_keyA);
      expect(pubkey, pubkey.toLowerCase());
      expect(pubkey, matches(RegExp(r'^[0-9a-f]{64}$')));
    });
  });
}
