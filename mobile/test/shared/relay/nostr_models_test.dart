import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/relay/nostr_models.dart';

void main() {
  test('NostrFilter serializes and preserves authors', () {
    const filter = NostrFilter(
      kinds: [EventKind.readState],
      authors: ['pubkey-a'],
      tags: {
        '#t': ['read-state'],
      },
      since: 10,
      limit: 5,
    );

    expect(filter.toJson(), {
      'kinds': [EventKind.readState],
      'limit': 5,
      'authors': ['pubkey-a'],
      'since': 10,
      '#t': ['read-state'],
    });

    final copied = filter.copyWithSince(20);
    expect(copied.toJson(), {
      'kinds': [EventKind.readState],
      'limit': 5,
      'authors': ['pubkey-a'],
      'since': 20,
      '#t': ['read-state'],
    });
  });

  test('NostrFilter can serialize broad deletion subscriptions', () {
    const filter = NostrFilter(kinds: [EventKind.deletion], limit: 0);

    expect(filter.toJson(), {
      'kinds': [EventKind.deletion],
      'limit': 0,
    });
  });

  test('NostrFilter serializes relay query extensions', () {
    const filter = NostrFilter(
      kinds: EventKind.channelTimelineContentKinds,
      tags: {
        '#h': ['channel-id'],
      },
      limit: 50,
      extensions: {
        'top_level': true,
        'include_summaries': true,
        'include_aux': true,
        'before_id': 'cursor-id',
      },
    );

    expect(filter.toJson(), {
      'kinds': EventKind.channelTimelineContentKinds,
      'limit': 50,
      '#h': ['channel-id'],
      'top_level': true,
      'include_summaries': true,
      'include_aux': true,
      'before_id': 'cursor-id',
    });
  });

  test('channel unread activity kinds exclude non-message updates', () {
    expect(
      EventKind.channelMessageEventKinds,
      contains(EventKind.streamMessage),
    );
    expect(EventKind.channelMessageEventKinds, contains(EventKind.forumPost));
    expect(
      EventKind.channelMessageEventKinds,
      contains(EventKind.forumComment),
    );

    expect(
      EventKind.channelMessageEventKinds,
      isNot(contains(EventKind.reaction)),
    );
    expect(
      EventKind.channelMessageEventKinds,
      isNot(contains(EventKind.streamMessageEdit)),
    );
    expect(
      EventKind.channelMessageEventKinds,
      isNot(contains(EventKind.streamMessageDiff)),
    );
    expect(
      EventKind.channelMessageEventKinds,
      isNot(contains(EventKind.deletion)),
    );
    expect(
      EventKind.channelMessageEventKinds,
      isNot(contains(EventKind.systemMessage)),
    );
  });
}
