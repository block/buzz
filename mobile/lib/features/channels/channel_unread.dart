import '../../shared/read_state/read_state_provider.dart';
import 'channel.dart';
import 'channels_provider.dart';
import 'unread_badge/observed_unread_event.dart';

/// Shared channel highlights, including reply and individual-message read marks.
/// Every unread event highlights the channel, as Solo chats need.
Set<String> computeUnreadChannelIds({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
}) => _computeHighlights(
  channels: channels,
  readState: readState,
  channelsNotifier: channelsNotifier,
  separateAttention: false,
).unread;

/// Channel-list highlights, following the desktop sidebar (`buzz-app`'s
/// `docs/unread.md`). Ordinary unread backlog bolds a channel ([unread]).
/// Unread thread replies, mentions and broadcasts only add an attention dot
/// ([attention]), so a channel the reader caught up on stays regular. A DM
/// bolds for anything. A channel marked unread bolds without a dot.
({Set<String> unread, Set<String> attention}) computeChannelListHighlights({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
}) => _computeHighlights(
  channels: channels,
  readState: readState,
  channelsNotifier: channelsNotifier,
  separateAttention: true,
);

({Set<String> unread, Set<String> attention}) _computeHighlights({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
  required bool separateAttention,
}) {
  final unread = <String>{};
  final attention = <String>{};
  if (!readState.isReady) return (unread: unread, attention: attention);

  final latestObservedByChannel = channelsNotifier.latestObservedByChannel;
  final observedEventsByChannel =
      channelsNotifier.observedUnreadEventsByChannel;

  for (final channel in channels) {
    if (readState.locallyForcedChannelIds.contains(channel.id)) {
      unread.add(channel.id);
      continue;
    }

    final latestObserved = latestObservedByChannel[channel.id];
    if (latestObserved == null) continue;

    final channelReadAt = readState.effectiveTimestamp(channel.id);
    if (channelReadAt != null && latestObserved <= channelReadAt) continue;

    final observedEvents = observedEventsByChannel[channel.id];
    if (observedEvents == null) continue;
    for (final event in observedEvents.values) {
      final readAt = observedUnreadEventReadAt(
        event,
        channel.id,
        readState.effectiveTimestamp,
      );
      if (readAt != null && event.createdAt <= readAt) continue;
      if (!separateAttention || channel.isDm || event.channelCatchUp) {
        unread.add(channel.id);
      } else {
        attention.add(channel.id);
      }
    }
  }

  return (unread: unread, attention: attention);
}
