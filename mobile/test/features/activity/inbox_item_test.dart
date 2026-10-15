import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/thread_ids.dart';

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
  test(
    'Inbox groups mixed-case replies despite trailing malformed markers',
    () {
      final root = 'a' * 64;
      final parent = 'b' * 64;
      final tags = [
        ['e', root.toUpperCase(), '', 'root'],
        ['e', parent.toUpperCase(), '', 'reply'],
        ['e', 'bad', '', 'reply'],
        ['e', '', '', 'root'],
      ];
      final target = item(id: 'target', createdAt: 20, tags: tags);
      final rows = buildInboxItems([
        item(id: 'earlier', createdAt: 10, tags: replyTags(root, parent)),
        target,
      ]);
      expect(rows, hasLength(1));
      expect(rows.single.conversationId, root);
      expect(rows.single.threadRootId, root);
      expect(rows.single.deepLinkTarget((e) => e.createdAt <= 10).id, 'target');
      expect(threadReferenceOf(target.tags).parentId, parent);
      expect(isThreadReply(tags), isTrue);
      expect(
        isThreadReply([
          ...tags,
          ['broadcast', '1'],
        ]),
        isFalse,
      );
    },
  );

  group('inboxConversationId', () {
    test('uses root id when present', () {
      expect(
        inboxConversationId(
          replyTags(threadIdInboxRoot1, threadIdParent1),
          'ev1',
        ),
        threadIdInboxRoot1,
      );
    });

    test('falls back to parent id without a root tag', () {
      expect(
        inboxConversationId([
          ['e', threadIdParent1, '', 'reply'],
        ], 'ev1'),
        threadIdParent1,
      );
    });

    test('falls back to the event id for top-level messages', () {
      expect(inboxConversationId(const [], 'ev1'), 'ev1');
    });
  });

  group('isThreadReply', () {
    test('true for reply-tagged events', () {
      expect(isThreadReply(replyTags(threadIdInboxR, threadIdInboxP)), isTrue);
    });

    test('false for broadcast replies', () {
      expect(
        isThreadReply([
          ...replyTags(threadIdInboxR, threadIdInboxP),
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
        item(
          id: threadIdInboxA,
          createdAt: 10,
          tags: replyTags(threadIdInboxRoot1, threadIdInboxRoot1),
        ),
        item(
          id: threadIdInboxB,
          createdAt: 20,
          tags: replyTags(threadIdInboxRoot1, threadIdInboxA),
        ),
        item(id: 'c', createdAt: 15),
      ]);

      expect(rows, hasLength(2));
      final threadRow = rows.firstWhere(
        (r) => r.conversationId == threadIdInboxRoot1,
      );
      expect(threadRow.id, threadIdInboxB); // latest event represents the row
      expect(threadRow.groupItems, hasLength(2));
      expect(threadRow.latestActivityAt, 20);
      expect(threadRow.threadRootId, threadIdInboxRoot1);
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
        item(id: threadIdInboxTop, createdAt: 10, channelId: 'dm1'),
        item(
          id: 'reply',
          createdAt: 20,
          channelId: 'dm1',
          tags: replyTags(threadIdInboxTop, threadIdInboxTop),
        ),
      ], isDmChannel: isDm);

      expect(rows, hasLength(2));
      expect(rows.map((r) => r.conversationId).toSet(), {
        'dm:dm1',
        threadIdInboxTop,
      });
    });

    test('categories are priority sorted and set isActionRequired', () {
      final rows = buildInboxItems([
        item(
          id: threadIdInboxA,
          createdAt: 10,
          category: 'activity',
          tags: replyTags(threadIdInboxRoot1, threadIdInboxRoot1),
        ),
        item(
          id: threadIdInboxB,
          createdAt: 20,
          category: 'needs_action',
          tags: replyTags(threadIdInboxRoot1, threadIdInboxA),
        ),
      ]);
      expect(rows.single.categories.first, 'needs_action');
      expect(rows.single.isActionRequired, isTrue);
    });
  });

  group('InboxItem.deepLinkTarget', () {
    final rows = buildInboxItems([
      item(
        id: threadIdInboxA,
        createdAt: 10,
        tags: replyTags(threadIdInboxRoot1, threadIdInboxRoot1),
      ),
      item(
        id: threadIdInboxB,
        createdAt: 20,
        tags: replyTags(threadIdInboxRoot1, threadIdInboxA),
      ),
      item(
        id: 'c',
        createdAt: 30,
        tags: replyTags(threadIdInboxRoot1, threadIdInboxB),
      ),
    ]);

    bool readUpTo(int time, FeedItem event) => event.createdAt <= time;

    test('targets the oldest unread event', () {
      expect(
        rows.single.deepLinkTarget((e) => readUpTo(15, e)).id,
        threadIdInboxB,
      );
    });

    test('targets the oldest event when nothing is read', () {
      expect(rows.single.deepLinkTarget((_) => false).id, threadIdInboxA);
    });

    test('falls back to the latest event when all are read', () {
      expect(rows.single.deepLinkTarget((_) => true).id, 'c');
    });

    test('checks each event, not a single time', () {
      // Only the middle reply is unread, though the newest is read.
      expect(
        rows.single.deepLinkTarget((e) => e.id != threadIdInboxB).id,
        threadIdInboxB,
      );
    });
  });

  group('matchesInboxFilter', () {
    InboxItem rowOf(FeedItem feedItem) => buildInboxItems([feedItem]).single;

    test('all matches everything', () {
      expect(
        matchesInboxFilter(rowOf(item(id: threadIdInboxA)), InboxFilter.all),
        isTrue,
      );
    });

    test('thread filter needs a thread reply in the group', () {
      expect(
        matchesInboxFilter(rowOf(item(id: threadIdInboxA)), InboxFilter.thread),
        isFalse,
      );
      expect(
        matchesInboxFilter(
          rowOf(
            item(
              id: threadIdInboxA,
              tags: replyTags(threadIdInboxR, threadIdInboxP),
            ),
          ),
          InboxFilter.thread,
        ),
        isTrue,
      );
    });

    test('category filters match their category', () {
      final mention = rowOf(item(id: threadIdInboxA, category: 'mention'));
      expect(matchesInboxFilter(mention, InboxFilter.mention), isTrue);
      expect(matchesInboxFilter(mention, InboxFilter.needsAction), isFalse);
      expect(matchesInboxFilter(mention, InboxFilter.activity), isFalse);
      expect(
        matchesInboxFilter(
          rowOf(item(id: threadIdInboxB, category: 'agent_activity')),
          InboxFilter.agentActivity,
        ),
        isTrue,
      );
    });

    test('reminders and drafts are separate surfaces', () {
      final row = rowOf(item(id: threadIdInboxA));
      expect(matchesInboxFilter(row, InboxFilter.reminders), isFalse);
      expect(matchesInboxFilter(row, InboxFilter.drafts), isFalse);
    });
  });

  group('inboxTypeLabel', () {
    InboxItem rowOf(FeedItem feedItem) => buildInboxItems([feedItem]).single;

    test('DM rows lead with the sender', () {
      final label = inboxTypeLabel(
        rowOf(item(id: threadIdInboxA, category: 'activity')),
        channelName: null,
        isDm: true,
        senderLabel: 'Alice',
      );
      expect(label.text, 'DM from Alice');
      expect(label.channelLabel, isNull);
    });

    test('mention rows say Mentioned in #channel', () {
      final label = inboxTypeLabel(
        rowOf(item(id: threadIdInboxA, category: 'mention')),
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
          item(
            id: threadIdInboxA,
            category: 'needs_action',
            tags: replyTags(threadIdInboxR, threadIdInboxP),
          ),
        ),
        channelName: 'general',
        isDm: false,
        senderLabel: 'Alice',
      );
      expect(label.text, 'Needs action in');
    });

    test('plain thread replies say Thread in', () {
      final label = inboxTypeLabel(
        rowOf(
          item(
            id: threadIdInboxA,
            category: 'activity',
            tags: replyTags(threadIdInboxR, threadIdInboxP),
          ),
        ),
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
