import 'dart:convert';
import 'dart:typed_data';

import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_manager.dart';
import 'package:buzz/shared/read_state/read_state_storage.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

const _relayUrl = 'https://relay.example';

/// A crypto whose cipher work is counted: [decrypts] is how many times the
/// real NIP-44 decrypt ran.
class _CountingCrypto {
  _CountingCrypto(nostr.Keys keys, {int? maxRetainedLength}) {
    final key = _key(keys);
    crypto = ReadStateCrypto.forTesting(
      key,
      decrypt: (conversationKey, ciphertext) {
        decrypts++;
        return nip44Decrypt(conversationKey, ciphertext);
      },
      maxRetainedLength:
          maxRetainedLength ?? ReadStateCrypto.defaultMaxRetainedLength,
    );
  }

  late final ReadStateCrypto crypto;
  int decrypts = 0;
}

void main() {
  late nostr.Keys keys;
  setUp(() => keys = nostr.Keys.generate());

  group('ReadStateCrypto memo', () {
    test('decrypting the same ciphertext twice runs the cipher once', () {
      final counting = _CountingCrypto(keys);
      // Another device's blob: this instance never encrypted it.
      final foreign = ReadStateCrypto.forTesting(_key(keys)).encrypt('{"a":1}');

      expect(counting.crypto.decrypt(foreign), '{"a":1}');
      expect(counting.crypto.decrypt(foreign), '{"a":1}');
      expect(counting.decrypts, 1);
    });

    test('decrypting a ciphertext it produced runs the cipher zero times', () {
      final counting = _CountingCrypto(keys);
      final ciphertext = counting.crypto.encrypt('{"own":2}');

      expect(counting.crypto.decrypt(ciphertext), '{"own":2}');
      expect(counting.decrypts, 0);
    });

    test('a failed decrypt throws every time and is not memoized', () {
      final counting = _CountingCrypto(keys);
      final ciphertext = counting.crypto.encrypt('{"own":2}');
      // Flip the last base64 characters: the MAC no longer matches.
      final tampered = '${ciphertext.substring(0, ciphertext.length - 4)}AAAA';

      expect(() => counting.crypto.decrypt(tampered), throwsA(anything));
      expect(() => counting.crypto.decrypt(tampered), throwsA(anything));
      expect(counting.decrypts, 2);
    });

    test('the bound evicts the oldest entries and caps retained memory', () {
      const bound = 3000;
      final counting = _CountingCrypto(keys, maxRetainedLength: bound);
      final plain = 'x' * 400;
      final ciphertexts = <String>[
        for (var i = 0; i < 20; i++) counting.crypto.encrypt('$i$plain'),
      ];

      expect(counting.crypto.retainedLength, lessThanOrEqualTo(bound));
      expect(counting.crypto.retainedLength, greaterThan(0));

      // Newest is retained, oldest was evicted and decrypts again.
      counting.crypto.decrypt(ciphertexts.last);
      expect(counting.decrypts, 0);
      expect(counting.crypto.decrypt(ciphertexts.first), '0$plain');
      expect(counting.decrypts, 1);
      expect(counting.crypto.retainedLength, lessThanOrEqualTo(bound));
    });

    test('replaying 27 full slots twice decrypts each slot once', () {
      // A measured account: 27 slots near the 40 KiB plaintext budget, all
      // replayed on every resume. Their total (about 2.6M code units) is
      // more than an oldest-first memo of 2M can hold, which made every
      // replay miss.
      final counting = _CountingCrypto(keys);
      final otherDevice = ReadStateCrypto.forTesting(_key(keys));
      final slots = <String>[
        for (var i = 0; i < 27; i++)
          otherDevice.encrypt('$i${'z' * (readStatePlaintextBytes - 8)}'),
      ];

      for (var pass = 0; pass < 2; pass++) {
        for (final ciphertext in slots) {
          counting.crypto.decrypt(ciphertext);
        }
      }
      expect(counting.decrypts, slots.length);
    });

    test('a hit keeps an entry ahead of unused ones for eviction', () {
      const bound = 3000;
      final counting = _CountingCrypto(keys, maxRetainedLength: bound);
      final plain = 'x' * 400;
      final live = counting.crypto.encrypt('live$plain');
      // Our own superseded blobs: each publish adds one and never reads it.
      for (var i = 0; i < 20; i++) {
        counting.crypto.encrypt('$i$plain');
        expect(counting.crypto.decrypt(live), 'live$plain');
      }
      expect(counting.decrypts, 0);
    });

    test('a blob larger than the whole bound is not retained', () {
      final counting = _CountingCrypto(keys, maxRetainedLength: 100);
      final ciphertext = counting.crypto.encrypt('y' * 200);

      expect(counting.crypto.retainedLength, 0);
      expect(counting.crypto.decrypt(ciphertext), 'y' * 200);
      expect(counting.decrypts, 1);
    });
  });

  group('ReadStateManager decrypt work', () {
    test('echo and later pre-publish fetches do not re-decrypt', () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final counting = _CountingCrypto(keys);
      final otherDevice = ReadStateCrypto.forTesting(_key(keys));
      final session = _CapturingRelaySession();
      final relay = _RecordingSignedEventRelay();
      final manager = ReadStateManager(
        pubkey: keys.public,
        relayUrl: _relayUrl,
        prefs: prefs,
        crypto: counting.crypto,
        relaySession: session,
        signedEventRelay: relay,
        remoteEnabled: true,
        onChanged: () {},
      );
      final slotId = ReadStateStorage(
        prefs,
      ).getOrCreateSlotId(keys.public, _relayUrl);
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;

      NostrEvent event(String content, String id, int createdAt, String slot) =>
          NostrEvent(
            id: id,
            pubkey: keys.public,
            createdAt: createdAt,
            kind: EventKind.readState,
            tags: [
              ['d', '$readStateDTagPrefix$slot'],
              ['t', 'read-state'],
            ],
            content: content,
            sig: 'sig',
          );

      // Three blobs from other devices, stored before we ever publish.
      const foreignCount = 3;
      session.history = [
        for (var i = 0; i < foreignCount; i++)
          event(
            otherDevice.encrypt(
              jsonEncode(
                ReadStateBlob(
                  clientId: 'device-$i',
                  contexts: {'channel-$i': 10 + i},
                ).toJson(),
              ),
            ),
            'foreign-event-$i',
            now - 100 + i,
            'other-$i',
          ),
      ];
      await manager.initialize();
      expect(counting.decrypts, foreignCount);

      // Publish 1: its pre-fetch re-reads the foreign blobs.
      manager.markContextRead('channel-a', now);
      await manager.flush();
      expect(relay.contents, hasLength(1));
      expect(counting.decrypts, foreignCount);

      // The relay echoes our own blob on the live subscription.
      final echo = event(relay.contents.last, 'own-event-1', now + 1, slotId);
      session.onEvent!(echo);
      expect(counting.decrypts, foreignCount, reason: 'echo of our own blob');

      // Publish 2 and 3 pre-fetch the same events plus our own blobs.
      session.history = [...session.history, echo];
      manager.markContextRead('channel-b', now);
      await manager.flush();
      expect(relay.contents, hasLength(2));
      final echo2 = event(relay.contents.last, 'own-event-2', now + 2, slotId);
      session.history = [...session.history, echo2];
      manager.markContextRead('channel-c', now);
      await manager.flush();
      expect(relay.contents, hasLength(3));

      expect(
        counting.decrypts,
        foreignCount,
        reason: 'each ciphertext is decrypted at most once',
      );
      manager.dispose(flushPending: false);
    });
  });
}

class _CapturingRelaySession extends RelaySessionNotifier {
  List<NostrEvent> history = [];
  void Function(NostrEvent)? onEvent;

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => history;

  @override
  Future<List<NostrEvent>> fetchReadStateSnapshot({
    Duration timeout = const Duration(seconds: 8),
  }) async => history;

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    this.onEvent = onEvent;
    return () {};
  }
}

class _RecordingSignedEventRelay implements SignedEventRelay {
  final List<String> contents = [];

  @override
  String? get pubkey => null;

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    void Function(NostrEvent event)? onSigned,
  }) async {
    contents.add(content);
    return NostrEvent(
      id: 'ack-${contents.length}',
      pubkey: '',
      createdAt: createdAt ?? 0,
      kind: kind,
      tags: tags,
      content: content,
      sig: '',
    );
  }
}

Uint8List _key(nostr.Keys keys) => getConversationKey(
  nostr.Nip19.decode(payload: keys.nsec).data,
  keys.public,
);
