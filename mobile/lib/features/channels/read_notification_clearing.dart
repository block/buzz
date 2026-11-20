import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/auth/auth.dart';
import '../../shared/push/push_bridge.dart';
import '../../shared/read_state/message_read_state.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/relay/relay.dart';

/// Removes delivered iOS notifications whose messages are read, whether read
/// in this app or on another device once its read marker syncs, including on
/// return to the foreground. Only the active community's read state is
/// known, so another community's notifications clear once it is active.
final readNotificationClearerProvider = Provider<void>((ref) {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  var clearing = false;
  var clearAgain = false;
  Future<void> clear() async {
    if (clearing) {
      clearAgain = true;
      return;
    }
    clearing = true;
    try {
      do {
        clearAgain = false;
        final delivered = await deliveredMessageNotifications();
        final communityId = ref.read(activeCommunityProvider).value?.id;
        final readState = ref.read(readStateProvider);
        if (!readState.isReady) continue;
        final read = [
          for (final notification in delivered)
            if (notification.link.communityId == communityId &&
                !isMessageUnread(
                  readState,
                  channelId: notification.link.channelId,
                  messageId: notification.link.messageId,
                  createdAt: notification.messageCreatedAt,
                  threadRootId: notification.replyRootId,
                ))
              notification.id,
        ];
        if (read.isNotEmpty) await removeDeliveredNotifications(read);
      } while (clearAgain);
    } catch (error) {
      debugPrint('[ReadNotificationClearer] not cleared: $error');
    } finally {
      clearing = false;
    }
  }

  ref.listen(readStateProvider, (_, _) => unawaited(clear()));
  ref.listen(appForegroundProvider, (_, foreground) {
    if (foreground) unawaited(clear());
  }, fireImmediately: true);
});
