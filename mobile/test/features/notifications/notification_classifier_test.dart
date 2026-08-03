import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/notifications/notification_classifier.dart';
import 'package:buzz/features/notifications/notification_event.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('classifyNotificationEvent', () {
    test('excludes self-authored events', () {
      final notification = classifyNotificationEvent(
        event: event(pubkey: 'me'),
        channel: channel(),
        myPubkey: 'ME',
      );

      expect(notification, isNull);
    });

    test('mention passes a muted channel as priority', () {
      final notification = classifyNotificationEvent(
        event: event(
          id: 'mention-1',
          content: '  hello there  ',
          tags: const [
            ['h', 'channel-1'],
            ['p', 'ME'],
          ],
        ),
        channel: channel(),
        myPubkey: 'me',
        senderName: ' Alice ',
        mutedChannelIds: const {'channel-1'},
      );

      expect(notification, isNotNull);
      expect(notification!.category, NotificationCategory.priority);
      expect(notification.channel, 'priority');
      expect(notification.title, 'Alice mentioned you in #general');
      expect(notification.body, 'hello there');
      expect(
        notification.route,
        'buzz://message?channel=channel-1&id=mention-1',
      );
    });

    test('direct message is priority and uses its sender title', () {
      final notification = classifyNotificationEvent(
        event: event(
          id: 'dm-1',
          tags: const [
            ['h', 'dm-1'],
          ],
        ),
        channel: channel(id: 'dm-1', type: 'dm', name: 'DM'),
        myPubkey: 'me',
        senderName: 'Bob',
      );

      expect(notification, isNotNull);
      expect(notification!.category, NotificationCategory.priority);
      expect(notification.title, 'Bob');
    });

    test('direct message falls back to a generic title', () {
      final notification = classifyNotificationEvent(
        event: event(
          id: 'dm-1',
          tags: const [
            ['h', 'dm-1'],
          ],
        ),
        channel: channel(id: 'dm-1', type: 'dm', name: 'DM'),
        myPubkey: 'me',
      );

      expect(notification?.title, 'Direct message');
    });

    test('muted regular activity is suppressed', () {
      final notification = classifyNotificationEvent(
        event: event(),
        channel: channel(),
        myPubkey: 'me',
        mutedChannelIds: const {'channel-1'},
      );

      expect(notification, isNull);
    });

    test('rejects an event routed through the wrong channel', () {
      final notification = classifyNotificationEvent(
        event: event(
          tags: const [
            ['h', 'channel-2'],
          ],
        ),
        channel: channel(),
        myPubkey: 'me',
      );

      expect(notification, isNull);
    });

    test('relevant thread reply is preserved as activity', () {
      final notification = classifyNotificationEvent(
        event: event(
          id: 'reply-1',
          tags: const [
            ['h', 'channel-1'],
            ['e', 'root-1', '', 'root'],
            ['e', 'parent-1', '', 'reply'],
          ],
        ),
        channel: channel(),
        myPubkey: 'me',
        followedRootIds: const {'root-1'},
      );

      expect(notification, isNotNull);
      expect(notification!.category, NotificationCategory.activity);
      expect(notification.title, 'New message in #general');
      expect(
        notification.route,
        'buzz://message?channel=channel-1&id=reply-1&thread=root-1',
      );
    });

    test('unrelated thread reply is suppressed', () {
      final notification = classifyNotificationEvent(
        event: event(
          tags: const [
            ['h', 'channel-1'],
            ['e', 'root-1', '', 'root'],
            ['e', 'parent-1', '', 'reply'],
          ],
        ),
        channel: channel(),
        myPubkey: 'me',
      );

      expect(notification, isNull);
    });

    test('regular top-level message is optional activity', () {
      final notification = classifyNotificationEvent(
        event: event(),
        channel: channel(),
        myPubkey: 'me',
      );

      expect(notification, isNotNull);
      expect(notification!.category, NotificationCategory.activity);
      expect(notification.title, 'New message in #general');
    });

    test('non-message channel events are excluded', () {
      final notification = classifyNotificationEvent(
        event: event(kind: 7),
        channel: channel(),
        myPubkey: 'me',
      );

      expect(notification, isNull);
    });
  });

  test('trimNotificationBody trims and caps Unicode at 140 characters', () {
    final body = trimNotificationBody('  ${'😀' * 141}  ');

    expect(body.runes.length, notificationBodyMaxCharacters);
    expect(body, '😀' * notificationBodyMaxCharacters);
  });

  test('trimNotificationBody collapses private multi-line preview text', () {
    expect(trimNotificationBody('  hello\n\tthere  '), 'hello there');
  });

  test('notificationIdForEvent is deterministic and Android-safe', () {
    final first = notificationIdForEvent('event-1');

    expect(notificationIdForEvent('event-1'), first);
    expect(first, inInclusiveRange(1, 0x7fffffff));
    expect(notificationIdForEvent('event-2'), isNot(first));
  });

  group('notificationCategoryEnabled', () {
    test('master setting disables both categories', () {
      expect(
        notificationCategoryEnabled(
          category: NotificationCategory.priority,
          alertsEnabled: false,
          priorityEnabled: true,
          activityEnabled: true,
        ),
        isFalse,
      );
    });

    test('category settings are independent', () {
      expect(
        notificationCategoryEnabled(
          category: NotificationCategory.priority,
          alertsEnabled: true,
          priorityEnabled: true,
          activityEnabled: false,
        ),
        isTrue,
      );
      expect(
        notificationCategoryEnabled(
          category: NotificationCategory.activity,
          alertsEnabled: true,
          priorityEnabled: true,
          activityEnabled: false,
        ),
        isFalse,
      );
    });
  });
}

NostrEvent event({
  String id = 'event-1',
  String pubkey = 'sender-pubkey',
  int kind = 9,
  String content = 'hello',
  List<List<String>> tags = const [
    ['h', 'channel-1'],
  ],
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: 100,
  kind: kind,
  tags: tags,
  content: content,
  sig: 'sig',
);

Channel channel({
  String id = 'channel-1',
  String name = 'general',
  String type = 'stream',
}) => Channel(
  id: id,
  name: name,
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: 'creator',
  createdAt: DateTime.fromMillisecondsSinceEpoch(0, isUtc: true),
  memberCount: 2,
  isMember: true,
);
