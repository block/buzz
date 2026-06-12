import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/read_state/read_state_format.dart';
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
