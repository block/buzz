import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/inbox_notification_eligibility.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:flutter_test/flutter_test.dart';

const _testChannelId = '580ca78b-9dae-46f3-8854-bd671853ba32';
const _testEventId =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _testRootId =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _testParentId =
    'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';

FeedItem feedItem({
  required String id,
  String category = 'mention',
  String pubkey = 'sender-pk',
  String? channelId = 'channel-1',
  int kind = 9,
  String content = 'hello there',
  int createdAt = 100,
  List<List<String>> tags = const [],
}) {
  return FeedItem(
    id: id,
    kind: kind,
    pubkey: pubkey,
    content: content,
    createdAt: createdAt,
    channelId: channelId,
    channelName: 'general',
    tags: tags,
    category: category,
  );
}

HomeFeedResponse feed({
  List<FeedItem> mentions = const [],
  List<FeedItem> needsAction = const [],
  List<FeedItem> activity = const [],
  List<FeedItem> agentActivity = const [],
}) {
  return HomeFeedResponse(
    mentions: mentions,
    needsAction: needsAction,
    activity: activity,
    agentActivity: agentActivity,
  );
}

List<List<String>> replyTags(String rootId, String parentId) => [
  ['e', rootId, '', 'root'],
  ['e', parentId, '', 'reply'],
];

void main() {
  const myPk = 'my-pk';
  const dmChannelIds = {'dm-1'};

  group('inboxNotificationKindFor', () {
    test('maps mention, needs-action, dm, and agent activity', () {
      expect(
        inboxNotificationKindFor(
          feedItem(id: 'm', category: 'mention'),
          dmChannelIds: dmChannelIds,
        ),
        InboxNotificationKind.mention,
      );
      expect(
        inboxNotificationKindFor(
          feedItem(id: 'n', category: 'needs_action', kind: 46010),
          dmChannelIds: dmChannelIds,
        ),
        InboxNotificationKind.needsAction,
      );
      expect(
        inboxNotificationKindFor(
          feedItem(id: 'd', category: 'activity', channelId: 'dm-1'),
          dmChannelIds: dmChannelIds,
        ),
        InboxNotificationKind.dm,
      );
      expect(
        inboxNotificationKindFor(
          feedItem(id: 'a', category: 'agent_activity', kind: 43004),
          dmChannelIds: dmChannelIds,
        ),
        InboxNotificationKind.agentActivity,
      );
    });

    test('ignores non-dm activity', () {
      expect(
        inboxNotificationKindFor(
          feedItem(id: 'x', category: 'activity', channelId: 'stream-1'),
          dmChannelIds: dmChannelIds,
        ),
        isNull,
      );
    });
  });

  group('shouldNotifyForInboxItem', () {
    test('allows mention, needs-action, dm, and job_result 43004', () {
      for (final item in [
        feedItem(id: 'm', category: 'mention'),
        feedItem(id: 'n', category: 'needs_action', kind: 46010),
        feedItem(id: 'd', category: 'activity', channelId: 'dm-1'),
        feedItem(id: 'j', category: 'agent_activity', kind: 43004),
      ]) {
        expect(
          shouldNotifyForInboxItem(
            item,
            myPubkey: myPk,
            dmChannelIds: dmChannelIds,
            mutedChannelIds: const {},
            mutedRootIds: const {},
          ),
          isTrue,
          reason: item.id,
        );
      }
    });

    test('blocks own pubkey, muted channel, and muted thread', () {
      expect(
        shouldNotifyForInboxItem(
          feedItem(id: 'own', pubkey: myPk),
          myPubkey: myPk,
          dmChannelIds: dmChannelIds,
          mutedChannelIds: const {},
          mutedRootIds: const {},
        ),
        isFalse,
      );
      expect(
        shouldNotifyForInboxItem(
          feedItem(
            id: 'muted',
            category: 'needs_action',
            channelId: 'muted-ch',
          ),
          myPubkey: myPk,
          dmChannelIds: dmChannelIds,
          mutedChannelIds: const {'muted-ch'},
          mutedRootIds: const {},
        ),
        isFalse,
      );
      expect(
        shouldNotifyForInboxItem(
          feedItem(
            id: 'thread',
            category: 'mention',
            tags: replyTags('root-1', 'parent-1'),
          ),
          myPubkey: myPk,
          dmChannelIds: dmChannelIds,
          mutedChannelIds: const {},
          mutedRootIds: const {'root-1'},
        ),
        isFalse,
      );
    });

    test('mention still notifies on muted channel', () {
      expect(
        shouldNotifyForInboxItem(
          feedItem(
            id: 'mention-muted',
            category: 'mention',
            channelId: 'muted-ch',
          ),
          myPubkey: myPk,
          dmChannelIds: dmChannelIds,
          mutedChannelIds: const {'muted-ch'},
          mutedRootIds: const {},
        ),
        isTrue,
      );
    });

    test('agent job_result notifies without mention tag', () {
      expect(
        shouldNotifyForInboxItem(
          feedItem(
            id: 'job',
            category: 'agent_activity',
            kind: 43004,
            tags: const [],
          ),
          myPubkey: myPk,
          dmChannelIds: dmChannelIds,
          mutedChannelIds: const {},
          mutedRootIds: const {},
        ),
        isTrue,
      );
    });
  });

  group('seen snapshot behavior', () {
    test('first snapshot ids are collected without implying delivery', () {
      final response = feed(
        mentions: [
          feedItem(id: 'old-1'),
          feedItem(id: 'old-2'),
        ],
        agentActivity: [
          feedItem(id: 'old-agent', category: 'agent_activity', kind: 43004),
        ],
      );
      final ids = collectInboxNotificationItemIds(
        response,
        dmChannelIds: dmChannelIds,
      );
      expect(ids, ['old-1', 'old-2', 'old-agent']);

      final seen = ids.toSet();
      final later = feed(
        mentions: [
          ...response.mentions,
          feedItem(id: 'new-1', createdAt: 200),
        ],
      );
      final fresh = eligibleInboxNotificationItems(
        later,
        dmChannelIds: dmChannelIds,
      ).where((item) => !seen.contains(item.id)).toList();
      expect(fresh.map((item) => item.id), ['new-1']);
    });
  });

  group('payload + title/body', () {
    test('payload includes channel and event id for tap', () {
      final item = feedItem(id: _testEventId, channelId: _testChannelId);
      final payload = inboxNotificationPayload(item);
      final link = parseMessageDeepLink(Uri.parse(payload));
      expect(link?.channelId, _testChannelId);
      expect(link?.messageId, _testEventId);
    });

    test('thread payload includes thread root when reply-tagged', () {
      final item = feedItem(
        id: _testEventId,
        channelId: _testChannelId,
        tags: replyTags(_testRootId, _testParentId),
      );
      final payload = inboxNotificationPayload(item);
      final link = parseMessageDeepLink(Uri.parse(payload));
      expect(link?.threadRootId, _testParentId);
    });

    test('job_result title/body are agent-specific', () {
      final item = feedItem(
        id: 'job',
        category: 'agent_activity',
        kind: 43004,
        content: 'Done: shipped the fix',
      );
      expect(
        inboxNotificationTitle(
          item,
          isDm: false,
          channelLabel: '#agents',
          senderName: 'Sprig',
        ),
        'Sprig finished a job in #agents',
      );
      expect(inboxNotificationBody(item), 'Done: shipped the fix');
    });
  });

  group('shouldSkipInboxNotificationBecauseVisible', () {
    test('skips when resumed and same channel is visible', () {
      expect(
        shouldSkipInboxNotificationBecauseVisible(
          item: feedItem(id: 'x', channelId: 'ch-1'),
          visible: const VisibleConversationSnapshot(channelId: 'ch-1'),
          appIsResumed: true,
        ),
        isTrue,
      );
      expect(
        shouldSkipInboxNotificationBecauseVisible(
          item: feedItem(id: 'x', channelId: 'ch-1'),
          visible: const VisibleConversationSnapshot(channelId: 'ch-1'),
          appIsResumed: false,
        ),
        isFalse,
      );
    });
  });
}
