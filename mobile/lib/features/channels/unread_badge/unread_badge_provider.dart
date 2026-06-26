import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../channels_provider.dart';
import '../read_state/read_state_provider.dart';
import '../read_state/read_state_format.dart';
import 'observed_unread_event.dart';

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
      final notifier = ref.read(channelsProvider.notifier);
      final observedEventsByChannel = notifier.observedUnreadEventsByChannel;
      final latestObservedByChannel = notifier.latestObservedByChannel;

      var highPriority = 0;
      var general = 0;

      for (final channel in channels) {
        if (!channel.isMember || channel.isArchived) continue;

        if (readState.locallyForcedChannelIds.contains(channel.id)) {
          general++;
          continue;
        }

        if (!latestObservedByChannel.containsKey(channel.id)) continue;

        final observedEvents = observedEventsByChannel[channel.id];
        final channelReadAt = readState.effectiveTimestamp(channel.id);
        int? readAtForObservedEvent(ObservedUnreadEvent event) =>
            observedUnreadEventReadAt(
              event,
              channelReadAt,
              (rootId) =>
                  readState.effectiveTimestamp(threadContextKey(rootId)),
              (messageId) =>
                  readState.effectiveTimestamp(msgContextKey(messageId)),
            );

        final unreadCount = countUnreadObservedEvents(
          observedEvents,
          readAtForObservedEvent,
        );
        if (unreadCount == 0) continue;

        if (channel.isDm ||
            countUnreadHighPriorityObservedEvents(
                  observedEvents,
                  readAtForObservedEvent,
                ) >
                0) {
          final appBadgeCount = countUnreadAppBadgeObservedEvents(
            observedEvents,
            readAtForObservedEvent,
          );
          highPriority += appBadgeCount > 0 ? appBadgeCount : 1;
        } else {
          final badgeCount = countUnreadBadgeObservedEvents(
            observedEvents,
            readAtForObservedEvent,
          );
          general += badgeCount > 0 ? badgeCount : 1;
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
