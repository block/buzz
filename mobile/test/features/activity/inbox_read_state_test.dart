import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:buzz/features/activity/inbox_read_state.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/thread_ids.dart';

FeedItem item({
  required String id,
  int createdAt = 100,
  String category = 'mention',
  String? channelId = 'ch1',
  List<List<String>> tags = const [],
}) => FeedItem(
  id: id,
  kind: 9,
  pubkey: 'pk1',
  content: 'hello',
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

int? Function(String) markers(Map<String, int> map) =>
    (contextId) => map[contextId];

void main() {
  group('inboxEventReadAt', () {
    test('a top-level event uses the channel and its own mark', () {
      final event = item(id: threadIdInboxA, createdAt: 60);
      expect(inboxEventReadAt(event, markerOf: markers({'ch1': 42})), 42);
      expect(
        inboxEventReadAt(
          event,
          markerOf: markers({'ch1': 42, 'msg:$threadIdInboxA': 60}),
        ),
        60,
      );
    });

    test('a reply also uses its thread marks', () {
      final event = item(
        id: threadIdInboxA,
        tags: replyTags(threadIdInboxRoot1, threadIdInboxRoot1),
      );
      expect(
        inboxEventReadAt(
          event,
          markerOf: markers({
            'thread:$threadIdInboxRoot1': 10,
            'thread-activity:$threadIdInboxRoot1': 30,
          }),
        ),
        30,
      );
    });

    test('channel catch-up never reads an Activity event', () {
      final event = item(
        id: threadIdInboxA,
        createdAt: 60,
        category: 'activity',
      );
      expect(
        inboxEventReadAt(event, markerOf: markers({'activity:ch1': 99})),
        isNull,
      );
    });

    test('an event without a channel has no marker', () {
      final event = item(id: threadIdInboxA, channelId: null);
      expect(inboxEventReadAt(event, markerOf: markers({})), isNull);
    });
  });

  group('isInboxItemDone', () {
    test('done when no grouped activity is newer than the marker', () {
      final row = buildInboxItems([
        item(id: threadIdInboxA, createdAt: 50),
      ]).single;
      expect(
        isInboxItemDone(
          row,
          markerOf: markers({'ch1': 50}),
          localUnreadOverrides: const {},
          localDoneSet: const {},
        ),
        isTrue,
      );
      expect(
        isInboxItemDone(
          row,
          markerOf: markers({'ch1': 49}),
          localUnreadOverrides: const {},
          localDoneSet: const {},
        ),
        isFalse,
      );
    });

    test('a channel mention read by its own mark is done', () {
      // Reading a channel writes `msg:` for a mention, not the channel mark.
      final row = buildInboxItems([item(id: 'm', createdAt: 50)]).single;
      expect(
        isInboxItemDone(
          row,
          markerOf: markers({'ch1': 10, 'msg:m': 50}),
          localUnreadOverrides: const {},
          localDoneSet: const {},
        ),
        isTrue,
      );
    });

    test('a thread mention needs every grouped reply read', () {
      final row = buildInboxItems([
        item(
          id: threadIdR1,
          createdAt: 40,
          tags: replyTags(threadIdInboxRoot1, threadIdInboxRoot1),
        ),
        item(
          id: 'r2',
          createdAt: 50,
          tags: replyTags(threadIdInboxRoot1, threadIdR1),
        ),
      ]).single;
      bool done(Map<String, int> marks) => isInboxItemDone(
        row,
        markerOf: markers(marks),
        localUnreadOverrides: const {},
        localDoneSet: const {},
      );
      // Seeing only the newest reply leaves the older mention unread.
      expect(done({'msg:r2': 50}), isFalse);
      expect(done({'msg:$threadIdR1': 40, 'msg:r2': 50}), isTrue);
      expect(done({'thread-activity:$threadIdInboxRoot1': 50}), isTrue);
    });

    test('a local unread override always wins', () {
      final row = buildInboxItems([
        item(id: threadIdInboxA, createdAt: 50),
      ]).single;
      expect(
        isInboxItemDone(
          row,
          markerOf: markers({'ch1': 99}),
          localUnreadOverrides: const {threadIdInboxA},
          localDoneSet: const {},
        ),
        isFalse,
      );
    });

    test('channel-less rows fall back to the local done set', () {
      final row = buildInboxItems([
        item(id: threadIdInboxA, channelId: null),
      ]).single;
      expect(
        isInboxItemDone(
          row,
          markerOf: markers({}),
          localUnreadOverrides: const {},
          localDoneSet: const {},
        ),
        isFalse,
      );
      expect(
        isInboxItemDone(
          row,
          markerOf: markers({}),
          localUnreadOverrides: const {},
          localDoneSet: const {threadIdInboxA},
        ),
        isTrue,
      );
    });
  });

  group('groupedChannelReadTimestamp', () {
    test('only counts top-level events in the channel', () {
      final row = buildInboxItems([
        item(
          id: threadIdInboxA,
          createdAt: 10,
          tags: replyTags(threadIdInboxRoot1, threadIdInboxRoot1),
        ),
        item(
          id: threadIdInboxB,
          createdAt: 30,
          tags: replyTags(threadIdInboxRoot1, threadIdInboxA),
        ),
      ]).single;
      // Every grouped event is a thread reply, so marking the thread read
      // must not advance the channel marker.
      expect(groupedChannelReadTimestamp(row), isNull);
    });

    test('returns the newest top-level timestamp', () {
      final row = buildInboxItems([
        item(id: threadIdInboxRoot1, createdAt: 10),
        item(
          id: threadIdInboxB,
          createdAt: 30,
          tags: replyTags(threadIdInboxRoot1, threadIdInboxRoot1),
        ),
      ]).single;
      final result = groupedChannelReadTimestamp(row);
      expect(result?.channelId, 'ch1');
      expect(result?.timestamp, 10);
    });
  });
}
