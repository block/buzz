import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/push/push_models.dart';
import 'package:buzz/shared/relay/nostr_models.dart';

void main() {
  test('resolves newest user-visible event into notification content', () {
    final mine = 'a' * 64;
    final alice = 'b' * 64;
    final older = _event(
      id: '1' * 64,
      pubkey: alice,
      createdAt: 10,
      content: 'older',
    );
    final newest = _event(
      id: '2' * 64,
      pubkey: alice,
      createdAt: 20,
      content: 'hello [there](https://example.com)',
      tags: [
        ['h', 'chan-1'],
      ],
    );
    final profile = _event(
      id: '3' * 64,
      pubkey: alice,
      kind: EventKind.contactList,
      content: '{"display_name":"Alice"}',
    );

    final resolved = resolveBuzzPushNotification(
      events: [older, newest],
      myPubkey: mine,
      communityName: 'Team',
      channelName: 'mobile',
      profilesByPubkey: {alice: ProfileData.fromEvent(profile)},
    );

    expect(resolved, isNotNull);
    expect(resolved!.title, 'Alice');
    expect(resolved.subtitle, 'mobile');
    expect(resolved.body, 'hello there');
    expect(resolved.threadIdentifier, 'chan-1');
  });

  test(
    'keeps fallback when only self-authored or non-message events exist',
    () {
      final mine = 'a' * 64;
      final resolved = resolveBuzzPushNotification(
        events: [
          _event(id: '1' * 64, pubkey: mine, content: 'self'),
          _event(
            id: '2' * 64,
            pubkey: 'b' * 64,
            kind: EventKind.reaction,
            content: '+',
          ),
        ],
        myPubkey: mine,
        communityName: 'Team',
      );

      expect(resolved, isNull);
    },
  );

  test('trims long previews', () {
    final resolved = resolveBuzzPushNotification(
      events: [_event(id: '1' * 64, pubkey: 'b' * 64, content: 'x' * 240)],
      myPubkey: 'a' * 64,
      communityName: 'Team',
    );

    expect(resolved!.body.length, lessThanOrEqualTo(180));
    expect(resolved.body.endsWith('…'), isTrue);
  });
}

NostrEvent _event({
  required String id,
  required String pubkey,
  required String content,
  int kind = EventKind.streamMessage,
  int createdAt = 1,
  List<List<String>> tags = const [],
}) {
  return NostrEvent(
    id: id,
    pubkey: pubkey,
    createdAt: createdAt,
    kind: kind,
    tags: tags,
    content: content,
    sig: '0' * 128,
  );
}
