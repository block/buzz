import '../../shared/read_state/read_state_provider.dart';
import 'channel.dart';
import 'channels_provider.dart';
import 'unread_badge/observed_unread_event.dart';

/// Shared channel highlights, including reply and individual-message read marks.
Set<String> computeUnreadChannelIds({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
}) {
  if (!readState.isReady) {
    return const {};
  }

  final latestObservedByChannel = channelsNotifier.latestObservedByChannel;
  final observedEventsByChannel =
      channelsNotifier.observedUnreadEventsByChannel;
  final ids = <String>{};

  for (final channel in channels) {
    if (readState.locallyForcedChannelIds.contains(channel.id)) {
      ids.add(channel.id);
      continue;
    }

    final latestObserved = latestObservedByChannel[channel.id];
    if (latestObserved == null) continue;

    final channelReadAt = readState.effectiveTimestamp(channel.id);
    if (channelReadAt != null && latestObserved <= channelReadAt) continue;

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

    ids.add(channel.id);
  }

  return ids;
}
