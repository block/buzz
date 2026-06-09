import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../channels_provider.dart';
import '../read_state/read_state_provider.dart';
import '../read_state/read_state_time.dart';

class UnreadBadgeState {
  const UnreadBadgeState({
    this.highPriorityCount = 0,
    this.generalUnreadCount = 0,
  });

  final int highPriorityCount;
  final int generalUnreadCount;
}

final unreadBadgeProvider = Provider<UnreadBadgeState>((ref) {
  final channelsAsync = ref.watch(channelsProvider);
  final readState = ref.watch(readStateProvider);

  return channelsAsync.when(
    data: (channels) {
      // Safe to ref.read the notifier here: _latestHighPriorityByChannel is
      // only mutated inside _handleLiveEvent's state.whenData block, which
      // always emits a new channelsProvider state — so the ref.watch above
      // guarantees we re-run whenever the map changes.
      final notifier = ref.read(channelsProvider.notifier);
      final highPriorityMap = notifier.latestHighPriorityByChannel;

      int highPriority = 0;
      int general = 0;

      for (final channel in channels) {
        if (!channel.isMember || channel.isArchived) continue;

        final isLocallyForced = readState.locallyForcedChannelIds.contains(
          channel.id,
        );
        final lastMessageAt = dateTimeToUnixSeconds(channel.lastMessageAt);
        if (lastMessageAt == null && !isLocallyForced) continue;

        final readAt = readState.effectiveTimestamp(channel.id);
        final isUnread =
            isLocallyForced ||
            readAt == null ||
            (lastMessageAt != null && lastMessageAt > readAt);
        if (!isUnread) continue;

        if (channel.isDm) {
          highPriority++;
        } else {
          final highPriorityAt = highPriorityMap[channel.id];
          if (highPriorityAt != null &&
              (readAt == null || highPriorityAt > readAt)) {
            highPriority++;
          } else {
            general++;
          }
        }
      }

      return UnreadBadgeState(
        highPriorityCount: highPriority,
        generalUnreadCount: general,
      );
    },
    loading: () => const UnreadBadgeState(),
    error: (_, _) => const UnreadBadgeState(),
  );
});
