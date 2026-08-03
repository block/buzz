import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/notifications/notifications.dart';
import '../../shared/relay/relay.dart';
import '../channels/channel.dart';
import 'notification_classifier.dart';
import 'notification_event.dart';
import 'notification_event_deduper.dart';

class LiveNotificationDispatcher {
  LiveNotificationDispatcher(this._ref);

  final Ref _ref;
  final NotificationEventDeduper _deduper = NotificationEventDeduper();

  Future<void> dispatch({
    required NostrEvent event,
    required Channel channel,
    required String myPubkey,
    String? senderName,
    Set<String> participatedRootIds = const {},
    Set<String> followedRootIds = const {},
    Set<String> authoredRootIds = const {},
    Set<String> mutedChannelIds = const {},
  }) async {
    final notification = classifyNotificationEvent(
      event: event,
      channel: channel,
      myPubkey: myPubkey,
      senderName: senderName,
      participatedRootIds: participatedRootIds,
      followedRootIds: followedRootIds,
      authoredRootIds: authoredRootIds,
      mutedChannelIds: mutedChannelIds,
    );
    if (notification == null || !_deduper.add(notification.eventId)) return;

    try {
      var settings = _ref.read(notificationSettingsProvider);
      if (settings.alertsEnabled &&
          settings.permission == AndroidNotificationPermission.notDetermined) {
        await _ref.read(notificationSettingsProvider.notifier).refreshStatus();
        settings = _ref.read(notificationSettingsProvider);
      }
      if (!notificationCategoryEnabled(
        category: notification.category,
        alertsEnabled: settings.alertsEnabled,
        priorityEnabled: settings.priorityEnabled,
        activityEnabled: settings.activityEnabled,
      )) {
        return;
      }
      if (settings.permission != AndroidNotificationPermission.granted ||
          !_androidCategoryEnabled(notification.category, settings)) {
        return;
      }

      final title = settings.previewsEnabled ? notification.title : 'Buzz';
      final body = settings.previewsEnabled ? notification.body : 'New message';

      await _ref
          .read(androidNotificationBridgeProvider)
          .show(
            id: notification.id,
            channel: notification.channel,
            title: title,
            body: body,
            route: notification.route,
          );
    } on MissingPluginException {
      // Non-Android development and test surfaces have no native bridge.
      _deduper.remove(notification.eventId);
    } on PlatformException catch (error) {
      _deduper.remove(notification.eventId);
      debugPrint('[LiveNotificationDispatcher] show failed: $error');
    } catch (error) {
      // Notification delivery must never break the relay's live event path.
      _deduper.remove(notification.eventId);
      debugPrint('[LiveNotificationDispatcher] dispatch failed: $error');
    }
  }
}

final liveNotificationDispatcherProvider = Provider<LiveNotificationDispatcher>(
  (ref) {
    // Notification dedupe is connection/community-bound, like the live stream.
    ref.watch(relayConfigProvider);
    ref.watch(myPubkeyProvider);
    return LiveNotificationDispatcher(ref);
  },
);

bool _androidCategoryEnabled(
  NotificationCategory category,
  NotificationSettingsState settings,
) => switch (category) {
  NotificationCategory.priority => settings.priorityChannelEnabled,
  NotificationCategory.activity => settings.activityChannelEnabled,
};
