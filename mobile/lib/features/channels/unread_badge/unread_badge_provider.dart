import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../channels_provider.dart';
import '../../../shared/push/app_badge_preference.dart';
import '../../../shared/read_state/read_state_provider.dart';
import 'observed_unread_event.dart';

class UnreadBadgeState {
  const UnreadBadgeState({
    this.highPriorityCount = 0,
    this.generalUnreadCount = 0,
    this.mentionOrDmCount = 0,
  });

  final int highPriorityCount;
  final int generalUnreadCount;

  /// Unread DMs and mentions of the viewer, each counted once.
  final int mentionOrDmCount;
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
      var mentionOrDm = 0;

      for (final channel in channels) {
        if (!channel.isMember || channel.isArchived) continue;

        if (readState.locallyForcedChannelIds.contains(channel.id)) {
          general++;
          continue;
        }

        if (!latestObservedByChannel.containsKey(channel.id)) continue;

        final observedEvents = observedEventsByChannel[channel.id];
        int? readAtForObservedEvent(ObservedUnreadEvent event) =>
            observedUnreadEventReadAt(
              event,
              channel.id,
              readState.effectiveTimestamp,
            );

        final unreadCount = countUnreadObservedEvents(
          observedEvents,
          readAtForObservedEvent,
        );
        if (unreadCount == 0) continue;
        mentionOrDm += countUnreadMentionOrDmObservedEvents(
          observedEvents,
          readAtForObservedEvent,
        );

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
          if (badgeCount > 0) {
            general += badgeCount;
          } else {
            // Only ordinary backlog is left. Count it when its row is bold:
            // a synced `seen:<channel>` mark quiets backlog at or before it.
            final seenAt = readState.contexts['seen:${channel.id}'];
            final rowShowsBacklog =
                seenAt == null ||
                observedEvents!.values.any((event) {
                  if (event.createdAt <= seenAt) return false;
                  final readAt = readAtForObservedEvent(event);
                  return readAt == null || event.createdAt > readAt;
                });
            if (rowShowsBacklog) general++;
          }
        }
      }

      return UnreadBadgeState(
        highPriorityCount: highPriority,
        generalUnreadCount: general,
        mentionOrDmCount: mentionOrDm,
      );
    },
    loading: () => const UnreadBadgeState(),
    error: (_, _) => const UnreadBadgeState(),
  );
});

/// The number on the app icon. By default: the high-priority count, else 1
/// for ordinary unreads. With mentions only: unread mentions and DMs.
final appBadgeCountProvider = Provider<int>((ref) {
  final state = ref.watch(unreadBadgeProvider);
  if (ref.watch(appBadgeMentionsOnlyProvider)) return state.mentionOrDmCount;
  if (state.highPriorityCount > 0) return state.highPriorityCount;
  return state.generalUnreadCount > 0 ? 1 : 0;
});
