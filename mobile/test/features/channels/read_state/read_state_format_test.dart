import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/relay/nostr_models.dart';

void main() {
  group('read state event validation', () {
    test('requires exactly one valid d tag and one read-state t tag', () {
      final plaintext = jsonEncode({
        'v': 1,
        'client_id': 'client-a',
        'contexts': {'channel-a': 1},
      });

      expect(
        decodeReadStateEvent(
          _event(tags: const []),
          pubkey: 'user-pubkey',
          decrypt: (_) => plaintext,
        ),
        isNull,
      );
      expect(
        decodeReadStateEvent(
          _event(
            tags: const [
              ['d', 'read-state:slot-a'],
              ['d', 'read-state:slot-b'],
              ['t', 'read-state'],
            ],
          ),
          pubkey: 'user-pubkey',
          decrypt: (_) => plaintext,
        ),
        isNull,
      );
      expect(
        decodeReadStateEvent(
          _event(
            tags: const [
              ['d', 'read-state:slót'],
              ['t', 'read-state'],
            ],
          ),
          pubkey: 'user-pubkey',
          decrypt: (_) => plaintext,
        ),
        isNull,
      );
      expect(
        decodeReadStateEvent(
          _event(
            tags: const [
              ['d', 'read-state:slot-a'],
            ],
          ),
          pubkey: 'user-pubkey',
          decrypt: (_) => plaintext,
        ),
        isNull,
      );
      expect(
        decodeReadStateEvent(
          _event(
            tags: const [
              ['d', 'read-state:slot-a'],
              ['t', 'read-state'],
              ['t', 'read-state'],
            ],
          ),
          pubkey: 'user-pubkey',
          decrypt: (_) => plaintext,
        ),
        isNull,
      );
    });

    test('decrypts and sanitizes a valid read-state blob', () {
      final longContextId = 'x' * 257;
      final plaintext = jsonEncode({
        'v': 1,
        'client_id': 'client-a',
        'contexts': {
          'channel-a': 10,
          'string-value': '10',
          'double-value': 10.5,
          'negative': -1,
          'too-large': 4294967296,
          longContextId: 20,
        },
      });

      final decoded = decodeReadStateEvent(
        _event(),
        pubkey: 'user-pubkey',
        decrypt: (_) => plaintext,
      );

      expect(decoded, isNotNull);
      expect(decoded!.dTag, 'read-state:slot-a');
      expect(decoded.blob.clientId, 'client-a');
      expect(decoded.blob.contexts, {'channel-a': 10});
    });

    test('rejects malformed blobs', () {
      expect(decodeReadStateBlob('not json'), isNull);
      expect(
        decodeReadStateBlob(
          jsonEncode({
            'v': 2,
            'client_id': 'client-a',
            'contexts': <String, int>{},
          }),
        ),
        isNull,
      );
      expect(
        decodeReadStateBlob(
          jsonEncode({'v': 1, 'client_id': '', 'contexts': <String, int>{}}),
        ),
        isNull,
      );
      expect(
        decodeReadStateBlob(
          jsonEncode({
            'v': 1,
            'client_id': 'client-a',
            'contexts': List.filled(1, 'not-a-map'),
          }),
        ),
        isNull,
      );
    });
  });

  test('mergeReadStateContexts keeps the maximum timestamp per context', () {
    expect(
      mergeReadStateContexts([
        {'channel-a': 10, 'channel-b': 5},
        {'channel-a': 7, 'channel-c': 1},
        {'channel-b': 12},
      ]),
      {'channel-a': 10, 'channel-b': 12, 'channel-c': 1},
    );
  });

  group('retainReadStateContexts', () {
    int publishedBytes(Map<String, int> contexts) => utf8
        .encode(
          jsonEncode(
            ReadStateBlob(clientId: 'client-a', contexts: contexts).toJson(),
          ),
        )
        .length;

    test('counts JSON escaping in the slot budget', () {
      // Each quote and backslash doubles when encoded.
      final contexts = {
        for (var index = 0; index < 400; index++)
          'msg:${'"\\' * 120}$index': index + 1,
      };

      final retained = retainReadStateContexts(contexts, clientId: 'client-a')!;

      expect(retained, isNotEmpty);
      expect(
        publishedBytes(retained),
        lessThanOrEqualTo(readStatePlaintextBytes),
      );
    });

    test('keeps recent reads when old broad marks fill the budget', () {
      final contexts = <String, int>{
        for (var index = 0; index < 300; index++)
          index.toString().padLeft(36, 'c'): 100,
        for (var index = 0; index < 380; index++)
          'thread:${index.toString().padLeft(64, 't')}': 100,
        'activity:fresh-channel': 500,
        // An old message, read just now from history.
        'msg:${'a' * 64}': 50,
      };
      final recent = {
        for (final key in contexts.keys) key: 1000,
        'activity:fresh-channel': 2000,
        'msg:${'a' * 64}': 2000,
      };
      expect(publishedBytes(contexts), greaterThan(readStatePlaintextBytes));

      final retained = retainReadStateContexts(
        contexts,
        clientId: 'client-a',
        recent: recent,
      )!;

      expect(retained['activity:fresh-channel'], 500);
      expect(retained['msg:${'a' * 64}'], 50);
      expect(
        publishedBytes(retained),
        lessThanOrEqualTo(readStatePlaintextBytes),
      );
      // Broad marks still fill most of the slot.
      expect(retained.keys.where((key) => !key.contains(':')).length, 300);
    });

    test('keeps carried override keys whole ahead of marks', () {
      final carried = {
        for (var index = 0; index < 50; index++) ...{
          'ov_s:channel-$index': 2,
          'ov_c:channel-$index': 1,
          'ov_b:channel-$index': 100,
        },
      };
      final contexts = {
        for (var index = 0; index < 1400; index++)
          'msg:${index.toString().padLeft(64, '0')}': index + 1,
      };

      final retained = retainReadStateContexts(
        contexts,
        clientId: 'client-a',
        carried: carried,
      )!;

      for (final entry in carried.entries) {
        expect(retained[entry.key], entry.value);
      }
      expect(retained.length, lessThan(contexts.length + carried.length));
      expect(
        publishedBytes(retained),
        lessThanOrEqualTo(readStatePlaintextBytes),
      );
    });

    test('returns null instead of splitting carried override keys', () {
      final carried = {
        for (var index = 0; index < 320; index++) ...{
          'ov_s:${index.toString().padLeft(36, 'c')}': 2,
          'ov_c:${index.toString().padLeft(36, 'c')}': 1,
          'ov_b:${index.toString().padLeft(36, 'c')}': 100,
        },
      };

      expect(
        retainReadStateContexts(
          {'channel-1': 5},
          clientId: 'client-a',
          carried: carried,
        ),
        isNull,
      );
    });
  });

  test('pruneStaleContexts bounds message and thread marks only', () {
    const now = 10 * readStateHorizonSeconds;
    final fresh = now - 60;
    final stale = now - readStateHorizonSeconds - 1;
    final contexts = <String, int>{
      'channel-1': stale,
      'activity:channel-1': stale,
      'thread-activity:root': stale,
      'msg:stale': stale,
      'thread:stale': stale,
      for (var index = 0; index < localMaxPrunableContexts + 10; index++)
        'msg:$index': fresh + index,
    };

    final kept = pruneStaleContexts(contexts, nowUnixSeconds: now);

    expect(kept['channel-1'], stale);
    expect(kept['activity:channel-1'], stale);
    expect(kept['thread-activity:root'], stale);
    expect(kept, isNot(contains('msg:stale')));
    expect(kept, isNot(contains('thread:stale')));
    final messages = kept.keys.where((key) => key.startsWith('msg:'));
    expect(messages.length, localMaxPrunableContexts);
    // The oldest are dropped first.
    expect(kept, isNot(contains('msg:0')));
    expect(kept, contains('msg:${localMaxPrunableContexts + 9}'));
  });
}

NostrEvent _event({List<List<String>>? tags}) {
  return NostrEvent(
    id: 'event-id',
    pubkey: 'user-pubkey',
    createdAt: 100,
    kind: EventKind.readState,
    tags:
        tags ??
        const [
          ['d', 'read-state:slot-a'],
          ['t', 'read-state'],
        ],
    content: 'ciphertext',
    sig: 'sig',
  );
}
