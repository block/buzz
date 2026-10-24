import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/relay/nostr_models.dart';

void main() {
  group('thread marker validation and normalization', () {
    final parent = 'a' * 64;
    final root = 'b' * 64;
    NostrEvent event(List<List<String>> tags) => NostrEvent(
      id: 'c' * 64,
      pubkey: 'd' * 64,
      createdAt: 1,
      kind: EventKind.streamMessage,
      tags: tags,
      content: 'reply',
      sig: '',
    );
    test('last valid markers win and normalize before thread indexing', () {
      final result = event([
        ['e', 'c' * 64, '', 'reply'],
        ['e', parent.toUpperCase(), '', 'reply'],
        ['e', root.toUpperCase(), '', 'root'],
        ['e', 'bad', '', 'reply'],
        ['e', '', '', 'root'],
      ]).threadReference;
      expect(result, (parentId: parent, rootId: root));
    });
    test('malformed markers do not create thread links or hide valid ones', () {
      for (final invalid in [
        '',
        'bad',
        'a' * 63,
        'a' * 65,
        'g' * 64,
        'é' * 64,
        '${'a' * 63}\n',
      ]) {
        expect(
          event([
            ['e', invalid, '', 'reply'],
          ]).threadReference,
          (parentId: null, rootId: null),
        );
        expect(
          event([
            ['e', parent, '', 'reply'],
            ['e', invalid, '', 'reply'],
            ['e', invalid, '', 'root'],
          ]).threadReference,
          (parentId: parent, rootId: parent),
        );
      }
    });
    test('root-only, unmarked, and non-e tags remain top-level', () {
      expect(
        event([
          ['e', root, '', 'root'],
          ['e', parent],
          ['p', parent, '', 'reply'],
        ]).threadReference,
        (parentId: null, rootId: null),
      );
    });
  });

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
