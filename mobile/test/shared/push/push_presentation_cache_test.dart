import 'package:buzz/shared/push/push_presentation_cache.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  const secretKey =
      '0000000000000000000000000000000000000000000000000000000000000001';

  test('accepts a valid signed profile event', () {
    final signed = nostr.Event.from(
      kind: 0,
      content: '{"display_name":"Alice"}',
      secretKey: secretKey,
      createdAt: 1700000000,
    );

    expect(
      isVerifiedPushPresentationEvent(NostrEvent.fromJson(signed.toMap())),
      isTrue,
    );
  });

  test('accepts opaque channel IDs in a valid signed metadata event', () {
    final signed = nostr.Event.from(
      kind: 39000,
      content: '',
      tags: const [
        ['d', 'channel/general:v5'],
        ['name', 'General'],
      ],
      secretKey: secretKey,
      createdAt: 1700000000,
    );

    expect(
      isVerifiedPushPresentationEvent(NostrEvent.fromJson(signed.toMap())),
      isTrue,
    );
  });

  test('accepts a valid signed channel membership snapshot', () {
    final signed = nostr.Event.from(
      kind: 39002,
      content: '',
      tags: const [
        ['d', 'channel/general:v5'],
        [
          'p',
          '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',
          'member',
        ],
      ],
      secretKey: secretKey,
      createdAt: 1700000000,
    );

    expect(
      isVerifiedPushPresentationEvent(NostrEvent.fromJson(signed.toMap())),
      isTrue,
    );
  });

  test('channel selection keeps newest metadata paired with rosters', () {
    NostrEvent signedChannelEvent(int kind, String channelID, int createdAt) {
      final signed = nostr.Event.from(
        kind: kind,
        content: '',
        tags: [
          ['d', channelID],
          if (kind == 39002)
            [
              'p',
              '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',
            ],
        ],
        secretKey: secretKey,
        createdAt: createdAt,
      );
      return NostrEvent.fromJson(signed.toMap());
    }

    final batch = selectPushChannelEvents(
      [
        signedChannelEvent(39000, 'channel-0', 100),
        signedChannelEvent(39000, 'channel-1', 300),
        signedChannelEvent(39000, 'channel-1', 200),
        signedChannelEvent(39000, 'metadata-only', 400),
      ],
      [
        signedChannelEvent(39002, 'channel-0', 300),
        signedChannelEvent(39002, 'channel-1', 200),
      ],
    );

    expect(batch.metadata.map((event) => event.getTagValue('d')).toSet(), {
      'channel-0',
      'channel-1',
    });
    expect(batch.membership.map((event) => event.getTagValue('d')).toSet(), {
      'channel-0',
      'channel-1',
    });
  });

  test('rejects changed content and malformed signatures', () {
    final signed = nostr.Event.from(
      kind: 0,
      content: '{"name":"Alice"}',
      secretKey: secretKey,
      createdAt: 1700000000,
    );
    final event = NostrEvent.fromJson(signed.toMap());

    expect(
      isVerifiedPushPresentationEvent(
        NostrEvent(
          id: event.id,
          pubkey: event.pubkey,
          createdAt: event.createdAt,
          kind: event.kind,
          tags: event.tags,
          content: '{"name":"Mallory"}',
          sig: event.sig,
        ),
      ),
      isFalse,
    );
    expect(
      isVerifiedPushPresentationEvent(
        NostrEvent(
          id: event.id,
          pubkey: event.pubkey,
          createdAt: event.createdAt,
          kind: event.kind,
          tags: event.tags,
          content: event.content,
          sig: '00',
        ),
      ),
      isFalse,
    );
  });
}
