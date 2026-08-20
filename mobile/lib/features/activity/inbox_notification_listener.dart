import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/notifications/inbox_notification_seen.dart';
import '../../shared/notifications/local_notifications_service.dart';
import '../../shared/notifications/visible_conversation.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme_provider.dart';
import '../channels/channel.dart';
import '../channels/channel_mutes/channel_mutes_provider.dart';
import '../channels/channels_provider.dart';
import '../profile/user_cache_provider.dart';
import 'activity_provider.dart';
import 'feed_item.dart';
import 'inbox_notification_eligibility.dart';

/// Wires Activity feed changes to OS inbox notifications.
final inboxNotificationListenerProvider = Provider<void>((ref) {
  final myPk = ref.watch(myPubkeyProvider)?.trim().toLowerCase();
  if (myPk == null || myPk.isEmpty) return;

  final coordinator = ref.watch(_inboxNotificationCoordinatorProvider(myPk));

  ref.listen<AsyncValue<HomeFeedResponse>>(activityProvider, (_, next) {
    final feed = next.value;
    if (feed == null) return;

    final channels =
        ref.read(channelsProvider).asData?.value ?? const <Channel>[];
    final dmChannelIds = {
      for (final channel in channels)
        if (channel.isDm) channel.id,
    };
    final channelById = {for (final channel in channels) channel.id: channel};
    final mutedChannelIds = {
      for (final entry in ref.read(channelMutesProvider).store.channels.entries)
        if (entry.value.muted) entry.key,
    };
    final prefs = ref.read(savedPrefsProvider);
    final mutedRootIds = readMutedThreadRootIds(
      prefs.getString(mutedThreadRootIdsPrefsKey(myPk)),
    );
    final lifecycle = ref.read(appLifecycleProvider);
    final visible = currentVisibleConversation;
    final profiles = ref.read(userCacheProvider);

    coordinator.processFeed(
      feed: feed,
      myPubkey: myPk,
      dmChannelIds: dmChannelIds,
      mutedChannelIds: mutedChannelIds,
      mutedRootIds: mutedRootIds,
      channelById: channelById,
      appIsResumed: lifecycle == AppLifecycleState.resumed,
      visible: visible == null
          ? null
          : VisibleConversationSnapshot(
              channelId: visible.channelId,
              messageId: visible.messageId,
              threadRootId: visible.threadRootId,
            ),
      resolveSenderName: (pubkey) {
        final profile = profiles[pubkey.toLowerCase()];
        final display = profile?.displayName?.trim();
        if (display != null && display.isNotEmpty) return display;
        return null;
      },
    );
  }, fireImmediately: true);
});

final _inboxNotificationCoordinatorProvider =
    Provider.family<_InboxNotificationCoordinator, String>((ref, pubkey) {
      final prefs = ref.watch(savedPrefsProvider);
      return _InboxNotificationCoordinator(prefs: prefs, pubkey: pubkey);
    });

class _InboxNotificationCoordinator {
  _InboxNotificationCoordinator({required this.prefs, required this.pubkey})
    : _seenIds = readStoredInboxNotificationSeenIds(prefs, pubkey).toSet();

  final SharedPreferences prefs;
  final String pubkey;

  Set<String> _seenIds;
  bool _hasInitializedFeed = false;
  bool _requestedPermission = false;

  Future<void> processFeed({
    required HomeFeedResponse feed,
    required String myPubkey,
    required Set<String> dmChannelIds,
    required Set<String> mutedChannelIds,
    required Set<String> mutedRootIds,
    required Map<String, Channel> channelById,
    required bool appIsResumed,
    required VisibleConversationSnapshot? visible,
    required String? Function(String pubkey) resolveSenderName,
  }) async {
    final allEligible = collectInboxNotificationItemIds(
      feed,
      dmChannelIds: dmChannelIds,
    );

    if (!_hasInitializedFeed) {
      _hasInitializedFeed = true;
      if (allEligible.isNotEmpty) {
        _seenIds = allEligible.toSet();
        await writeStoredInboxNotificationSeenIds(prefs, pubkey, _seenIds);
      }
      return;
    }

    final newItems =
        eligibleInboxNotificationItems(feed, dmChannelIds: dmChannelIds).where((
          item,
        ) {
          if (_seenIds.contains(item.id)) return false;
          if (!shouldNotifyForInboxItem(
            item,
            myPubkey: myPubkey,
            dmChannelIds: dmChannelIds,
            mutedChannelIds: mutedChannelIds,
            mutedRootIds: mutedRootIds,
          )) {
            return false;
          }
          return !shouldSkipInboxNotificationBecauseVisible(
            item: item,
            visible: visible,
            appIsResumed: appIsResumed,
          );
        }).toList();

    final nextSeen = {..._seenIds, ...allEligible};
    _seenIds = capSeenInboxNotificationIds(nextSeen);
    await writeStoredInboxNotificationSeenIds(prefs, pubkey, _seenIds);

    if (newItems.isEmpty) return;

    if (!_requestedPermission) {
      _requestedPermission = true;
      await LocalNotificationsService.instance.requestPermissionsIfNeeded();
    }

    for (final item in newItems) {
      final channelId = item.channelId;
      if (channelId == null) continue;
      final channel = channelById[channelId];
      final isDm = channel?.isDm ?? dmChannelIds.contains(channelId);
      final senderName = resolveSenderName(item.pubkey);
      final title = inboxNotificationTitle(
        item,
        isDm: isDm,
        channelLabel: notificationChannelLabel(
          channelId: channelId,
          channelName: channel?.name ?? item.channelName,
          isDm: isDm,
        ),
        senderName: senderName,
      );
      final body = inboxNotificationBody(item);
      final payload = inboxNotificationPayload(item);
      await LocalNotificationsService.instance.showInboxNotification(
        notificationId: stableInboxNotificationId(item.id),
        title: title,
        body: body,
        payload: payload,
      );
    }
  }
}
