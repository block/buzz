import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/activity/dm_resurface.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  const self =
      '1111111111111111111111111111111111111111111111111111111111111111';
  const alice =
      '2222222222222222222222222222222222222222222222222222222222222222';
  const bob =
      '3333333333333333333333333333333333333333333333333333333333333333';

  test('derives group DM peers from authoritative membership', () {
    expect(dmPeerPubkeysFromMembers([self, alice, bob], self), {alice, bob});
    expect(dmPeerPubkeysFromMembers([alice, bob], self), isEmpty);
  });

  test('accepts only external addressed human-message events', () {
    NostrEvent event({int kind = EventKind.streamMessage, String? author}) =>
        NostrEvent(
          id: 'event-1',
          pubkey: author ?? alice,
          createdAt: 1,
          kind: kind,
          tags: const [
            ['h', 'dm-1'],
            ['p', self],
          ],
          content: 'hello',
          sig: 'sig',
        );

    expect(isIncomingDmMessageEvent(event(), self), isTrue);
    expect(
      isIncomingDmMessageEvent(event(kind: EventKind.reaction), self),
      isFalse,
    );
    expect(isIncomingDmMessageEvent(event(author: self), self), isFalse);
  });
}
