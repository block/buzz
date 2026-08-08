import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:flutter_test/flutter_test.dart';

FeedItem item({
  required String id,
  int createdAt = 100,
  String category = 'mention',
  String pubkey = 'pk1',
  String? channelId = 'ch1',
  List<List<String>> tags = const [],
  int kind = 9,
  String content = 'hello',
}) => FeedItem(
  id: id,
  kind: kind,
  pubkey: pubkey,
  content: content,
  createdAt: createdAt,
  channelId: channelId,
  channelName: '',
  tags: tags,
  category: category,
);

List<List<String>> replyTags(String rootId, String parentId) => [
  ['e', rootId, '', 'root'],
  ['e', parentId, '', 'reply'],
];

void main() {
  group('inboxConversationId', () {
    test('uses root id when present', () {
      expect(
        inboxConversationId(replyTags('root1', 'parent1'), 'ev1'),
        'root1',
      );
    });

    test('falls back to parent id without a root tag', () {
      expect(
        inboxConversationId([
          ['e', 'parent1', '', 'reply'],
        ], 'ev1'),
        'parent1',
      );
    });

    test('falls back to the event id for top-level messages', () {
      expect(inboxConversationId(const [], 'ev1'), 'ev1');
    });
  });

  group('isThreadReply', () {
    test('true for reply-tagged events', () {
      expect(isThreadReply(replyTags('r', 'p')), isTrue);
    });

    test('false for broadcast replies', () {
      expect(
        isThreadReply([
          ...replyTags('r', 'p'),
          ['broadcast', '1'],
        ]),
        isFalse,
      );
    });

    test('false for top-level messages', () {
      expect(isThreadReply(const []), isFalse);
    });
  });

  group('buildInboxItems', () {
    test('groups thread events into one conversation row', () {
      final rows = buildInboxItems([
        item(id: 'a', createdAt: 10, tags: replyTags('root1', 'root1')),
        item(id: 'b', createdAt: 20, tags: replyTags('root1', 'a')),
        item(id: 'c', createdAt: 15),
      ]);

      expect(rows, hasLength(2));
      final threadRow = rows.firstWhere((r) => r.conversationId == 'root1');
      expect(threadRow.id, 'b'); // latest event represents the row
      expect(threadRow.groupItems, hasLength(2));
      expect(threadRow.latestActivityAt, 20);
      expect(threadRow.threadRootId, 'root1');
    });

    test('sorts rows by latest activity, newest first', () {
      final rows = buildInboxItems([
        item(id: 'old', createdAt: 10),
        item(id: 'new', createdAt: 99),
      ]);
      expect(rows.map((r) => r.id).toList(), ['new', 'old']);
    });

    test('groups ordinary top-level DM messages into one row per DM', () {
      bool isDm(String channelId) => channelId == 'dm1';
      final rows = buildInboxItems([
        // Three top-level messages (no thread tags) in the same DM.
        item(id: 'd1', createdAt: 10, channelId: 'dm1', category: 'activity'),
        item(id: 'd2', createdAt: 20, channelId: 'dm1', category: 'activity'),
        item(id: 'd3', createdAt: 30, channelId: 'dm1', category: 'activity'),
        // Ordinary top-level channel posts still group per event.
        item(id: 'c1', createdAt: 40, channelId: 'ch1'),
        item(id: 'c2', createdAt: 50, channelId: 'ch1'),
      ], isDmChannel: isDm);

      expect(rows, hasLength(3));
      final dmRow = rows.singleWhere((r) => r.conversationId == 'dm:dm1');
      expect(dmRow.groupItems, hasLength(3));
      expect(dmRow.id, 'd3'); // latest DM message represents the row
      expect(dmRow.latestActivityAt, 30);
    });

    test('thread replies inside a DM still group by thread root', () {
      bool isDm(String channelId) => channelId == 'dm1';
      final rows = buildInboxItems([
        item(id: 'top', createdAt: 10, channelId: 'dm1'),
        item(
          id: 'reply',
          createdAt: 20,
          channelId: 'dm1',
          tags: replyTags('top', 'top'),
        ),
      ], isDmChannel: isDm);

      expect(rows, hasLength(2));
      expect(rows.map((r) => r.conversationId).toSet(), {'dm:dm1', 'top'});
    });

    test('categories are priority sorted and set isActionRequired', () {
      final rows = buildInboxItems([
        item(
          id: 'a',
          createdAt: 10,
          category: 'activity',
          tags: replyTags('root1', 'root1'),
        ),
        item(
          id: 'b',
          createdAt: 20,
          category: 'needs_action',
          tags: replyTags('root1', 'a'),
        ),
      ]);
      expect(rows.single.categories.first, 'needs_action');
      expect(rows.single.isActionRequired, isTrue);
    });
  });

  group('InboxItem.deepLinkTarget', () {
    final rows = buildInboxItems([
      item(id: 'a', createdAt: 10, tags: replyTags('root1', 'root1')),
      item(id: 'b', createdAt: 20, tags: replyTags('root1', 'a')),
      item(id: 'c', createdAt: 30, tags: replyTags('root1', 'b')),
    ]);

    test('targets the oldest unread event', () {
      expect(rows.single.deepLinkTarget(15).id, 'b');
    });

    test('targets the oldest event when nothing is read', () {
      expect(rows.single.deepLinkTarget(5).id, 'a');
    });

    test('falls back to the latest event when all are read', () {
      expect(rows.single.deepLinkTarget(99).id, 'c');
    });

    test('falls back to the latest event without a read marker', () {
      expect(rows.single.deepLinkTarget(null).id, 'c');
    });
  });

  group('matchesInboxFilter', () {
    InboxItem rowOf(FeedItem feedItem) => buildInboxItems([feedItem]).single;

    test('all matches everything', () {
      expect(matchesInboxFilter(rowOf(item(id: 'a')), InboxFilter.all), isTrue);
    });

    test('thread filter needs a thread reply in the group', () {
      expect(
        matchesInboxFilter(rowOf(item(id: 'a')), InboxFilter.thread),
        isFalse,
      );
      expect(
        matchesInboxFilter(
          rowOf(item(id: 'a', tags: replyTags('r', 'p'))),
          InboxFilter.thread,
        ),
        isTrue,
      );
    });

    test('category filters match their category', () {
      final mention = rowOf(item(id: 'a', category: 'mention'));
      expect(matchesInboxFilter(mention, InboxFilter.mention), isTrue);
      expect(matchesInboxFilter(mention, InboxFilter.needsAction), isFalse);
      expect(matchesInboxFilter(mention, InboxFilter.activity), isFalse);
      expect(
        matchesInboxFilter(
          rowOf(item(id: 'b', category: 'agent_activity')),
          InboxFilter.agentActivity,
        ),
        isTrue,
      );
    });

    test('reminders and drafts are separate surfaces', () {
      final row = rowOf(item(id: 'a'));
      expect(matchesInboxFilter(row, InboxFilter.reminders), isFalse);
      expect(matchesInboxFilter(row, InboxFilter.drafts), isFalse);
    });
  });

  group('inboxTypeLabel', () {
    InboxItem rowOf(FeedItem feedItem) => buildInboxItems([feedItem]).single;

    test('DM rows lead with the sender', () {
      final label = inboxTypeLabel(
        rowOf(item(id: 'a', category: 'activity')),
        channelName: null,
        isDm: true,
        senderLabel: 'Alice',
      );
      expect(label.text, 'DM from Alice');
      expect(label.channelLabel, isNull);
    });

    test('mention rows say Mentioned in #channel', () {
      final label = inboxTypeLabel(
        rowOf(item(id: 'a', category: 'mention')),
        channelName: 'general',
        isDm: false,
        senderLabel: 'Alice',
      );
      expect(label.text, 'Mentioned in');
      expect(label.channelLabel, 'general');
    });

    test('needs action wins over thread context', () {
      final label = inboxTypeLabel(
        rowOf(
          item(id: 'a', category: 'needs_action', tags: replyTags('r', 'p')),
        ),
        channelName: 'general',
        isDm: false,
        senderLabel: 'Alice',
      );
      expect(label.text, 'Needs action in');
    });

    test('plain thread replies say Thread in', () {
      final label = inboxTypeLabel(
        rowOf(item(id: 'a', category: 'activity', tags: replyTags('r', 'p'))),
        channelName: 'general',
        isDm: false,
        senderLabel: 'Alice',
      );
      expect(label.text, 'Thread in');
    });
  });

  group('inboxDayLabel', () {
    final now = DateTime(2026, 7, 25, 12);
    int at(DateTime d) => d.millisecondsSinceEpoch ~/ 1000;

    test('today / yesterday / weekday / date buckets', () {
      expect(inboxDayLabel(at(DateTime(2026, 7, 25, 9)), now: now), 'Today');
      expect(
        inboxDayLabel(at(DateTime(2026, 7, 24, 9)), now: now),
        'Yesterday',
      );
      expect(inboxDayLabel(at(DateTime(2026, 7, 21, 9)), now: now), 'Tuesday');
      expect(inboxDayLabel(at(DateTime(2026, 1, 2)), now: now), 'Jan 2');
      expect(
        inboxDayLabel(at(DateTime(2025, 12, 31)), now: now),
        'Dec 31, 2025',
      );
    });
  });
}
