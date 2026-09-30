import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:buzz/features/activity/inbox_read_state.dart';
import 'package:flutter_test/flutter_test.dart';

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
  group('resolveInboxItemReadAt', () {
    test('channel rows use the channel marker', () {
      final row = buildInboxItems([item(id: 'a')]).single;
      expect(resolveInboxItemReadAt(row, markerOf: markers({'ch1': 42})), 42);
    });

    test('thread rows use max of thread and msg markers', () {
      final row = buildInboxItems([
        item(id: 'a', tags: replyTags('root1', 'root1')),
      ]).single;
      expect(
        resolveInboxItemReadAt(
          row,
          markerOf: markers({'thread:root1': 10, 'msg:a': 25}),
        ),
        25,
      );
    });

    test('channel-less rows have no marker', () {
      final row = buildInboxItems([item(id: 'a', channelId: null)]).single;
      expect(resolveInboxItemReadAt(row, markerOf: markers({})), isNull);
    });
  });

  group('isInboxItemDone', () {
    test('done when no grouped activity is newer than the marker', () {
      final row = buildInboxItems([item(id: 'a', createdAt: 50)]).single;
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

    test('a local unread override always wins', () {
      final row = buildInboxItems([item(id: 'a', createdAt: 50)]).single;
      expect(
        isInboxItemDone(
          row,
          markerOf: markers({'ch1': 99}),
          localUnreadOverrides: const {'a'},
          localDoneSet: const {},
        ),
        isFalse,
      );
    });

    test('channel-less rows fall back to the local done set', () {
      final row = buildInboxItems([item(id: 'a', channelId: null)]).single;
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
          localDoneSet: const {'a'},
        ),
        isTrue,
      );
    });
  });

  group('groupedChannelReadTimestamp', () {
    test('only counts top-level events in the channel', () {
      final row = buildInboxItems([
        item(id: 'a', createdAt: 10, tags: replyTags('root1', 'root1')),
        item(id: 'b', createdAt: 30, tags: replyTags('root1', 'a')),
      ]).single;
      // Every grouped event is a thread reply, so marking the thread read
      // must not advance the channel marker.
      expect(groupedChannelReadTimestamp(row), isNull);
    });

    test('returns the newest top-level timestamp', () {
      final row = buildInboxItems([
        item(id: 'root1', createdAt: 10),
        item(id: 'b', createdAt: 30, tags: replyTags('root1', 'root1')),
      ]).single;
      final result = groupedChannelReadTimestamp(row);
      expect(result?.channelId, 'ch1');
      expect(result?.timestamp, 10);
    });
  });
}
