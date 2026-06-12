import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/activity/feed_item.dart';

void main() {
  group('FeedItem.fromJson', () {
    test('parses all fields', () {
      final item = FeedItem.fromJson({
        'id': 'evt1',
        'kind': 9,
        'pubkey': 'abc123',
        'content': 'Hello world',
        'created_at': 1700000000,
        'channel_id': 'ch1',
        'channel_name': 'general',
        'tags': [
          ['p', 'abc123'],
        ],
        'category': 'mention',
      });

      expect(item.id, 'evt1');
      expect(item.kind, 9);
      expect(item.pubkey, 'abc123');
      expect(item.content, 'Hello world');
      expect(item.createdAt, 1700000000);
      expect(item.channelId, 'ch1');
      expect(item.channelName, 'general');
      expect(item.tags, [
        ['p', 'abc123'],
      ]);
      expect(item.category, 'mention');
    });

    test('handles null channel_id', () {
      final item = FeedItem.fromJson({
        'id': 'evt2',
        'kind': 9,
        'pubkey': 'abc',
        'content': '',
        'created_at': 0,
        'channel_id': null,
        'channel_name': null,
        'tags': null,
        'category': 'activity',
      });

      expect(item.channelId, isNull);
      expect(item.channelName, '');
      expect(item.tags, isEmpty);
    });
  });

  group('FeedItem.headline', () {
    FeedItem makeItem({required int kind, String category = 'activity'}) =>
        FeedItem(
          id: 'x',
          kind: kind,
          pubkey: 'pk',
          content: '',
          createdAt: 0,
          channelId: null,
          channelName: '',
          tags: const [],
          category: category,
        );

    test('returns known kind labels', () {
      expect(makeItem(kind: 45001).headline, 'Forum post');
      expect(makeItem(kind: 45003).headline, 'Forum reply');
      expect(makeItem(kind: 46010).headline, 'Approval requested');
      expect(makeItem(kind: 43001).headline, 'Job requested');
      expect(makeItem(kind: 43002).headline, 'Job accepted');
      expect(makeItem(kind: 43003).headline, 'Progress update');
      expect(makeItem(kind: 43004).headline, 'Job result');
      expect(makeItem(kind: 43005).headline, 'Job cancelled');
      expect(makeItem(kind: 43006).headline, 'Job failed');
    });

    test('falls back to category for unknown kinds', () {
      expect(makeItem(kind: 9, category: 'mention').headline, 'Mention');
      expect(
        makeItem(kind: 9, category: 'agent_activity').headline,
        'Agent update',
      );
      expect(
        makeItem(kind: 9, category: 'activity').headline,
        'Channel update',
      );
    });
  });

  group('FeedItem.displayContent', () {
    FeedItem makeItem({required String content, int kind = 9}) => FeedItem(
      id: 'x',
      kind: kind,
      pubkey: 'pk',
      content: content,
      createdAt: 0,
      channelId: null,
      channelName: '',
      tags: const [],
      category: 'activity',
    );

    test('returns trimmed content when non-empty', () {
      expect(makeItem(content: '  Hello  ').displayContent, 'Hello');
    });

    test('returns approval fallback for empty approval events', () {
      expect(
        makeItem(content: '', kind: 46010).displayContent,
        'A workflow is waiting for approval.',
      );
    });

    test('returns generic fallback for other empty events', () {
      expect(makeItem(content: '').displayContent, 'No additional details.');
      expect(makeItem(content: '   ').displayContent, 'No additional details.');
    });
  });

  group('HomeFeedResponse', () {
    test('parses from JSON with all four categories', () {
      final response = HomeFeedResponse.fromJson({
        'feed': {
          'mentions': [
            {
              'id': 'm1',
              'kind': 9,
              'pubkey': 'pk',
              'content': 'hi',
              'created_at': 100,
              'channel_id': 'c1',
              'channel_name': 'general',
              'tags': [],
              'category': 'mention',
            },
          ],
          'needs_action': [],
          'activity': [
            {
              'id': 'a1',
              'kind': 9,
              'pubkey': 'pk',
              'content': 'update',
              'created_at': 200,
              'channel_id': 'c2',
              'channel_name': 'dev',
              'tags': [],
              'category': 'activity',
            },
          ],
          'agent_activity': [],
        },
        'meta': {'since': 0, 'total': 2, 'generated_at': 300},
      });

      expect(response.mentions.length, 1);
      expect(response.needsAction, isEmpty);
      expect(response.activity.length, 1);
      expect(response.agentActivity, isEmpty);
    });

    test('handles null category lists gracefully', () {
      final response = HomeFeedResponse.fromJson({
        'feed': {
          'mentions': null,
          'needs_action': null,
          'activity': null,
          'agent_activity': null,
        },
      });

      expect(response.mentions, isEmpty);
      expect(response.isEmpty, isTrue);
    });

    test('all merges and sorts newest-first', () {
      final response = HomeFeedResponse(
        mentions: [
          FeedItem(
            id: 'old',
            kind: 9,
            pubkey: 'pk',
            content: '',
            createdAt: 100,
            channelId: null,
            channelName: '',
            tags: const [],
            category: 'mention',
          ),
        ],
        needsAction: const [],
        activity: [
          FeedItem(
            id: 'new',
            kind: 9,
            pubkey: 'pk',
            content: '',
            createdAt: 300,
            channelId: null,
            channelName: '',
            tags: const [],
            category: 'activity',
          ),
        ],
        agentActivity: [
          FeedItem(
            id: 'mid',
            kind: 9,
            pubkey: 'pk',
            content: '',
            createdAt: 200,
            channelId: null,
            channelName: '',
            tags: const [],
            category: 'agent_activity',
          ),
        ],
      );

      final all = response.all;
      expect(all.length, 3);
      expect(all[0].id, 'new');
      expect(all[1].id, 'mid');
      expect(all[2].id, 'old');
    });

    test('isEmpty returns true when all lists empty', () {
      final response = HomeFeedResponse(
        mentions: [],
        needsAction: [],
        activity: [],
        agentActivity: [],
      );
      expect(response.isEmpty, isTrue);
    });

    test('isEmpty returns false when any list has items', () {
      final response = HomeFeedResponse(
        mentions: [
          FeedItem(
            id: 'x',
            kind: 9,
            pubkey: 'pk',
            content: '',
            createdAt: 0,
            channelId: null,
            channelName: '',
            tags: const [],
            category: 'mention',
          ),
        ],
        needsAction: const [],
        activity: const [],
        agentActivity: const [],
      );
      expect(response.isEmpty, isFalse);
    });
  });
}
