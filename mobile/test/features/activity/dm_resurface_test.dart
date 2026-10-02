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

  test('accepts external channel messages regardless of p tags', () {
    NostrEvent event({
      int kind = EventKind.streamMessage,
      String? author,
      List<List<String>>? tags,
    }) => NostrEvent(
      id: 'event-1',
      pubkey: author ?? alice,
      createdAt: 1,
      kind: kind,
      tags:
          tags ??
          const [
            ['h', 'dm-1'],
            ['p', self],
          ],
      content: 'hello',
      sig: 'sig',
    );

    expect(isIncomingChannelMessageFromOther(event(), self), isTrue);
    expect(
      isIncomingChannelMessageFromOther(event(kind: EventKind.reaction), self),
      isFalse,
    );
    expect(
      isIncomingChannelMessageFromOther(event(author: self), self),
      isFalse,
    );
    // #h-scoped delivery already guarantees relevance: an untagged DM from
    // another sender still qualifies.
    expect(
      isIncomingChannelMessageFromOther(
        event(
          tags: const [
            ['h', 'dm-1'],
          ],
        ),
        self,
      ),
      isTrue,
    );
  });
}
